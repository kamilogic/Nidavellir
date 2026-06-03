//! NVAPI-backed GPU access for Nidavellir.
//!
//! Stage 1 (this file): **read-only** — enumerate the GPU and read its real
//! voltage/frequency curve, validated against the live RTX hardware. The write
//! path (`set_pstates` clock offset, `set_vfp_locks` undervolt, `set_power_limit`)
//! is added in a later, carefully-gated stage.

/// One point of the GPU's voltage/frequency curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfCurvePoint {
    pub voltage_mv: u32,
    pub freq_mhz: u32,
}

/// A snapshot of the GPU's current V/F curve (the same data MSI Afterburner's
/// curve editor shows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCurve {
    pub name: String,
    pub points: Vec<VfCurvePoint>,
}

impl GpuCurve {
    /// The highest frequency present and the lowest voltage at which it appears
    /// — i.e. where a flat-curve undervolt has locked the clock.
    pub fn plateau(&self) -> Option<VfCurvePoint> {
        let max_freq = self.points.iter().map(|p| p.freq_mhz).max()?;
        self.points
            .iter()
            .filter(|p| p.freq_mhz == max_freq)
            .min_by_key(|p| p.voltage_mv)
            .copied()
    }
}

/// Read the live V/F curve from the first NVIDIA GPU (read-only, safe).
#[cfg(windows)]
pub fn read_curve() -> Result<GpuCurve, String> {
    nvapi::initialize().map_err(|e| format!("NvAPI_Initialize failed: {e:?}"))?;
    let gpu = nvapi::PhysicalGpu::enumerate()
        .map_err(|e| format!("enumerate failed: {e:?}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "no NVIDIA GPU found".to_string())?;

    let name = gpu.full_name().map_err(|e| format!("full_name failed: {e:?}"))?;
    let mask = gpu.vfp_mask().map_err(|e| format!("vfp_mask failed: {e:?}"))?;
    let curve = gpu
        .vfp_curve(mask.mask)
        .map_err(|e| format!("vfp_curve failed: {e:?}"))?;

    // The crate splits the (all-graphics) VF table into two arrays; both are
    // core V/F points for these cards, so read both or the curve is truncated.
    // `.0` of both Kilohertz2 and Kilohertz is kHz here (Display divides by
    // 1000); read the field directly to avoid the From<Kilohertz2> halving.
    let mut points: Vec<VfCurvePoint> = curve
        .graphics
        .iter()
        .map(|(_, e)| VfCurvePoint { voltage_mv: e.voltage.0 / 1000, freq_mhz: e.frequency.0 / 1000 })
        .collect();
    points.extend(
        curve
            .memory
            .iter()
            .map(|(_, e)| VfCurvePoint { voltage_mv: e.voltage.0 / 1000, freq_mhz: e.frequency.0 / 1000 })
            // Guard against a card that truly reports memory clocks here.
            .filter(|p| p.freq_mhz < 4000),
    );
    points.sort_by_key(|p| p.voltage_mv);
    points.dedup();

    Ok(GpuCurve { name, points })
}

#[cfg(not(windows))]
pub fn read_curve() -> Result<GpuCurve, String> {
    Err("NVAPI is Windows-only".into())
}

/// Apply a core clock offset (MHz) to P0 graphics. Reversible (offset 0 = stock).
#[cfg(windows)]
pub fn set_core_offset_mhz(mhz: i32) -> Result<(), String> {
    let gpu = first_gpu()?;
    gpu.set_pstates(std::iter::once((
        nvapi::PState::P0,
        nvapi::ClockDomain::Graphics,
        nvapi::KilohertzDelta(mhz * 1000),
    )))
    .map_err(|e| format!("set_pstates failed: {e:?}"))
}

/// Apply a memory clock offset (MHz) to P0. Reversible (offset 0 = stock).
#[cfg(windows)]
pub fn set_mem_offset_mhz(mhz: i32) -> Result<(), String> {
    let gpu = first_gpu()?;
    gpu.set_pstates(std::iter::once((
        nvapi::PState::P0,
        nvapi::ClockDomain::Memory,
        nvapi::KilohertzDelta(mhz * 1000),
    )))
    .map_err(|e| format!("set_pstates(memory) failed: {e:?}"))
}

/// Apply an Afterburner-style **VF CEILING**: flatten every graphics curve point
/// at or above `ceiling_mv` to `target_mhz` (cap the top of the curve), leaving
/// lower-voltage points untouched so the GPU keeps its V/F elasticity (it can
/// still downclock/downvolt). Unlike a hard voltage lock or NVML clock cap, this
/// doesn't remove the card's power management — which is what TDR'd under heavy
/// load. `khz_per_mhz` is the table delta unit (use [`calibrate_vf_unit`]).
/// Reversible via [`reset_vf_table`].
#[cfg(windows)]
pub fn set_vf_ceiling(target_mhz: u32, ceiling_mv: u32, khz_per_mhz: i32) -> Result<(), String> {
    let gpu = first_gpu()?;
    let mask = gpu.vfp_mask().map_err(|e| format!("vfp_mask: {e:?}"))?;
    let curve = gpu.vfp_curve(mask.mask).map_err(|e| format!("vfp_curve: {e:?}"))?;
    let deltas: Vec<(usize, nvapi::Kilohertz2Delta)> = curve
        .graphics
        .iter()
        .filter_map(|(i, e)| {
            let v = e.voltage.0 / 1000;
            let f = (e.frequency.0 / 1000) as i32;
            if v >= ceiling_mv {
                Some((*i, nvapi::Kilohertz2Delta((target_mhz as i32 - f) * khz_per_mhz)))
            } else {
                None
            }
        })
        .collect();
    if deltas.is_empty() {
        return Err("no curve points at/above the ceiling voltage".into());
    }
    gpu.set_vfp_table(mask.mask, deltas.into_iter(), std::iter::empty())
        .map_err(|e| format!("set_vfp_table: {e:?}"))
}

/// Clear all graphics VFP curve deltas (curve back to stock).
#[cfg(windows)]
pub fn reset_vf_table() -> Result<(), String> {
    let gpu = first_gpu()?;
    let mask = gpu.vfp_mask().map_err(|e| format!("vfp_mask: {e:?}"))?;
    let curve = gpu.vfp_curve(mask.mask).map_err(|e| format!("vfp_curve: {e:?}"))?;
    let deltas: Vec<(usize, nvapi::Kilohertz2Delta)> =
        curve.graphics.iter().map(|(i, _)| (*i, nvapi::Kilohertz2Delta(0))).collect();
    gpu.set_vfp_table(mask.mask, deltas.into_iter(), std::iter::empty())
        .map_err(|e| format!("reset vfp_table: {e:?}"))
}

/// Calibrate the VFP table's delta unit (the Kilohertz/Kilohertz2 ×2 quirk):
/// write a small **lowering** delta to the top-voltage graphics point (safe — no
/// load, and lowering can't destabilize), read the curve back, and return
/// `(probe_units, mhz_moved, base_mhz)`. The caller derives kHz-units-per-MHz =
/// probe_units / mhz_moved. Resets the probe after. If `mhz_moved == 0` the read
/// doesn't reflect deltas → don't trust a guessed unit.
#[cfg(windows)]
pub fn calibrate_vf_unit() -> Result<(i32, i32, i32), String> {
    let gpu = first_gpu()?;
    let mask = gpu.vfp_mask().map_err(|e| format!("vfp_mask: {e:?}"))?;
    let curve = gpu.vfp_curve(mask.mask).map_err(|e| format!("vfp_curve: {e:?}"))?;
    let (idx, base) = curve
        .graphics
        .iter()
        .max_by_key(|(_, e)| e.voltage.0)
        .map(|(i, e)| (*i, (e.frequency.0 / 1000) as i32))
        .ok_or_else(|| "no graphics points".to_string())?;
    const PROBE: i32 = -30000; // lowering delta in table units
    gpu.set_vfp_table(mask.mask, std::iter::once((idx, nvapi::Kilohertz2Delta(PROBE))), std::iter::empty())
        .map_err(|e| format!("probe set: {e:?}"))?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    let c2 = gpu.vfp_curve(mask.mask).map_err(|e| format!("re-read: {e:?}"))?;
    let after = c2
        .graphics
        .iter()
        .find(|(i, _)| *i == idx)
        .map(|(_, e)| (e.frequency.0 / 1000) as i32)
        .unwrap_or(base);
    let _ = gpu.set_vfp_table(mask.mask, std::iter::once((idx, nvapi::Kilohertz2Delta(0))), std::iter::empty());
    Ok((PROBE, after - base, base))
}

/// Lock the core voltage to `mv` (the GPU runs at the curve frequency for that
/// voltage). Reversible via [`unlock_core_voltage`].
#[cfg(windows)]
pub fn lock_core_voltage_mv(mv: u32) -> Result<(), String> {
    let gpu = first_gpu()?;
    gpu.set_vfp_locks(std::iter::once((0usize, Some(nvapi::Microvolts(mv * 1000)))))
        .map_err(|e| format!("set_vfp_locks failed: {e:?}"))
}

/// Release any core voltage lock (back to the dynamic curve).
#[cfg(windows)]
pub fn unlock_core_voltage() -> Result<(), String> {
    let gpu = first_gpu()?;
    gpu.set_vfp_locks(std::iter::once((0usize, None)))
        .map_err(|e| format!("set_vfp_locks(None) failed: {e:?}"))
}

/// Read the current core voltage in mV (parsed from NVAPI's formatted value).
#[cfg(windows)]
pub fn read_core_voltage_mv() -> Option<u32> {
    let gpu = first_gpu().ok()?;
    let v = gpu.core_voltage().ok()?;
    // Displays as e.g. "875 mV"; take the leading number.
    let s = format!("{v:?}");
    s.split_whitespace().next()?.parse::<f32>().ok().map(|x| x as u32)
}

/// Full reset: unlock voltage and clear the core + memory clock offsets.
#[cfg(windows)]
pub fn reset_all() -> Result<(), String> {
    let a = unlock_core_voltage();
    let b = set_core_offset_mhz(0);
    let c = set_mem_offset_mhz(0);
    a.and(b).and(c)
}

#[cfg(windows)]
fn first_gpu() -> Result<nvapi::PhysicalGpu, String> {
    nvapi::initialize().map_err(|e| format!("NvAPI_Initialize failed: {e:?}"))?;
    nvapi::PhysicalGpu::enumerate()
        .map_err(|e| format!("enumerate failed: {e:?}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "no NVIDIA GPU found".to_string())
}

/// Count of NVIDIA GPUs (binding sanity check).
#[cfg(windows)]
pub fn probe() -> Result<usize, String> {
    nvapi::initialize().map_err(|e| format!("NvAPI_Initialize failed: {e:?}"))?;
    let gpus = nvapi::PhysicalGpu::enumerate().map_err(|e| format!("enumerate failed: {e:?}"))?;
    Ok(gpus.len())
}

#[cfg(not(windows))]
pub fn probe() -> Result<usize, String> {
    Err("NVAPI is Windows-only".into())
}
