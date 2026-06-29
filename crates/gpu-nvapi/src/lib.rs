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
        .map(|(_, e)| VfCurvePoint { voltage_mv: e.voltage.0 / 1000, freq_mhz: e.frequency.0 / 1000,
        })
        .collect();
    points.extend(
        curve
            .memory
            .iter()
            .map(|(_, e)| VfCurvePoint { voltage_mv: e.voltage.0 / 1000, freq_mhz: e.frequency.0 / 1000,
            })
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
                Some((*i, nvapi::Kilohertz2Delta((target_mhz as i32 - f) * khz_per_mhz),
                ))
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
    gpu.set_vfp_table(mask.mask, std::iter::once((idx, nvapi::Kilohertz2Delta(PROBE))), std::iter::empty(),
    )
        .map_err(|e| format!("probe set: {e:?}"))?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    let c2 = gpu.vfp_curve(mask.mask).map_err(|e| format!("re-read: {e:?}"))?;
    let after = c2
        .graphics
        .iter()
        .find(|(i, _)| *i == idx)
        .map(|(_, e)| (e.frequency.0 / 1000) as i32)
        .unwrap_or(base);
    let _ = gpu.set_vfp_table(mask.mask, std::iter::once((idx, nvapi::Kilohertz2Delta(0))), std::iter::empty(),
    );
    Ok((PROBE, after - base, base))
}

