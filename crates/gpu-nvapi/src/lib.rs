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

    let points = curve
        .graphics
        .iter()
        .map(|(_, e)| VfCurvePoint {
            voltage_mv: e.voltage.0 / 1000,
            // Kilohertz2's `.0` is kHz here (its Display divides by 1000); the
            // From<Kilohertz2> conversion halves, so read the field directly.
            freq_mhz: e.frequency.0 / 1000,
        })
        .collect();

    Ok(GpuCurve { name, points })
}

#[cfg(not(windows))]
pub fn read_curve() -> Result<GpuCurve, String> {
    Err("NVAPI is Windows-only".into())
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
