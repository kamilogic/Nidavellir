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
