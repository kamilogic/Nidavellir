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

/// Modern per-point V/F curve control via the undocumented NvAPI `ClkVfPoints`
/// family — what MSI Afterburner / NVIDIA App / Green Curve / NV-UV use on
/// Pascal+ with current drivers (550+/590+). The `nvapi` crate only wraps the
/// OLD `SetClockBoostTable`, which driver 595.97 rejects; these are called by
/// function id via `NvAPI_QueryInterface`. READ side is harmless; writes are
/// gated behind a working read probe.
#[cfg(windows)]
mod vfcurve {
    use nvapi_sys::handles::NvPhysicalGpuHandle;

    const ID_ENUM: u32 = 0xE5AC_921F; // NvAPI_EnumPhysicalGPUs
    const ID_GET: u32 = 0x23F1_B133; // ClkVfPointsGetControl
    pub const ID_SET: u32 = 0x0733_E009; // ClkVfPointsSetControl
    const VER: u32 = 0x0001_2420; // size 0x2420 | (1<<16)
    const NPTS: usize = 255;

    // Per-point CONTROL entry (36 B = 0x24). Exact layout from LACT/NvAPI RE:
    // type_(+0,4) · rsvd[16](+4) · union data{ prog.freq_offset_khz: i32 }(+20,4) ·
    // rest of the 16-byte union (+24,12). The frequency offset (kHz) is at +20.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Entry {
        type_: u32,
        _rsvd: [u8; 16],
        freq_offset_khz: i32, // +20
        _rsvd2: [u8; 12],
    }
    const _: () = assert!(core::mem::size_of::<Entry>() == 0x24);

    // ClockClientClkVfPointsControlV1: version · mask[8] (256-bit) · rsvd[32] · 255 entries.
    #[repr(C)]
    struct Control {
        version: u32,
        mask: [u32; 8],
        _rsvd: [u8; 32],
        points: [Entry; NPTS],
    }
    const _: () = assert!(core::mem::size_of::<Control>() == 0x2420);

    // ---- GetStatus: per-point ACTUAL freq + voltage (read-only) ---------------
    const ID_STATUS: u32 = 0x2153_7AD4; // ClkVfPointsGetStatus

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Tuple {
        freq_khz: u32,
        voltage_uv: u32,
        _rsvd: [u8; 32],
    }
    // ClockClientClkVfPointStatusV3 (348 B): type_ · freq_khz · voltage_uv ·
    // vf_tuple_base · vf_tuple_offset · rsvd[256].
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct StatusEntry {
        type_: u32,
        freq_khz: u32,
        voltage_uv: u32,
        base: Tuple,
        offset: Tuple,
        _rsvd: [u8; 256],
    }
    const _: () = assert!(core::mem::size_of::<StatusEntry>() == 348);
    #[repr(C)]
    struct Status {
        version: u32,
        mask: [u32; 8],
        b_base_supported: u8,
        _rsvd: [u8; 64],
        points: [StatusEntry; NPTS],
    }
    // NVAPI MAKE_VERSION(struct, 3) = sizeof | (3<<16); derive from our struct so
    // it always matches our layout (driver returns -190 if the size is wrong).
    const VER_STATUS: u32 = (core::mem::size_of::<Status>() as u32) | (3 << 16);

    /// Read one point's ACTUAL (freq_khz, voltage_µV) via GetStatus. Single-bit
    /// mask. Returns `None` if the point is invalid / API fails.
    pub fn get_status(index: usize) -> Option<(u32, u32)> {
        if index >= NPTS {
            return None;
        }
        let _ = nvapi::initialize();
        let p = qi(ID_STATUS)?;
        let h = handle()?;
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Status) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut s: Box<Status> = Box::new(unsafe { core::mem::zeroed() });
        s.version = VER_STATUS;
        s.mask[index / 32] = 1u32 << (index % 32);
        if f(h, s.as_mut()) != 0 {
            return None;
        }
        Some((s.points[index].freq_khz, s.points[index].voltage_uv))
    }

    /// Diagnostic: GetStatus for sampled points — confirms the struct version and
    /// shows real freq/voltage data (proves the curve is read, not zeroed).
    pub fn dump_status() -> String {
        let _ = nvapi::initialize();
        let Some(p) = qi(ID_STATUS) else { return "qi(status) fail".into() };
        let Some(h) = handle() else { return "handle fail".into() };
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Status) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut out = format!("VER_STATUS {VER_STATUS:#x} ");
        for i in [0usize, 40, 80, 120, 160] {
            let mut s: Box<Status> = Box::new(unsafe { core::mem::zeroed() });
            s.version = VER_STATUS;
            s.mask[i / 32] = 1u32 << (i % 32);
            let st = f(h, s.as_mut());
            out.push_str(&format!(
                "[{i}:st{st} {}MHz {}mV] ",
                s.points[i].freq_khz / 1000,
                s.points[i].voltage_uv / 1000
            ));
        }
        out
    }

    fn qi(id: u32) -> Option<usize> {
        nvapi_sys::nvapi::nvapi_QueryInterface(id).ok()
    }

    fn handle() -> Option<NvPhysicalGpuHandle> {
        let p = qi(ID_ENUM)?;
        type F = extern "C" fn(*mut [NvPhysicalGpuHandle; 64], *mut u32) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut h: [NvPhysicalGpuHandle; 64] = unsafe { core::mem::zeroed() };
        let mut n: u32 = 0;
        if f(&mut h, &mut n) == 0 && n > 0 {
            Some(h[0])
        } else {
            None
        }
    }

    /// Read-only probe of `ClkVfPointsGetControl` for point 0. Returns the NvAPI
    /// status — 0 means the modern API + struct version work on this driver.
    /// Status `-1001`/`-1002` are our own markers (QueryInterface / enum failed).
    pub fn probe_get() -> i32 {
        let _ = nvapi::initialize();
        let Some(p) = qi(ID_GET) else { return -1001 };
        let Some(h) = handle() else { return -1002 };
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Control) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut c: Control = unsafe { core::mem::zeroed() };
        c.version = VER;
        c.mask[0] = 1; // point 0 only
        f(h, &mut c)
    }

    /// Read-only diagnostic: per-point single-bit GET (all-bits mask returns -1).
    /// Reports the GET status + current freq offset for a few sampled points so we
    /// can confirm the modern GET reads the right field with the corrected struct.
    pub fn dump_points() -> String {
        let _ = nvapi::initialize();
        let Some(p) = qi(ID_GET) else { return "qi fail".into() };
        let Some(h) = handle() else { return "handle fail".into() };
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Control) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut out = String::new();
        for i in [0usize, 50, 100, 150, 200, 254] {
            let mut c: Control = unsafe { core::mem::zeroed() };
            c.version = VER;
            c.mask[i / 32] = 1u32 << (i % 32);
            let st = f(h, &mut c);
            out.push_str(&format!(
                "[{i}:st{st} ty{} off{}] ",
                c.points[i].type_, c.points[i].freq_offset_khz
            ));
        }
        out
    }

    /// Read back ONE point's current freq offset (kHz) via the modern GET — used
    /// to verify a write round-trips in the new API's own 128-point index space.
    /// Returns `Some(khz)` on success, `None` on API failure.
    pub fn get_point(index: usize) -> Option<i32> {
        if index >= NPTS {
            return None;
        }
        let _ = nvapi::initialize();
        let p = qi(ID_GET)?;
        let h = handle()?;
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Control) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut c: Control = unsafe { core::mem::zeroed() };
        c.version = VER;
        c.mask[index / 32] = 1u32 << (index % 32);
        if f(h, &mut c) != 0 {
            return None;
        }
        Some(c.points[index].freq_offset_khz)
    }

    /// Write a per-point graphics-clock frequency offset (kHz) to ONE curve point
    /// (`index` = the 128-bit mask bit; the API rejects multiple bits per call).
    /// Returns the NvAPI status (0 = OK). This is the modern Afterburner-style
    /// curve write — it does NOT hard-lock voltage, so the GPU keeps elasticity.
    pub fn set_point(index: usize, freq_delta_khz: i32) -> i32 {
        if index >= NPTS {
            return -1003;
        }
        let _ = nvapi::initialize();
        let Some(pset) = qi(ID_SET) else { return -1001 };
        let Some(pget) = qi(ID_GET) else { return -1001 };
        let Some(h) = handle() else { return -1002 };
        type F = extern "C" fn(NvPhysicalGpuHandle, *mut Control) -> i32;
        let fget: F = unsafe { core::mem::transmute(pget) };
        let fset: F = unsafe { core::mem::transmute(pset) };
        // Read-modify-write: GET the full control first so every point's hidden
        // control fields (valid flags, min/max ranges) are populated. Writing a
        // zeroed struct makes the driver silently ignore the write (status 0,
        // no-op). Then modify only the target offset and SET with just its bit.
        let mut c: Control = unsafe { core::mem::zeroed() };
        c.version = VER;
        c.mask[index / 32] = 1u32 << (index % 32); // single point (all-bits → err -1)
        let g = fget(h, &mut c);
        if g != 0 {
            return g;
        }
        // The target entry's hidden control fields are now populated; set the
        // offset and write back with the SAME single-bit mask.
        c.version = VER;
        c.points[index].freq_offset_khz = freq_delta_khz;
        fset(h, &mut c)
    }
}

