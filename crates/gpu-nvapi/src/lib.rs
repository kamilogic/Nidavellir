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
    /// NVAPI handles are opaque pointer-sized values. `nvapi_sys`'s
    /// `NvPhysicalGpuHandle` is a `repr(Rust)` newtype around `*const c_void` (via
    /// `nv_declare_handle!`), so passing it BY VALUE in an `extern "C" fn` signature
    /// trips `improper_ctypes_definitions` — the compiler can't guarantee a
    /// `repr(Rust)` type's layout matches the C ABI. NVAPI passes the handle as one
    /// opaque pointer and the enum call *fills* the handle array, so we carry it as a
    /// raw `*mut c_void` here: ABI-identical, FFI-safe, and never dereferenced.
    type RawGpuHandle = *mut core::ffi::c_void;

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
        type F = extern "C" fn(RawGpuHandle, *mut Status) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut s: Box<Status> = Box::new(unsafe { core::mem::zeroed() });
        s.version = VER_STATUS;
        s.mask[index / 32] = 1u32 << (index % 32);
        if f(h, s.as_mut()) != 0 {
            return None;
        }
        Some((s.points[index].freq_khz, s.points[index].voltage_uv))
    }

    /// Read one point's STATIC `vf_tuple_base` (freq_khz, voltage_µV) via GetStatus —
    /// the deterministic VF-table base, independent of any applied offset and of idle
    /// boost behavior (unlike the actual freq returned by [`get_status`], which the
    /// project documents as under-reporting at idle). Same single-bit mask and the SAME
    /// modern point index as [`get_status`]/`set_point`, so it joins by index with the
    /// rest of the verifier. Returns `None` if the point is invalid, the API fails, the
    /// driver reports the base tuple unsupported (`b_base_supported == 0`), or the base
    /// reads zero.
    pub fn get_status_base(index: usize) -> Option<(u32, u32)> {
        if index >= NPTS {
            return None;
        }
        let _ = nvapi::initialize();
        let p = qi(ID_STATUS)?;
        let h = handle()?;
        type F = extern "C" fn(RawGpuHandle, *mut Status) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut s: Box<Status> = Box::new(unsafe { core::mem::zeroed() });
        s.version = VER_STATUS;
        s.mask[index / 32] = 1u32 << (index % 32);
        if f(h, s.as_mut()) != 0 {
            return None;
        }
        if s.b_base_supported == 0 {
            return None;
        }
        let b = s.points[index].base;
        if b.freq_khz == 0 {
            return None;
        }
        Some((b.freq_khz, b.voltage_uv))
    }

    /// Diagnostic: GetStatus for sampled points — confirms the struct version and
    /// shows real freq/voltage data (proves the curve is read, not zeroed).
    pub fn dump_status() -> String {
        let _ = nvapi::initialize();
        let Some(p) = qi(ID_STATUS) else { return "qi(status) fail".into() };
        let Some(h) = handle() else { return "handle fail".into() };
        type F = extern "C" fn(RawGpuHandle, *mut Status) -> i32;
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

    fn handle() -> Option<RawGpuHandle> {
        let p = qi(ID_ENUM)?;
        type F = extern "C" fn(*mut [RawGpuHandle; 64], *mut u32) -> i32;
        let f: F = unsafe { core::mem::transmute(p) };
        let mut h: [RawGpuHandle; 64] = unsafe { core::mem::zeroed() };
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
        type F = extern "C" fn(RawGpuHandle, *mut Control) -> i32;
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
        type F = extern "C" fn(RawGpuHandle, *mut Control) -> i32;
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
        type F = extern "C" fn(RawGpuHandle, *mut Control) -> i32;
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
        type F = extern "C" fn(RawGpuHandle, *mut Control) -> i32;
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

/// Read the full live V/F curve via the modern GetStatus as `(index, voltage_mv,
/// freq_mhz)` for every valid point (voltage > 0). This is the index→voltage→freq
/// map the VF ceiling needs.
#[cfg(windows)]
pub fn read_vf_curve_modern() -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    for i in 0..255 {
        if let Some((f_khz, uv)) = vfcurve::get_status(i) {
            if uv > 0 {
                out.push((i, uv / 1000, f_khz / 1000));
            }
        }
    }
    out
}

/// Read the STATIC VF-table base curve via GetStatus's `vf_tuple_base` as
/// `(index, base_voltage_mv, base_freq_mhz)` for every valid point. Index-aligned with
/// [`read_vf_curve_modern`] (same modern point index). This is the deterministic,
/// offset-independent and idle-independent stock base the `NoDownCapNeeded` benign-zero
/// verifier evidence requires — NOT the actual/effective freq, which under-reports at
/// idle. Empty if the driver does not support the base tuple (then the verifier falls
/// back to strict behavior).
#[cfg(windows)]
pub fn read_vf_base_curve_modern() -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    for i in 0..255 {
        if let Some((f_khz, uv)) = vfcurve::get_status_base(i) {
            if uv > 0 {
                out.push((i, uv / 1000, f_khz / 1000));
            }
        }
    }
    out
}