/// Lock the core voltage to `mv` (the GPU runs at the curve frequency for that
/// voltage). Reversible via [`unlock_core_voltage`].
#[cfg(windows)]
pub fn lock_core_voltage_mv(mv: u32) -> Result<(), String> {
    let gpu = first_gpu()?;
    gpu.set_vfp_locks(std::iter::once((0usize, Some(nvapi::Microvolts(mv * 1000)),
    )))
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
    unlock_core_voltage()?;
    set_core_offset_mhz(0)?;
    set_mem_offset_mhz(0)?;
    if vf_curve_supported() {
        reset_vf_curve_checked()?;
    }
    Ok(())
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
        let Some(p) = qi(ID_STATUS) else { return "qi(status) fail".into();
        };
        let Some(h) = handle() else { return "handle fail".into();
        };
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
        let Some(p) = qi(ID_GET) else { return "qi fail".into();
        };
        let Some(h) = handle() else { return "handle fail".into();
        };
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

/// Pure preview of the build-frontier **monotone-down** VF-ceiling transform, anchored to
/// the STATIC VF-table base (NOT the idle-depressed live curve `plan_vf_ceiling` uses). For
/// every point in `static_base_curve` (`(index, voltage_mv, base_freq_mhz)` from
/// [`read_vf_base_curve_modern`]): below-ceiling bins stay elastic (offset 0); bins at/above
/// `ceiling_mv` whose static base is ABOVE target are capped DOWN by exactly
/// `target - static_base` (negative → the driver lands them at target); bins whose static
/// base is already ≤ target keep offset 0 (NEVER raised). Emits ONLY offsets ≤ 0 — never a
/// `FlattenUp`. This is the deterministic ceiling the live-anchored planner could not express
/// (idle GetStatus under-reports vs the static base, so its down-caps were too weak and left
/// plateau overshoot). Pure + unit-testable; shares the plan shape so writer and diagnostic
/// cannot drift.
pub fn plan_vf_ceiling_monotone(
    static_base_curve: &[(usize, u32, u32)],
    ceiling_mv: u32,
    target_mhz: u32,
) -> Vec<VfWritePlanEntry> {
    static_base_curve
        .iter()
        .map(|&(index, voltage_mv, base_mhz)| {
            let in_flatten_set = voltage_mv >= ceiling_mv;
            let (desired_offset_mhz, class) = if !in_flatten_set {
                (0, VfBinClass::BelowCeiling)
            } else if base_mhz > target_mhz {
                // Negative down-cap → effective = static_base + (target - static_base) = target.
                (target_mhz as i32 - base_mhz as i32, VfBinClass::FlattenDown)
            } else {
                // Static base already ≤ target → no down-cap needed; never raise (no FlattenUp).
                (0, VfBinClass::AlreadyAtTarget)
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

/// Build-frontier-only **monotone-down** VF-ceiling writer anchored to the STATIC VF-table
/// base. Unlike [`apply_vf_ceiling`] (offsets derived from the idle-depressed live curve →
/// can under-cap and leave plateau overshoot), this reads [`read_vf_base_curve_modern`] and
/// writes the deterministic `target - static_base` down-cap (≤ 0) for every bin at/above
/// `ceiling_mv` whose static base exceeds target, leaving sub-target and below-ceiling bins
/// elastic (offset 0). FAILS CLOSED (`Err`) when the static base is unavailable/empty or no
/// bin sits at/above the ceiling — it NEVER falls back to the live-anchored writer and NEVER
/// locks voltage. Returns the number of non-zero down-cap writes. Used ONLY by the
/// build-frontier probe; the persisted-profile path keeps [`apply_vf_ceiling`].
#[cfg(windows)]
pub fn apply_vf_ceiling_monotone(ceiling_mv: u32, target_mhz: u32) -> Result<usize, String> {
    let static_base = read_vf_base_curve_modern();
    if static_base.is_empty() {
        return Err("static VF-table base unavailable (read_vf_base_curve_modern empty) — fail closed".into(),
        );
    }
    let plan = plan_vf_ceiling_monotone(&static_base, ceiling_mv, target_mhz);
    if !plan.iter().any(|e| e.in_flatten_set) {
        return Err(format!("no static-base bin at/above ceiling {ceiling_mv} mV — fail closed"));
    }
    let mut down_caps = 0;
    for entry in &plan {
        // Invariant: the monotone planner must NEVER emit a positive offset. Refuse to write
        // rather than ever raise a bin.
        if entry.desired_offset_mhz > 0 {
            return Err(format!(
                "monotone plan emitted positive offset {} at idx {} — refusing to write",
                entry.desired_offset_mhz, entry.index
            ));
        }
        let st = vfcurve::set_point(entry.index, entry.desired_offset_mhz * 1000);
        if st != 0 {
            return Err(format!("set_point({}) status {}", entry.index, st));
        }
        if entry.desired_offset_mhz != 0 {
            down_caps += 1;
        }
    }
    Ok(down_caps)
}

// ── F2 true-undervolt: bounded POSITIVE-offset planner/writer ────────────────────────────────
// True undervolt is the OPPOSITE of the build-frontier flatten-down: it RAISES a lower-voltage bin
// (a bounded positive offset) so the focus target clock holds at a lower voltage. The monotone
// flatten-down writer above refuses positive offsets by design; F2 therefore lives in its own
// bounded, fail-closed symbols and does NOT touch or relax `apply_vf_ceiling_monotone`.

/// Conservative absolute cap on an F2 positive (raise) frequency offset for a single VF bin (MHz).
/// True undervolt only needs to nudge a lower-voltage bin up a little to hold the focus target, so
/// the first F2 foundation keeps a small cap. Fail-closed: the planner REJECTS (never clamps) any
/// offset above this; the cap is a constant, never widened by a CLI flag.
pub const POS_OFFSET_MAX_MHZ: i32 = 30;

/// Conservative cap on the per-step INCREASE in positive offset between consecutive F2 probes (MHz).
/// Bounds how aggressively the undervolt deepens as the search descends voltage bins.
pub const POS_OFFSET_STEP_MAX_MHZ: i32 = 15;

/// Hard ABSOLUTE cap on an F2 positive offset for the OFFICIAL TARGET SWEEP's progressive learned
/// horizon (MHz). This is NOT a global cap widening and is DISTINCT from both the default/autonomous
/// absolute cap [`POS_OFFSET_MAX_MHZ`] (+30, for a single conservative descent — unchanged) and the
/// manual-prior cap (+250, an operator-PROVIDED KNOWN point applied in ONE shot). The target-sweep
/// horizon means "the maximum absolute offset discoverable ONLY through validated CHAINED per-step
/// increments": the per-step cap stays [`POS_OFFSET_STEP_MAX_MHZ`] (+15), so this absolute ceiling is
/// reachable solely by accumulating many small steps, each gated by a prior Validated outcome + clean
/// reset + cleared boot flag. It is deliberately smaller than the manual-prior cap (autonomous
/// discovery stays more conservative than an asserted point) yet large enough to let the descent reach
/// a low-voltage bin ~200 MHz below the target. Still fail-closed: the planner REJECTS (never clamps)
/// an offset above this, and it is a constant, never CLI-widenable.
pub const TARGET_SWEEP_HORIZON_MAX_MHZ: i32 = 210;

// Sanity bounds for an F2 base-curve point (mirror the service core-VF sanity domain so a foreign /
// memory-domain / zeroed curve is rejected). Voltage in mV, frequency in MHz.
const POS_SANE_MV_MIN: u32 = 600;
const POS_SANE_MV_MAX: u32 = 1150;
const POS_SANE_MHZ_MIN: u32 = 500;
const POS_SANE_MHZ_MAX: u32 = 3500;

fn is_sane_base_point(voltage_mv: u32, freq_mhz: u32) -> bool {
    (POS_SANE_MV_MIN..=POS_SANE_MV_MAX).contains(&voltage_mv)
        && (POS_SANE_MHZ_MIN..=POS_SANE_MHZ_MAX).contains(&freq_mhz)
}

/// True iff `curve` is a non-empty static base curve whose every point is a plausible graphics-core
/// VF point — used to reject an empty / foreign / non-sane base before any F2 positive-offset plan.
fn is_sane_base_curve(curve: &[(usize, u32, u32)]) -> bool {
    !curve.is_empty() && curve.iter().all(|&(_, mv, f)| is_sane_base_point(mv, f))
}

/// Fail-closed bounds for a bounded positive-offset (F2 true-undervolt) plan. The offset caps come
/// from the conservative constants; the floor/ceiling are hardware-derived by the caller. The
/// planner REJECTS anything outside these — it NEVER silently clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositiveOffsetLimits {
    /// Absolute cap on the positive freq offset applied to one bin (MHz).
    pub abs_max_offset_mhz: i32,
    /// Cap on the per-step INCREASE in offset between consecutive probes (MHz).
    pub step_max_offset_mhz: i32,
    /// Lowest safe voltage bin (mV); a bin below this is rejected (hardware-derived floor).
    pub hw_floor_mv: u32,
    /// Conservative absolute clock ceiling (MHz); a planned effective clock above this is rejected.
    pub clock_ceiling_mhz: u32,
}

impl PositiveOffsetLimits {
    /// Conservative limits: the built-in offset caps + the caller's hardware-derived floor/ceiling.
    /// This is the DEFAULT/autonomous-discovery envelope and is never widened by a CLI flag.
    pub fn conservative(hw_floor_mv: u32, clock_ceiling_mhz: u32) -> Self {
        Self {
            abs_max_offset_mhz: POS_OFFSET_MAX_MHZ,
            step_max_offset_mhz: POS_OFFSET_STEP_MAX_MHZ,
            hw_floor_mv,
            clock_ceiling_mhz,
        }
    }

    /// MANUAL-PRIOR (explicit development / known-GPU shortcut) limits: a SEPARATE, larger bounded
    /// positive-offset envelope for an operator-provided prior. Widens ONLY the offset caps — the
    /// absolute AND the per-step cap are both set to `max_offset_mhz` (manual-prior is single-step, so
    /// the per-step cap must admit the same one-shot raise) — while the hardware floor, the clock
    /// ceiling, and every real-bin/sanity check stay EXACTLY as `conservative`. Still fail-closed: the
    /// planner REJECTS (never clamps) an offset above `max_offset_mhz`. Used ONLY by the opt-in
    /// manual-prior path; the default/autonomous discovery keeps `conservative` (+30 / +15) and never
    /// sees this envelope.
    pub fn manual_prior(hw_floor_mv: u32, clock_ceiling_mhz: u32, max_offset_mhz: i32) -> Self {
        Self {
            abs_max_offset_mhz: max_offset_mhz,
            step_max_offset_mhz: max_offset_mhz,
            hw_floor_mv,
            clock_ceiling_mhz,
        }
    }

    /// OFFICIAL TARGET-SWEEP learned-offset-horizon limits: a SEPARATE envelope for the autonomous
    /// same-target minimum-stable-voltage sweep. Raises ONLY the absolute cap to
    /// [`TARGET_SWEEP_HORIZON_MAX_MHZ`] (+210) while keeping the per-step cap at the conservative
    /// [`POS_OFFSET_STEP_MAX_MHZ`] (+15) — the CRITICAL difference from [`manual_prior`], which widens
    /// BOTH caps for a one-shot known point. Here the larger absolute ceiling is reachable ONLY by
    /// accumulating validated chained +15 increments (each gated by a prior Validated outcome + clean
    /// reset + cleared boot flag), so a single step can never jump to it. The hardware floor, the clock
    /// ceiling, and every real-bin/sanity check stay EXACTLY as `conservative`. Still fail-closed: the
    /// planner REJECTS (never clamps) an offset above the absolute cap or a per-step delta above +15.
    /// Used ONLY by the opt-in `--auto-sweep` path; the default discovery keeps `conservative` (+30/+15),
    /// manual-prior keeps its own envelope, and neither is affected by this constructor.
    pub fn target_sweep_learning_horizon(hw_floor_mv: u32, clock_ceiling_mhz: u32) -> Self {
        Self {
            abs_max_offset_mhz: TARGET_SWEEP_HORIZON_MAX_MHZ,
            step_max_offset_mhz: POS_OFFSET_STEP_MAX_MHZ,
            hw_floor_mv,
            clock_ceiling_mhz,
        }
    }

    /// Full F2 frontier-discovery envelope. Unlike the legacy `+30`/`+210` policies, this bound is
    /// derived only from the real hardware domain: the highest allowed target minus the lowest sane
    /// base clock present in the live static VF table. The writer still requires a real bin, a target
    /// at/below `clock_ceiling_mhz`, an exact effective target, and an anchor at/above the hardware
    /// voltage floor. Setting the step cap to the same physical delta allows adjacent real VF bins
    /// even when their base-clock spacing is larger than an arbitrary 15 MHz.
    pub fn hardware_frontier(hw_floor_mv: u32, clock_ceiling_mhz: u32, min_base_mhz: u32) -> Self {
        let physical_delta = clock_ceiling_mhz.saturating_sub(min_base_mhz).max(1) as i32;
        Self {
            abs_max_offset_mhz: physical_delta,
            step_max_offset_mhz: physical_delta,
            hw_floor_mv,
            clock_ceiling_mhz,
        }
    }
}

/// A validated single-bin bounded positive-offset (F2) write plan. Pure data; produced ONLY when
/// every fail-closed rule in [`plan_bounded_positive_offset`] passes, and returned BEFORE any write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositiveOffsetPlan {
    pub index: usize,
    pub voltage_mv: u32,
    pub base_mhz: u32,
    /// The validated positive offset to apply to this bin (> 0, ≤ abs cap).
    pub offset_mhz: i32,
    /// The previously-applied offset on the prior probe (0 if none) — used for the per-step delta.
    pub prev_offset_mhz: i32,
    /// `offset_mhz - prev_offset_mhz` (the per-step increase; ≤ step cap).
    pub step_delta_mhz: i32,
    /// The planned effective clock at this bin (`base + offset`, == target).
    pub effective_mhz: u32,
}

/// Pure, fail-closed planner for ONE bounded POSITIVE-offset (F2 true-undervolt) point: raise the
/// `bin_index` VF point just enough to hold `target_mhz` at that (lower) voltage. This is the
/// OPPOSITE of [`plan_vf_ceiling_monotone`] (which only flattens DOWN and never raises) and is the
/// ONLY sanctioned positive-offset planner — it does NOT relax `apply_vf_ceiling_monotone`.
///
/// Computes `offset = target - base` for the chosen bin and REJECTS (never clamps) when any rule
/// fails: empty/foreign/non-sane base curve; `bin_index` not a real point on the curve; the bin's
/// voltage is below the hardware floor; the offset is ≤ 0 (positive-offset-only); the offset exceeds
/// the absolute cap; the per-step increase over `prev_offset_mhz` exceeds the per-step cap; or the
/// planned effective clock exceeds the conservative clock ceiling. Returns the plan BEFORE any write.
pub fn plan_bounded_positive_offset(
    static_base_curve: &[(usize, u32, u32)],
    bin_index: usize,
    target_mhz: u32,
    prev_offset_mhz: i32,
    limits: &PositiveOffsetLimits,
) -> Result<PositiveOffsetPlan, String> {
    if static_base_curve.is_empty() {
        return Err("F2: static VF-table base unavailable/empty — fail closed".into());
    }
    if !is_sane_base_curve(static_base_curve) {
        return Err("F2: base curve has foreign / non-sane points — fail closed".into());
    }
    let &(index, voltage_mv, base_mhz) = static_base_curve
        .iter()
        .find(|(i, _, _)| *i == bin_index)
        .ok_or_else(|| format!("F2: bin index {bin_index} is not a real VF point — fail closed"))?;
    if voltage_mv < limits.hw_floor_mv {
        return Err(format!(
            "F2: bin {voltage_mv} mV is below the hardware floor {} mV — fail closed",
            limits.hw_floor_mv
        ));
    }
    let offset_mhz = target_mhz as i32 - base_mhz as i32;
    if offset_mhz <= 0 {
        return Err(format!(
            "F2: bin {voltage_mv} mV base {base_mhz} MHz already >= target {target_mhz} MHz \
             (offset {offset_mhz} <= 0) — positive-offset-only, fail closed"
        ));
    }
    if offset_mhz > limits.abs_max_offset_mhz {
        return Err(format!(
            "F2: offset +{offset_mhz} MHz exceeds the absolute cap +{} MHz — fail closed",
            limits.abs_max_offset_mhz
        ));
    }
    let step_delta_mhz = offset_mhz - prev_offset_mhz;
    if step_delta_mhz > limits.step_max_offset_mhz {
        return Err(format!(
            "F2: per-step increase +{step_delta_mhz} MHz (prev +{prev_offset_mhz}) exceeds the \
             per-step cap +{} MHz — fail closed",
            limits.step_max_offset_mhz
        ));
    }
    let effective_mhz = base_mhz as i32 + offset_mhz; // == target_mhz by construction
    if effective_mhz <= 0 || effective_mhz as u32 > limits.clock_ceiling_mhz {
        return Err(format!(
            "F2: planned clock {effective_mhz} MHz exceeds the clock ceiling {} MHz — fail closed",
            limits.clock_ceiling_mhz
        ));
    }
    Ok(PositiveOffsetPlan {
        index,
        voltage_mv,
        base_mhz,
        offset_mhz,
        prev_offset_mhz,
        step_delta_mhz,
        effective_mhz: effective_mhz as u32,
    })
}

/// F2-only bounded POSITIVE-offset writer: plan via [`plan_bounded_positive_offset`] (fail-closed)
/// then write the single validated positive offset to the target bin via the modern ClkVfPoints
/// API. SEPARATE from [`apply_vf_ceiling_monotone`] (which it neither calls nor relaxes) and from
/// the flatten-down path. Re-checks the bound defensively before the write and refuses a
/// non-positive / out-of-bound offset. Returns the executed plan. NOT called by the dry-run probe.
#[cfg(windows)]
pub fn apply_bounded_positive_offset(
    static_base_curve: &[(usize, u32, u32)],
    bin_index: usize,
    target_mhz: u32,
    prev_offset_mhz: i32,
    limits: &PositiveOffsetLimits,
) -> Result<PositiveOffsetPlan, String> {
    let plan =
        plan_bounded_positive_offset(static_base_curve, bin_index, target_mhz, prev_offset_mhz, limits,
    )?;
    // Defensive: never write a non-positive or over-cap offset even if the planner ever changes.
    if plan.offset_mhz <= 0 || plan.offset_mhz > limits.abs_max_offset_mhz {
        return Err(format!(
            "F2: refusing to write out-of-bound offset {} MHz (cap +{}) — fail closed",
            plan.offset_mhz, limits.abs_max_offset_mhz
        ));
    }
    let st = vfcurve::set_point(plan.index, plan.offset_mhz * 1000);
    if st != 0 {
        return Err(format!("F2: set_point({}) status {}", plan.index, st));
    }
    Ok(plan)
}

// ── F2 ANCHORED true-undervolt: a classic curve point (raise the anchor + cap the plateau) ──────
// The bounded single-bin raise above proves the positive-offset MOTOR but leaves the rest of the
// boost curve free, so the GPU still boosts ABOVE the nominal target. A CLASSIC undervolt point is
// ANCHORED: the selected (lower-voltage) bin is RAISED to the target, and every HIGHER-voltage bin
// is CAPPED / flattened DOWN to the same target so the card cannot boost above it during the test;
// lower-voltage bins stay elastic (offset 0, never raised). This COMPOSES the bounded positive-offset
// planner (for the anchor) with a flatten-DOWN cap on the bins above it — it does NOT call, touch, or
// relax `apply_vf_ceiling_monotone` (the build-frontier writer) or the single-bin F2 path.

/// Role of one bin within an [`AnchoredPositiveOffsetPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchoredBinRole {
    /// The selected anchor bin: a bounded POSITIVE offset raises it to the target.
    Anchor,
    /// A higher-voltage bin held at/below the target by a ≤ 0 offset (capped DOWN, or already
    /// ≤ target). This is what prevents the GPU from boosting above the target during the test.
    CappedAbove,
    /// A lower-voltage bin left elastic (offset 0, never raised).
    ElasticBelow,
}

/// One bin's entry in an anchored plan. Pure data; carries exactly what the writer applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredBinEntry {
    pub index: usize,
    pub voltage_mv: u32,
    pub base_mhz: u32,
    /// Offset to write: `> 0` ONLY at the anchor; `≤ 0` for capped higher-voltage bins; `0` for
    /// elastic lower-voltage bins.
    pub offset_mhz: i32,
    /// `base + offset` — never above `target_mhz` for any bin.
    pub effective_mhz: u32,
    pub role: AnchoredBinRole,
}

/// A validated ANCHORED positive-offset (classic undervolt point) plan. Produced ONLY when every
/// fail-closed rule in [`plan_bounded_anchored_positive_offset`] passes, returned BEFORE any write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredPositiveOffsetPlan {
    pub target_mhz: u32,
    /// The validated bounded positive raise at the anchor bin (re-uses the single-bin planner, so it
    /// inherits every floor/offset/per-step/ceiling fail-closed rule).
    pub anchor: PositiveOffsetPlan,
    /// Every curve bin's write entry, in input-curve order (the anchor + the caps + the elastic).
    pub entries: Vec<AnchoredBinEntry>,
    /// Higher-voltage bins capped DOWN (negative offset) to the target.
    pub capped_above_bins: u32,
    /// Higher-voltage bins already at/below the target (offset 0 — no cap needed, never raised).
    pub above_already_ok_bins: u32,
    /// Lower-voltage bins left elastic (offset 0).
    pub elastic_below_bins: u32,
    /// The largest positive offset across all bins (== the anchor offset).
    pub max_positive_offset_mhz: i32,
    /// The largest downward flatten across the higher-voltage bins (absolute MHz; 0 if none capped).
    pub max_negative_flatten_mhz: i32,
}