/// Write a per-point V/F frequency offset (MHz) to one curve point via the modern
/// API. `index` is the curve point index (from [`read_curve_indexed`]).
#[cfg(windows)]
pub fn vf_set_point_mhz(index: usize, mhz: i32) -> i32 {
    vfcurve::set_point(index, mhz * 1000)
}

/// Read-only diagnostic dump of the modern GET control entries (for RE).
#[cfg(windows)]
pub fn vf_dump_points() -> String {
    vfcurve::dump_points()
}

/// Read-only diagnostic dump of GetStatus (per-point real freq + voltage).
#[cfg(windows)]
pub fn vf_dump_status() -> String {
    vfcurve::dump_status()
}

/// One V/F point's actual (freq_mhz, voltage_mv) via the modern GetStatus.
#[cfg(windows)]
pub fn vf_point_status(index: usize) -> Option<(u32, u32)> {
    vfcurve::get_status(index).map(|(f, v)| (f / 1000, v / 1000))
}

/// Read back one point's current freq offset (kHz) via the modern GET.
#[cfg(windows)]
pub fn vf_get_point_khz(index: usize) -> Option<i32> {
    vfcurve::get_point(index)
}

/// Read the graphics V/F curve as `(point_index, voltage_mv, freq_mhz)` — the
/// index is what [`vf_set_point_mhz`] addresses.
#[cfg(windows)]
pub fn read_curve_indexed() -> Result<Vec<(usize, u32, u32)>, String> {
    let gpu = first_gpu()?;
    let mask = gpu.vfp_mask().map_err(|e| format!("vfp_mask: {e:?}"))?;
    let curve = gpu.vfp_curve(mask.mask).map_err(|e| format!("vfp_curve: {e:?}"))?;
    Ok(curve
        .graphics
        .iter()
        .map(|(i, e)| (*i, e.voltage.0 / 1000, e.frequency.0 / 1000))
        .collect())
}

/// True if the modern per-point V/F curve API works on this GPU + driver.
#[cfg(windows)]
pub fn vf_curve_supported() -> bool {
    vfcurve::probe_get() == 0
}

/// Raw NvAPI status from the modern ClkVf read probe (for diagnostics).
#[cfg(windows)]
pub fn vf_curve_probe_status() -> i32 {
    vfcurve::probe_get()
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