/// Snap a *measured* (sensor) voltage to a deterministic VF-table bin: the lowest
/// curve voltage at or above `measured_mv`. If `measured_mv` is above every bin,
/// clamps to the highest bin (safe top-of-curve). Empty curve → `None`.
/// `curve` is `(index, voltage_mv, freq_mhz)` as returned by [`read_vf_curve_modern`].
///
/// This exists because a measured dwell voltage is a sparse sensor reading, NOT a
/// deterministic curve point; the apply ceiling must land on a real table bin, not
/// the raw measurement (see `decisions.md`: voltage field split). Pure + deterministic
/// so it is unit-testable without hardware, and platform-agnostic.
pub fn nearest_vf_bin_at_or_above(
    curve: &[(usize, u32, u32)],
    measured_mv: u32,
) -> Option<(usize, u32)> {
    if curve.is_empty() {
        return None;
    }
    if let Some(&(idx, mv, _)) = curve
        .iter()
        .filter(|(_, mv, _)| *mv >= measured_mv)
        .min_by_key(|(_, mv, _)| *mv)
    {
        return Some((idx, mv));
    }
    // Measured above all bins → clamp to the highest available table voltage.
    curve
        .iter()
        .max_by_key(|(_, mv, _)| *mv)
        .map(|&(idx, mv, _)| (idx, mv))
}

/// Classification of one VF-curve bin under a flatten-ceiling write plan. Pure /
/// diagnostic — lets a failed-probe analysis distinguish a legitimately-zero offset
/// (a bin already at target) from an elastic below-ceiling bin or a real pull-down,
/// WITHOUT post-write data alone deciding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfBinClass {
    /// Below the ceiling — left elastic (offset 0 by design, NOT part of the flatten set).
    BelowCeiling,
    /// At/above the ceiling, stock base above target → needs a negative (pull-down) offset.
    FlattenDown,
    /// At/above the ceiling, stock base below target → needs a positive (raise) offset.
    FlattenUp,
    /// At/above the ceiling, stock base already at target → desired offset is legitimately 0.
    AlreadyAtTarget,
}

/// One bin's entry in a flatten-ceiling write plan. Pure data; carries everything the
/// apply path writes and the read-only failed-probe diagnostic inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfWritePlanEntry {
    pub index: usize,
    pub voltage_mv: u32,
    pub base_mhz: u32,
    /// `target - base` for bins at/above the ceiling, else 0 (below-ceiling bins stay elastic).
    pub desired_offset_mhz: i32,
    pub below_ceiling: bool,
    /// At/above the ceiling → part of the intended flatten set.
    pub in_flatten_set: bool,
    pub desired_offset_is_zero: bool,
    pub class: VfBinClass,
}