/// Pure, fail-closed planner for an ANCHORED positive-offset (F2 classic undervolt) point. Raises
/// the `anchor_bin_index` bin to `target_mhz` (a bounded positive offset, re-using
/// [`plan_bounded_positive_offset`] so it inherits EVERY fail-closed rule), then CAPS every
/// higher-voltage bin DOWN to the target (offset ≤ 0; never raised) and leaves every lower-voltage
/// bin elastic (offset 0, never raised). The result is the classic point `target MHz @ anchor mV with
/// boost above target prevented`.
///
/// REJECTS (never clamps) when: the anchor raise fails any single-bin rule (empty/foreign/non-sane
/// curve; bin not real; below floor; non-positive offset; absolute or per-step cap; clock ceiling); a
/// higher-voltage bin would require a positive offset (only the anchor may be raised); a
/// higher-voltage bin would remain above the target after the cap; a lower-voltage bin's base already
/// sits above the target (a non-monotone / unsafe curve); or the final plan carries any positive
/// offset outside the anchor. Returns the plan BEFORE any write. Does NOT touch
/// `apply_vf_ceiling_monotone`.
pub fn plan_bounded_anchored_positive_offset(
    static_base_curve: &[(usize, u32, u32)],
    anchor_bin_index: usize,
    target_mhz: u32,
    prev_offset_mhz: i32,
    limits: &PositiveOffsetLimits,
) -> Result<AnchoredPositiveOffsetPlan, String> {
    // The anchor raise re-uses the single-bin planner → it inherits EVERY fail-closed rule
    // (empty/foreign/non-sane curve, bin-not-real, below-floor, non-positive offset, absolute &
    // per-step offset caps, clock ceiling). If it rejects, the whole anchored plan is rejected.
    let anchor = plan_bounded_positive_offset(
        static_base_curve,
        anchor_bin_index,
        target_mhz,
        prev_offset_mhz,
        limits,
    )?;
    let anchor_mv = anchor.voltage_mv;
    let target_i = target_mhz as i32;

    let mut entries = Vec::with_capacity(static_base_curve.len());
    let mut capped_above_bins = 0u32;
    let mut above_already_ok_bins = 0u32;
    let mut elastic_below_bins = 0u32;
    let mut max_negative_flatten_mhz = 0i32;

    for &(index, voltage_mv, base_mhz) in static_base_curve {
        if index == anchor.index {
            entries.push(AnchoredBinEntry {
                index,
                voltage_mv,
                base_mhz,
                offset_mhz: anchor.offset_mhz,
                effective_mhz: target_mhz,
                role: AnchoredBinRole::Anchor,
            });
            continue;
        }
        if voltage_mv > anchor_mv {
            // Higher-voltage bin: cap DOWN to target if above it; NEVER raise. Emit an offset ≤ 0.
            let base_i = base_mhz as i32;
            let offset = if base_i > target_i { target_i - base_i } else { 0 };
            // Defensive: a higher-voltage bin must NEVER receive a positive offset.
            if offset > 0 {
                return Err(format!(
                    "F2 anchored: higher-voltage bin {voltage_mv} mV would need a positive offset \
                     +{offset} — only the anchor may be raised, fail closed"
                ));
            }
            let effective = base_i + offset;
            // Defensive: no higher-voltage bin may remain above the target after the cap.
            if effective > target_i {
                return Err(format!(
                    "F2 anchored: higher-voltage bin {voltage_mv} mV stays at {effective} MHz above \
                     target {target_mhz} MHz — fail closed"
                ));
            }
            if offset < 0 {
                capped_above_bins += 1;
                max_negative_flatten_mhz = max_negative_flatten_mhz.max(-offset);
            } else {
                above_already_ok_bins += 1;
            }
            entries.push(AnchoredBinEntry {
                index,
                voltage_mv,
                base_mhz,
                offset_mhz: offset,
                effective_mhz: effective.max(0) as u32,
                role: AnchoredBinRole::CappedAbove,
            });
        } else {
            // Lower-voltage bin: leave elastic (offset 0, never raised). Monotone sanity: a bin below
            // the anchor must not already sit above the target (non-monotone / unsafe) — fail closed.
            if base_mhz as i32 > target_i {
                return Err(format!(
                    "F2 anchored: lower-voltage bin {voltage_mv} mV base {base_mhz} MHz already above \
                     target {target_mhz} MHz — non-monotone/unsafe, fail closed"
                ));
            }
            elastic_below_bins += 1;
            entries.push(AnchoredBinEntry {
                index,
                voltage_mv,
                base_mhz,
                offset_mhz: 0,
                effective_mhz: base_mhz,
                role: AnchoredBinRole::ElasticBelow,
            });
        }
    }

    // Defensive global invariant: EXACTLY one positive offset, and it is the anchor bin.
    let positive: Vec<&AnchoredBinEntry> = entries.iter().filter(|e| e.offset_mhz > 0).collect();
    if positive.len() != 1 || positive[0].index != anchor.index {
        return Err(
            "F2 anchored: plan would carry a positive offset outside the anchor bin — fail closed".into(),
        );
    }

    Ok(AnchoredPositiveOffsetPlan {
        target_mhz,
        anchor,
        entries,
        capped_above_bins,
        above_already_ok_bins,
        elastic_below_bins,
        max_positive_offset_mhz: anchor.offset_mhz,
        max_negative_flatten_mhz,
    })
}

