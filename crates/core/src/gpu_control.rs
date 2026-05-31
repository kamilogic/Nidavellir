//! GPU tuning backends and the stability evaluator, behind traits so the pure
//! [`crate::gpu_sweep`] engine can run against either real hardware or a
//! deterministic simulation (roadmap honesty principle: never claim to apply
//! something we didn't).
//!
//! - [`GpuTuner`] — apply/reset a V/F point + power limit. [`SimulatedGpuTuner`]
//!   records intent; [`NvmlReadOnlyTuner`] reads real telemetry via NVML but
//!   refuses writes until the NVAPI `NvAPI_GPU_SetVFPCurve` path is wired and
//!   validated (a deliberate future increment).
//! - [`StabilityEvaluator`] — judge a point Stable / SilentError / Crash. In
//!   production this is the Vulkan stressor + compute validation + WHEA delta;
//!   here [`SimulatedSilicon`] models a frequency-dependent stability frontier.

use crate::gpu_sweep::{StabilityResult, VfPoint};

/// Live GPU telemetry sampled during a dwell (subset relevant to evaluation).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuTelemetry {
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub core_clock_mhz: Option<u32>,
}

/// Applies and reverts GPU V/F points. Reversible, no reboot (roadmap §2.3).
pub trait GpuTuner: Send {
    /// Whether this backend actually writes to hardware.
    fn is_real(&self) -> bool;
    /// A short human label for the UI ("simulado" vs "NVAPI").
    fn backend_label(&self) -> &'static str;
    /// Read the current V/F operating point, if available.
    fn read_current(&self) -> Option<VfPoint>;
    /// Apply a V/F point. Returns Err if the backend cannot write.
    fn apply(&mut self, point: VfPoint) -> Result<(), String>;
    /// Restore stock (remove all offsets).
    fn reset(&mut self) -> Result<(), String>;
}

/// Judges whether an applied point is stable (the dwell verdict).
pub trait StabilityEvaluator: Send {
    fn evaluate(&mut self, point: VfPoint, telemetry: &GpuTelemetry) -> StabilityResult;
}

/// Deterministic simulation of a silicon stability frontier (for tests, demos,
/// and running the full pipeline end-to-end without touching the GPU).
///
/// The minimum stable voltage rises linearly with frequency. A point a little
/// below the frontier produces *silent errors*; far below it *crashes* — the
/// exact distinction the real engine cares about (§12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatedSilicon {
    /// vmin at `ref_freq_mhz`.
    pub base_mv: u32,
    /// Reference frequency for `base_mv`.
    pub ref_freq_mhz: u32,
    /// Added mV per MHz above the reference (×1000 for integer precision).
    pub slope_uv_per_mhz: u32,
    /// How far below the frontier (mV) tips from silent error into a crash.
    pub crash_margin_mv: u32,
}

impl Default for SimulatedSilicon {
    fn default() -> Self {
        Self {
            base_mv: 850,
            ref_freq_mhz: 1800,
            slope_uv_per_mhz: 800, // 0.8 mV/MHz
            crash_margin_mv: 60,
        }
    }
}

impl SimulatedSilicon {
    /// The true minimum stable voltage for a frequency.
    pub fn frontier_mv(&self, freq_mhz: u32) -> u32 {
        let delta_mhz = freq_mhz as i64 - self.ref_freq_mhz as i64;
        let delta_mv = delta_mhz * self.slope_uv_per_mhz as i64 / 1000;
        (self.base_mv as i64 + delta_mv).max(0) as u32
    }
}

impl StabilityEvaluator for SimulatedSilicon {
    fn evaluate(&mut self, point: VfPoint, _telemetry: &GpuTelemetry) -> StabilityResult {
        let frontier = self.frontier_mv(point.freq_mhz);
        if point.voltage_mv >= frontier {
            StabilityResult::Stable
        } else if point.voltage_mv + self.crash_margin_mv >= frontier {
            StabilityResult::SilentError
        } else {
            StabilityResult::Crash
        }
    }
}