/// Pure preview of the Afterburner-style VF-ceiling transform: for every point in
/// `curve`, compute the per-point frequency offset [`apply_vf_ceiling`] would write,
/// WITHOUT touching hardware. Points with voltage ≥ `ceiling_mv` are flattened to
/// `target_mhz` (offset `target - base`); lower-voltage points stay elastic (offset 0).
/// Preserves `curve` order. This is the single source of transform truth shared by the
/// real apply and the read-only diagnostic. Pure + deterministic + unit-testable; takes
/// the `(index, voltage_mv, freq_mhz)` shape returned by [`read_vf_curve_modern`].
pub fn plan_vf_ceiling(
    curve: &[(usize, u32, u32)],
    ceiling_mv: u32,
    target_mhz: u32,
) -> Vec<VfWritePlanEntry> {
    curve
        .iter()
        .map(|&(index, voltage_mv, base_mhz)| {
            let in_flatten_set = voltage_mv >= ceiling_mv;
            let desired_offset_mhz = if in_flatten_set {
                target_mhz as i32 - base_mhz as i32
            } else {
                0
            };
            let class = if !in_flatten_set {
                VfBinClass::BelowCeiling
            } else if desired_offset_mhz == 0 {
                VfBinClass::AlreadyAtTarget
            } else if desired_offset_mhz < 0 {
                VfBinClass::FlattenDown
            } else {
                VfBinClass::FlattenUp
            };
            VfWritePlanEntry {
                index,
                voltage_mv,
                base_mhz,
                desired_offset_mhz,
                below_ceiling: !in_flatten_set,
                in_flatten_set,
                desired_offset_is_zero: desired_offset_mhz == 0,
                class,
            }
        })
        .collect()
}

/// Apply an Afterburner-style **VF ceiling**: flatten every curve point whose
/// voltage is ≥ `ceiling_mv` to `target_mhz` (via per-point freq offsets), leaving
/// lower-voltage points untouched (elastic). This caps the top of the curve at
/// `target_mhz` without hard-locking voltage, so the GPU keeps its power-management
/// elasticity (the thing a rigid clock-cap / voltage-lock removed → TDR).
/// Returns the number of points flattened. The transform is computed by the pure
/// [`plan_vf_ceiling`] so the executed write and the diagnostic preview cannot drift.
#[cfg(windows)]
pub fn apply_vf_ceiling(ceiling_mv: u32, target_mhz: u32) -> Result<usize, String> {
    let curve = read_vf_curve_modern();
    if curve.is_empty() {
        return Err("curva V/F vazia (GetStatus não retornou pontos)".into());
    }
    let mut flattened = 0;
    for entry in plan_vf_ceiling(&curve, ceiling_mv, target_mhz) {
        let st = vfcurve::set_point(entry.index, entry.desired_offset_mhz * 1000);
        if st != 0 {
            return Err(format!("set_point({}) status {}", entry.index, st));
        }
        if entry.desired_offset_mhz != 0 {
            flattened += 1;
        }
    }
    Ok(flattened)
}