/// F2-only ANCHORED writer: plan via [`plan_bounded_anchored_positive_offset`] (fail-closed) then
/// write EVERY bin's offset — the anchor's bounded positive raise, the higher-voltage down-caps, and
/// zeros on the elastic bins — via the modern ClkVfPoints API. SEPARATE from
/// [`apply_vf_ceiling_monotone`] (neither called nor relaxed) and from the single-bin
/// [`apply_bounded_positive_offset`]. Defensively refuses any positive offset outside the anchor (and
/// an out-of-bound anchor offset) before each write. Returns the executed plan. NOT called by the
/// dry-run probe.
#[cfg(windows)]
pub fn apply_bounded_anchored_positive_offset(
    static_base_curve: &[(usize, u32, u32)],
    anchor_bin_index: usize,
    target_mhz: u32,
    prev_offset_mhz: i32,
    limits: &PositiveOffsetLimits,
) -> Result<AnchoredPositiveOffsetPlan, String> {
    let plan = plan_bounded_anchored_positive_offset(
        static_base_curve,
        anchor_bin_index,
        target_mhz,
        prev_offset_mhz,
        limits,
    )?;
    // Defensive: the anchor offset must stay positive and within the absolute cap.
    if plan.anchor.offset_mhz <= 0 || plan.anchor.offset_mhz > limits.abs_max_offset_mhz {
        return Err(format!(
            "F2 anchored: refusing to write out-of-bound anchor offset {} MHz (cap +{}) — fail closed",
            plan.anchor.offset_mhz, limits.abs_max_offset_mhz
        ));
    }
    for e in &plan.entries {
        // Defensive per-bin: only the anchor may carry a positive offset (caps are ≤ 0).
        if e.offset_mhz > 0 && e.index != plan.anchor.index {
            return Err(format!(
                "F2 anchored: refusing positive offset +{} on non-anchor bin {} mV — fail closed",
                e.offset_mhz, e.voltage_mv
            ));
        }
        let st = vfcurve::set_point(e.index, e.offset_mhz * 1000);
        if st != 0 {
            return Err(format!("F2 anchored: set_point({}) status {}", e.index, st));
        }
    }
    Ok(plan)
}