/// Simulated tuner — records the last applied point; no hardware touched.
#[derive(Debug, Clone, Default)]
pub struct SimulatedGpuTuner {
    pub applied: Option<VfPoint>,
    pub stock: Option<VfPoint>,
}

impl SimulatedGpuTuner {
    pub fn new(stock: VfPoint) -> Self {
        Self { applied: Some(stock), stock: Some(stock) }
    }
}

impl GpuTuner for SimulatedGpuTuner {
    fn is_real(&self) -> bool {
        false
    }
    fn backend_label(&self) -> &'static str {
        "simulated"
    }
    fn read_current(&self) -> Option<VfPoint> {
        self.applied.or(self.stock)
    }
    fn apply(&mut self, point: VfPoint) -> Result<(), String> {
        self.applied = Some(point);
        Ok(())
    }
    fn reset(&mut self) -> Result<(), String> {
        self.applied = self.stock;
        Ok(())
    }
}

/// Real NVIDIA backend: reads telemetry via NVML, but does **not** yet write.
///
/// Reading the live V/F point and applying a custom curve requires NVAPI
/// (`NvAPI_GPU_SetVFPCurve`), which is a separate native-binding effort gated on
/// careful validation. Until then this backend is honest: writes return an
/// explanatory error and the sweep must use [`SimulatedGpuTuner`].
#[derive(Debug, Clone, Default)]
pub struct NvmlReadOnlyTuner;

impl GpuTuner for NvmlReadOnlyTuner {
    fn is_real(&self) -> bool {
        true
    }
    fn backend_label(&self) -> &'static str {
        "NVAPI (read-only)"
    }
    fn read_current(&self) -> Option<VfPoint> {
        let reading = crate::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next()?;
        Some(VfPoint {
            freq_mhz: reading.core_clock_mhz?,
            // NVML does not expose the per-point voltage on consumer cards;
            // left at 0 so callers know it is not a real measurement.
            voltage_mv: 0,
        })
    }
    fn apply(&mut self, _point: VfPoint) -> Result<(), String> {
        Err("Real V/F curve writes via NVAPI are not wired yet (v0.3 ships the engine; real \
             writes land once the NVAPI bindings are validated). Use the simulated backend."
            .into())
    }
    fn reset(&mut self) -> Result<(), String> {
        // No offsets were applied, so reset is a safe no-op.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_frontier_rises_with_frequency() {
        let s = SimulatedSilicon::default();
        assert!(s.frontier_mv(1950) > s.frontier_mv(1800));
        assert_eq!(s.frontier_mv(1800), 850);
    }

    #[test]
    fn simulated_evaluator_three_zones() {
        let mut s = SimulatedSilicon::default();
        let t = GpuTelemetry::default();
        let f = s.frontier_mv(1800); // 850
        assert_eq!(s.evaluate(VfPoint { freq_mhz: 1800, voltage_mv: f }, &t), StabilityResult::Stable);
        assert_eq!(
            s.evaluate(VfPoint { freq_mhz: 1800, voltage_mv: f - 20 }, &t),
            StabilityResult::SilentError
        );
        assert_eq!(
            s.evaluate(VfPoint { freq_mhz: 1800, voltage_mv: f - 200 }, &t),
            StabilityResult::Crash
        );
    }

    #[test]
    fn simulated_tuner_records_and_resets() {
        let stock = VfPoint { freq_mhz: 1800, voltage_mv: 1050 };
        let mut t = SimulatedGpuTuner::new(stock);
        assert_eq!(t.read_current(), Some(stock));
        let p = VfPoint { freq_mhz: 1800, voltage_mv: 900 };
        t.apply(p).unwrap();
        assert_eq!(t.read_current(), Some(p));
        t.reset().unwrap();
        assert_eq!(t.read_current(), Some(stock));
        assert!(!t.is_real());
    }

    #[test]
    fn nvml_backend_refuses_writes() {
        let mut t = NvmlReadOnlyTuner;
        assert!(t.is_real());
        assert!(t.apply(VfPoint { freq_mhz: 1800, voltage_mv: 900 }).is_err());
        assert!(t.reset().is_ok());
    }
}