/// Reset the modern V/F curve: zero every valid point's frequency offset.
#[cfg(windows)]
pub fn reset_vf_curve() -> usize {
    let mut n = 0;
    for i in 0..255 {
        if vfcurve::get_status(i).map_or(false, |(_, uv)| uv > 0)
            && vfcurve::set_point(i, 0) == 0
        {
            n += 1;
        }
    }
    n
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

#[cfg(test)]
mod tests {
    use super::{nearest_vf_bin_at_or_above, plan_vf_ceiling, VfBinClass};

    // (index, voltage_mv, freq_mhz) — shape of read_vf_curve_modern().
    fn curve() -> Vec<(usize, u32, u32)> {
        vec![(0, 800, 1700), (1, 837, 1750), (2, 850, 1770), (3, 1062, 1900)]
    }

    #[test]
    fn exact_match_returns_that_bin() {
        assert_eq!(nearest_vf_bin_at_or_above(&curve(), 850), Some((2, 850)));
    }

    #[test]
    fn below_a_bin_rounds_up_to_it() {
        // 820 → the next table voltage at/above is 837.
        assert_eq!(nearest_vf_bin_at_or_above(&curve(), 820), Some((1, 837)));
    }

    #[test]
    fn between_bins_picks_lowest_at_or_above() {
        // 843 sits between the 837 and 850 bins → snaps up to 850.
        assert_eq!(nearest_vf_bin_at_or_above(&curve(), 843), Some((2, 850)));
    }

    #[test]
    fn above_all_bins_clamps_to_highest() {
        assert_eq!(nearest_vf_bin_at_or_above(&curve(), 1100), Some((3, 1062)));
    }

    #[test]
    fn empty_curve_is_none() {
        assert_eq!(nearest_vf_bin_at_or_above(&[], 850), None);
    }

    // ── plan_vf_ceiling (pure write-plan preview) ────────────────────────────────
    #[test]
    fn plan_below_ceiling_bins_are_elastic_zero() {
        // ceiling 850 mV → the 800 and 837 mV bins are below-ceiling: desired 0, elastic.
        let plan = plan_vf_ceiling(&curve(), 850, 1770);
        let below: Vec<_> = plan.iter().filter(|e| e.below_ceiling).collect();
        assert_eq!(below.len(), 2); // 800, 837 mV
        for e in below {
            assert_eq!(e.desired_offset_mhz, 0);
            assert!(e.desired_offset_is_zero);
            assert!(!e.in_flatten_set);
            assert_eq!(e.class, VfBinClass::BelowCeiling);
        }
    }

    #[test]
    fn plan_at_or_above_ceiling_computes_target_minus_base() {
        // ceiling 850 mV, target 1770 → the 850 (1770) and 1062 (1900) mV bins flatten.
        let plan = plan_vf_ceiling(&curve(), 850, 1770);
        let b850 = plan.iter().find(|e| e.voltage_mv == 850).unwrap();
        let b1062 = plan.iter().find(|e| e.voltage_mv == 1062).unwrap();
        assert!(b850.in_flatten_set && b1062.in_flatten_set);
        // 850 mV bin is naturally at target → desired 0 (legit zero), AlreadyAtTarget.
        assert_eq!(b850.desired_offset_mhz, 0);
        assert!(b850.desired_offset_is_zero);
        assert_eq!(b850.class, VfBinClass::AlreadyAtTarget);
        // 1062 mV bin base 1900 → 1770 - 1900 = -130 (pull-down).
        assert_eq!(b1062.desired_offset_mhz, -130);
        assert!(!b1062.desired_offset_is_zero);
        assert_eq!(b1062.class, VfBinClass::FlattenDown);
    }

    #[test]
    fn plan_bin_below_target_raises() {
        // A flatten-set bin whose base is BELOW target needs a positive offset.
        let c = vec![(0usize, 900u32, 1700u32)];
        let plan = plan_vf_ceiling(&c, 900, 1770);
        assert_eq!(plan[0].desired_offset_mhz, 70);
        assert_eq!(plan[0].class, VfBinClass::FlattenUp);
    }

    #[test]
    fn plan_flatten_count_matches_nonzero_desired() {
        // The flatten count apply_vf_ceiling reports = bins with a NON-ZERO desired offset.
        // curve(): at ceiling 837, target 1770 → bins 837(1750,+20), 850(1770,0), 1062(1900,-130).
        let plan = plan_vf_ceiling(&curve(), 837, 1770);
        let nonzero = plan.iter().filter(|e| e.desired_offset_mhz != 0).count();
        // 837 (+20) and 1062 (-130) are non-zero; 850 is a legit zero; 800 is below-ceiling.
        assert_eq!(nonzero, 2);
        let flatten_set = plan.iter().filter(|e| e.in_flatten_set).count();
        assert_eq!(flatten_set, 3); // 837, 850, 1062 mV
    }

    #[test]
    fn plan_ceiling_selection_matches_nearest_bin() {
        // A requested 843 mV snaps to the 850 mV bin; planning at that snapped ceiling
        // must put exactly the 850 and 1062 mV bins in the flatten set.
        let (_, snapped) = nearest_vf_bin_at_or_above(&curve(), 843).unwrap();
        assert_eq!(snapped, 850);
        let plan = plan_vf_ceiling(&curve(), snapped, 1770);
        let in_set: Vec<u32> = plan.iter().filter(|e| e.in_flatten_set).map(|e| e.voltage_mv).collect();
        assert_eq!(in_set, vec![850, 1062]);
    }
}