/// Reset the modern V/F curve: zero every valid point's frequency offset.
#[cfg(windows)]
pub fn reset_vf_curve() -> usize {
    reset_vf_curve_checked().unwrap_or(0)
}

/// Reset and verify every valid modern V/F point. Unlike the legacy best-effort helper, this returns
/// an error if any write or readback cannot prove the curve is back at stock.
#[cfg(windows)]
pub fn reset_vf_curve_checked() -> Result<usize, String> {
    let mut n = 0;
    for i in 0..255 {
        if vfcurve::get_status(i).is_none() {
            continue;
        }
        let status = vfcurve::set_point(i, 0);
        if status != 0 {
            return Err(format!("VF reset: set_point({i}) status {status}"));
        }
        match vfcurve::get_point(i) {
            Some(khz) if khz.abs() <= 1_000 => n += 1,
            Some(khz) => {
                return Err(format!("VF reset: point {i} still has {khz} kHz offset"));
            }
            None => {
                return Err(format!("VF reset: point {i} readback unavailable"));
            }
        }
    }
    Ok(n)
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
    use super::{
        nearest_vf_bin_at_or_above, plan_bounded_anchored_positive_offset,
        plan_bounded_positive_offset, plan_vf_ceiling, plan_vf_ceiling_monotone, AnchoredBinRole,
        PositiveOffsetLimits, VfBinClass, POS_OFFSET_MAX_MHZ, POS_OFFSET_STEP_MAX_MHZ,
        TARGET_SWEEP_HORIZON_MAX_MHZ,
    };

    // (index, voltage_mv, freq_mhz) — shape of read_vf_curve_modern().
    fn curve() -> Vec<(usize, u32, u32)> {
        vec![(0, 800, 1700), (1, 837, 1750), (2, 850, 1770), (3, 1062, 1900),
        ]
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

    // ── plan_vf_ceiling_monotone (build-frontier static-base monotone-down) ───────
    // Static-base curve: (index, voltage_mv, base_freq_mhz). Bins span below/at/above a 900 mV
    // ceiling; the 1062 mV bin's static base (1845) is the overshoot case from the 1755@900 run.
    fn base_curve() -> Vec<(usize, u32, u32)> {
        vec![(0, 800, 1650), (1, 875, 1700), (2, 900, 1740), (3, 975, 1800), (4, 1062, 1845),
        ]
    }

    #[test]
    fn monotone_never_emits_positive_offset() {
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        assert!(plan.iter().all(|e| e.desired_offset_mhz <= 0));
        assert!(plan.iter().all(|e| e.class != VfBinClass::FlattenUp));
    }

    #[test]
    fn monotone_high_base_caps_exactly_to_target() {
        // 1062 mV bin base 1845, target 1755 → offset -90; reconstructed base+offset == target.
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        let b = plan.iter().find(|e| e.voltage_mv == 1062).unwrap();
        assert_eq!(b.desired_offset_mhz, -90);
        assert_eq!(b.class, VfBinClass::FlattenDown);
        assert_eq!(b.base_mhz as i32 + b.desired_offset_mhz, 1755);
    }

    #[test]
    fn monotone_sub_target_bin_stays_zero() {
        // 900 mV bin base 1740 ≤ target 1755 → offset 0, AlreadyAtTarget, never raised.
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        let b = plan.iter().find(|e| e.voltage_mv == 900).unwrap();
        assert_eq!(b.desired_offset_mhz, 0);
        assert!(b.in_flatten_set);
        assert_eq!(b.class, VfBinClass::AlreadyAtTarget);
    }

    #[test]
    fn monotone_below_ceiling_is_elastic_zero() {
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        for e in plan.iter().filter(|e| e.voltage_mv < 900) {
            assert_eq!(e.desired_offset_mhz, 0);
            assert!(e.below_ceiling && !e.in_flatten_set);
            assert_eq!(e.class, VfBinClass::BelowCeiling);
        }
    }

    #[test]
    fn monotone_nonzero_bins_are_at_or_above_ceiling() {
        // Audit B2: every non-zero offset bin's static-base voltage is ≥ ceiling.
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        for e in plan.iter().filter(|e| e.desired_offset_mhz != 0) {
            assert!(e.voltage_mv >= 900);
        }
    }

    #[test]
    fn monotone_1755_at_900_zero_overshoot() {
        // Reconstruct the predicted plateau over the flatten set: effective = base + offset.
        let plan = plan_vf_ceiling_monotone(&base_curve(), 900, 1755);
        let predicted_max = plan
            .iter()
            .filter(|e| e.in_flatten_set)
            .map(|e| e.base_mhz as i32 + e.desired_offset_mhz)
            .max()
            .unwrap();
        assert_eq!(predicted_max, 1755);
        assert_eq!((predicted_max - 1755).max(0), 0); // overshoot == 0
    }

    #[test]
    fn monotone_empty_curve_is_empty_plan() {
        // No bins → empty plan → writer maps this to a fail-closed Err (no partial unsafe plan).
        assert!(plan_vf_ceiling_monotone(&[], 900, 1755).is_empty());
    }

    #[test]
    fn monotone_no_bins_above_ceiling_has_no_flatten_set() {
        // Ceiling above every bin → nothing in the flatten set → writer fails closed.
        let plan = plan_vf_ceiling_monotone(&base_curve(), 2000, 1755);
        assert!(plan.iter().all(|e| !e.in_flatten_set));
        assert!(plan.iter().all(|e| e.desired_offset_mhz == 0));
    }

    // ── plan_bounded_positive_offset (F2 true-undervolt, bounded positive raise) ──────────────
    // Static base curve (index, voltage_mv, base_freq_mhz): lower-voltage bins have lower base, so a
    // small positive offset lets them hold a higher target.
    fn pos_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1700), (1, 900, 1740), (2, 950, 1770), (3, 1000, 1800), (4, 1062, 1845),
        ]
    }

    fn pos_limits(floor_mv: u32) -> PositiveOffsetLimits {
        // Conservative built-in caps (+30 abs, +15 step) with a generous clock ceiling.
        PositiveOffsetLimits::conservative(floor_mv, 1900)
    }

    #[test]
    fn pos_offset_computes_expected_offset_for_valid_lower_bin() {
        // 900 mV bin base 1740, target 1755 → +15 offset; effective == target; prev 0 → step 15.
        let plan = plan_bounded_positive_offset(&pos_base(), 1, 1755, 0, &pos_limits(850)).unwrap();
        assert_eq!(plan.voltage_mv, 900);
        assert_eq!(plan.base_mhz, 1740);
        assert_eq!(plan.offset_mhz, 15);
        assert_eq!(plan.effective_mhz, 1755);
        assert_eq!(plan.step_delta_mhz, 15);
    }

    #[test]
    fn pos_offset_rejects_above_absolute_bound() {
        // 850 mV bin base 1700, target 1735 → +35 offset > +30 abs cap. prev 30 → step 5 (within),
        // floor 850 ok → fails ONLY on the absolute cap.
        let err = plan_bounded_positive_offset(&pos_base(), 0, 1735, 30, &pos_limits(850)).unwrap_err();
        assert!(err.contains("absolute cap"), "unexpected error: {err}");
    }

    #[test]
    fn pos_offset_rejects_per_step_bound_violation() {
        // 950 mV bin base 1770, target 1795 → +25 offset (≤ +30 abs) but prev 0 → step +25 > +15.
        let err = plan_bounded_positive_offset(&pos_base(), 2, 1795, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("per-step"), "unexpected error: {err}");
    }

    #[test]
    fn pos_offset_rejects_nonsane_foreign_and_empty_base() {
        // Empty base → fail closed.
        assert!(plan_bounded_positive_offset(&[], 0, 1755, 0, &pos_limits(850)).is_err());
        // Foreign / memory-domain point (freq 9000 MHz) → non-sane base, fail closed.
        let foreign = vec![(0usize, 900u32, 1740u32), (1, 1500, 9000)];
        let err = plan_bounded_positive_offset(&foreign, 0, 1755, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("non-sane") || err.contains("foreign"), "unexpected error: {err}");
        // A real index that is simply not on the curve → fail closed.
        assert!(plan_bounded_positive_offset(&pos_base(), 99, 1755, 0, &pos_limits(850)).is_err());
    }

    #[test]
    fn pos_offset_refuses_below_hardware_floor() {
        // Floor 950 mV; the 850 mV bin is below it → fails on the floor (before any offset math).
        let err = plan_bounded_positive_offset(&pos_base(), 0, 1710, 0, &pos_limits(950)).unwrap_err();
        assert!(err.contains("hardware floor"), "unexpected error: {err}");
    }

    #[test]
    fn pos_offset_rejects_non_positive_offset() {
        // 1062 mV bin base 1845 ≥ target 1755 → offset ≤ 0 → positive-offset-only, fail closed.
        let err = plan_bounded_positive_offset(&pos_base(), 4, 1755, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("positive-offset-only"), "unexpected error: {err}");
    }

    #[test]
    fn pos_offset_rejects_clock_above_ceiling() {
        // Ceiling 1750 MHz; 900 mV bin base 1740, target 1755 (+15, within offset caps) → planned
        // clock 1755 > ceiling 1750 → fail closed.
        let limits = PositiveOffsetLimits::conservative(850, 1750);
        let err = plan_bounded_positive_offset(&pos_base(), 1, 1755, 0, &limits).unwrap_err();
        assert!(err.contains("clock ceiling"), "unexpected error: {err}");
    }

    // ── plan_bounded_anchored_positive_offset (F2 anchored classic undervolt point) ───────────
    // pos_base(): (idx, mV, base) = 850/1700, 900/1740, 950/1770, 1000/1800, 1062/1845.
    // Anchor the 900 mV bin (base 1740) at target 1755: +15 raise; the 950/1000/1062 mV bins are all
    // above target → capped DOWN; the 850 mV bin is below the anchor → elastic.
    #[test]
    fn anchored_raises_selected_bin_to_target() {
        let plan =
            plan_bounded_anchored_positive_offset(&pos_base(), 1, 1755, 0, &pos_limits(850)).unwrap();
        assert_eq!(plan.target_mhz, 1755);
        let a = plan.entries.iter().find(|e| e.role == AnchoredBinRole::Anchor).unwrap();
        assert_eq!((a.voltage_mv, a.base_mhz, a.offset_mhz, a.effective_mhz), (900, 1740, 15, 1755));
        assert_eq!(plan.anchor.index, a.index);
        assert_eq!(plan.max_positive_offset_mhz, 15);
        // Exactly one positive offset, and it is the anchor.
        assert_eq!(plan.entries.iter().filter(|e| e.offset_mhz > 0).count(), 1);
    }

    #[test]
    fn anchored_caps_all_higher_voltage_bins_to_target() {
        let plan =
            plan_bounded_anchored_positive_offset(&pos_base(), 1, 1755, 0, &pos_limits(850)).unwrap();
        let caps: Vec<_> =
            plan.entries.iter().filter(|e| e.role == AnchoredBinRole::CappedAbove).collect();
        // 950 (1770→-15), 1000 (1800→-45), 1062 (1845→-90) — all land exactly at target, offset ≤ 0.
        assert_eq!(caps.len(), 3);
        for e in &caps {
            assert!(e.offset_mhz <= 0);
            assert_eq!(e.effective_mhz, 1755);
            assert_eq!(e.base_mhz as i32 + e.offset_mhz, 1755);
        }
        assert_eq!(plan.capped_above_bins, 3);
        assert_eq!(plan.max_negative_flatten_mhz, 90);
        // No higher-voltage bin remains above the target.
        assert!(caps.iter().all(|e| e.effective_mhz <= 1755));
    }

    #[test]
    fn anchored_does_not_over_raise_lower_voltage_bins() {
        let plan =
            plan_bounded_anchored_positive_offset(&pos_base(), 1, 1755, 0, &pos_limits(850)).unwrap();
        let below: Vec<_> =
            plan.entries.iter().filter(|e| e.role == AnchoredBinRole::ElasticBelow).collect();
        // The 850 mV bin (below the 900 mV anchor) stays elastic at offset 0 — never raised.
        assert_eq!(below.len(), 1);
        assert_eq!((below[0].voltage_mv, below[0].offset_mhz), (850, 0));
        assert_eq!(plan.elastic_below_bins, 1);
    }

    #[test]
    fn anchored_rejects_positive_offset_above_absolute_cap() {
        // Anchor the 850 mV bin (base 1700) at 1735 → +35 > +30 abs cap; prev 30 → step 5 (within),
        // floor 850 ok → rejects ONLY on the absolute cap (inherited from the single-bin planner).
        let err = plan_bounded_anchored_positive_offset(&pos_base(), 0, 1735, 30, &pos_limits(850))
            .unwrap_err();
        assert!(err.contains("absolute cap"), "unexpected error: {err}");
    }

    #[test]
    fn anchored_rejects_per_step_cap_violation() {
        // Anchor the 950 mV bin (base 1770) at 1795 → +25 (≤ +30 abs) but prev 0 → step +25 > +15.
        let err =
            plan_bounded_anchored_positive_offset(&pos_base(), 2, 1795, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("per-step"), "unexpected error: {err}");
    }

    #[test]
    fn anchored_rejects_target_above_clock_ceiling() {
        // Ceiling 1750 MHz; anchoring the 900 mV bin at 1755 → planned clock 1755 > 1750 → fail closed.
        let limits = PositiveOffsetLimits::conservative(850, 1750);
        let err = plan_bounded_anchored_positive_offset(&pos_base(), 1, 1755, 0, &limits).unwrap_err();
        assert!(err.contains("clock ceiling"), "unexpected error: {err}");
    }

    #[test]
    fn anchored_rejects_non_real_bin_and_malformed_curve() {
        // Index not on the curve → fail closed.
        assert!(plan_bounded_anchored_positive_offset(&pos_base(), 99, 1755, 0, &pos_limits(850)).is_err());
        // Empty base curve → fail closed.
        assert!(plan_bounded_anchored_positive_offset(&[], 0, 1755, 0, &pos_limits(850)).is_err());
        // Foreign / non-sane base (freq 9000 MHz) → fail closed.
        let foreign = vec![(0usize, 900u32, 1740u32), (1, 1500, 9000)];
        let err = plan_bounded_anchored_positive_offset(&foreign, 0, 1755, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("non-sane") || err.contains("foreign"), "unexpected error: {err}");
    }

    #[test]
    fn anchored_rejects_non_monotone_lower_bin_above_target() {
        // A lower-voltage bin whose base already sits ABOVE target is non-monotone/unsafe. Anchor the
        // 1000 mV bin (base 1800) at 1810 (+10); the 950 mV lower bin base 1770 ≤ 1810 is fine, but a
        // crafted curve with a below-anchor bin above target must be rejected.
        let curve = vec![(0usize, 850u32, 1820u32), (1, 1000, 1800)];
        // Anchor index 1 (1000 mV, base 1800) at 1810 → +10. The 850 mV bin base 1820 > 1810 target.
        let err =
            plan_bounded_anchored_positive_offset(&curve, 1, 1810, 0, &pos_limits(850)).unwrap_err();
        assert!(err.contains("non-monotone"), "unexpected error: {err}");
    }

    #[test]
    fn hardware_frontier_limits_come_from_real_clock_domain() {
        let limits = PositiveOffsetLimits::hardware_frontier(850, 1950, 1500);
        assert_eq!(limits.abs_max_offset_mhz, 450);
        assert_eq!(limits.step_max_offset_mhz, 450);
        assert_eq!((limits.hw_floor_mv, limits.clock_ceiling_mhz), (850, 1950));
    }

    #[test]
    fn hardware_frontier_allows_deep_real_bin_but_never_clock_above_stock_top() {
        let limits = PositiveOffsetLimits::hardware_frontier(850, 1950, 1500);
        let plan = plan_bounded_positive_offset(&pos_base(), 0, 1900, 0, &limits).unwrap();
        assert_eq!((plan.offset_mhz, plan.effective_mhz), (200, 1900));
        let err = plan_bounded_positive_offset(&pos_base(), 0, 1965, 0, &limits).unwrap_err();
        assert!(err.contains("clock ceiling"), "unexpected error: {err}");
    }

    // ── TARGET-SWEEP learned offset horizon (official --auto-sweep envelope) ──────────────────
    // The horizon raises ONLY the absolute cap (+210) while keeping the per-step cap at +15, so a
    // deeper bin is reachable solely through validated chained +15 increments — never one big jump.
    fn horizon_limits(floor_mv: u32) -> PositiveOffsetLimits {
        PositiveOffsetLimits::target_sweep_learning_horizon(floor_mv, 2000)
    }

    #[test]
    fn target_sweep_horizon_raises_only_absolute_cap_keeps_per_step() {
        let h = PositiveOffsetLimits::target_sweep_learning_horizon(850, 1900);
        assert_eq!(h.abs_max_offset_mhz, TARGET_SWEEP_HORIZON_MAX_MHZ);
        assert_eq!(h.abs_max_offset_mhz, 210);
        // CRITICAL: per-step stays conservative (unlike manual_prior, which widens BOTH caps).
        assert_eq!(h.step_max_offset_mhz, POS_OFFSET_STEP_MAX_MHZ);
        assert_eq!(h.step_max_offset_mhz, 15);
        // Floor / ceiling pass through unchanged from the caller (same as `conservative`).
        assert_eq!((h.hw_floor_mv, h.clock_ceiling_mhz), (850, 1900));
    }

    #[test]
    fn target_sweep_horizon_allows_plus45_after_validated_plus30() {
        // The +30 default ABSOLUTE cap saturates here: the 850 mV bin (base 1700) at 1745 needs +45.
        // Under the conservative envelope +45 > +30 → rejected; under the horizon, +45 ≤ +210 AND the
        // per-step delta from the last validated +30 is +15 ≤ +15 → it PLANS.
        assert!(plan_bounded_positive_offset(&pos_base(), 0, 1745, 30, &pos_limits(850)).is_err());
        let plan =
            plan_bounded_positive_offset(&pos_base(), 0, 1745, 30, &horizon_limits(850)).unwrap();
        assert_eq!(plan.offset_mhz, 45);
        assert_eq!(plan.step_delta_mhz, 15);
        assert_eq!(plan.effective_mhz, 1745);
    }

    #[test]
    fn target_sweep_horizon_still_rejects_per_step_jump_past_15() {
        // Even under the horizon, a +60 candidate after a validated +30 is a +30 per-step jump
        // (> +15) → rejected on the per-step cap; the +210 absolute cap is NOT the limiter here.
        let err =
            plan_bounded_positive_offset(&pos_base(), 0, 1760, 30, &horizon_limits(850)).unwrap_err();
        assert!(err.contains("per-step"), "unexpected error: {err}");
    }

    #[test]
    fn target_sweep_horizon_rejects_offset_above_hard_cap() {
        // An absolute offset past the +210 horizon hard cap is rejected even with a within-cap
        // per-step delta and a clock under the ceiling — the horizon is still a hard, bounded ceiling.
        let err =
            plan_bounded_positive_offset(&pos_base(), 0, 1911, 200, &horizon_limits(850)).unwrap_err();
        assert!(err.contains("absolute cap"), "unexpected error: {err}");
    }

    #[test]
    fn horizon_does_not_perturb_conservative_or_manual_prior_caps() {
        // Regression: the new constructor must leave the other two envelopes byte-for-byte the same.
        let c = PositiveOffsetLimits::conservative(850, 1900);
        assert_eq!(
            (c.abs_max_offset_mhz, c.step_max_offset_mhz),
            (POS_OFFSET_MAX_MHZ, POS_OFFSET_STEP_MAX_MHZ)
        );
        assert_eq!((c.abs_max_offset_mhz, c.step_max_offset_mhz), (30, 15));
        // Manual-prior still widens BOTH caps to the operator max (single-shot known point).
        let m = PositiveOffsetLimits::manual_prior(850, 1900, 250);
        assert_eq!((m.abs_max_offset_mhz, m.step_max_offset_mhz), (250, 250));
    }
}
