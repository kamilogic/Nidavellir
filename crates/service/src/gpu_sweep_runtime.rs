//! Service-side GPU sweep runner (roadmap §5/§12).
//!
//! Drives the pure [`GpuSweepEngine`] in a background thread against a
//! [`GpuTuner`] + [`StabilityEvaluator`], integrating the v0.2 Safe Loop: every
//! candidate arms the on-disk boot-flag *before* the (simulated) apply and
//! clears it once the point validates — so a real crash mid-sweep is recovered
//! on the next boot exactly like any other apply.
//!
//! The default backend is the **simulated** tuner/silicon (honest: no real V/F
//! writes happen yet — see [`nidavellir_core::gpu_control`]), so the whole
//! pipeline is observable end to end while real NVAPI writes remain a future
//! increment.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nidavellir_core::gpu_control::{
    GpuTelemetry, GpuTuner, SimulatedGpuTuner, SimulatedSilicon, StabilityEvaluator,
};
use nidavellir_core::gpu_sweep::{
    GpuSweepConfig, GpuSweepEngine, GpuSweepProgress, SweepCommand, SweepPhase, VfPoint,
};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

/// Short simulated dwell per step (real dwell is minutes; kept snappy in sim).
const DWELL: Duration = Duration::from_millis(120);

fn idle_progress() -> GpuSweepProgress {
    GpuSweepProgress {
        phase: SweepPhase::Idle,
        current: None,
        freq_index: 0,
        total_freqs: 0,
        tradeoffs: Vec::new(),
        profiles: None,
        simulated: true,
    }
}

/// Shared, thread-safe handle to the GPU sweep, stored in `AppState`.
#[derive(Clone)]
pub struct GpuSweepHandle {
    progress: Arc<Mutex<GpuSweepProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for GpuSweepHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle_progress())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GpuSweepHandle {
    pub fn progress(&self) -> GpuSweepProgress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_else(|_| idle_progress())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Request the running sweep to abort at the next step.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Start a sweep if one isn't already running. Returns false if it was busy.
    pub fn start(&self, store: SafeLoopStore) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false; // already running
        }
        self.stop.store(false, Ordering::SeqCst);

        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);

        std::thread::spawn(move || {
            run_sweep(progress.clone(), stop, store);
            running.store(false, Ordering::SeqCst);
            info!("GPU sweep thread finished");
        });
        true
    }
}

fn run_sweep(progress: Arc<Mutex<GpuSweepProgress>>, stop: Arc<AtomicBool>, store: SafeLoopStore) {
    // Baseline sustained clock would come from Phase 0; use a representative
    // value here. Stock voltage seeds the known-stable starting point.
    let config = GpuSweepConfig::default();
    let baseline_freq = *config.target_freqs_mhz.first().unwrap_or(&1800);
    let stock = VfPoint { freq_mhz: baseline_freq, voltage_mv: config.stock_mv };

    let mut tuner = SimulatedGpuTuner::new(stock);
    let mut silicon = SimulatedSilicon::default();
    let mut engine = GpuSweepEngine::new(config, baseline_freq, tuner.is_real() == false);

    info!("GPU sweep started (backend: {})", tuner.backend_label());

    loop {
        if stop.load(Ordering::SeqCst) {
            engine.abort();
            let _ = tuner.reset();
            let _ = store.clear_boot_flag();
            publish(&progress, &engine);
            warn!("GPU sweep aborted by request");
            return;
        }

        match engine.next_command() {
            SweepCommand::ApplyAndTest { point } => {
                // Safe Loop: arm the boot-flag before applying, so a crash mid
                // dwell is caught on reboot.
                let intent = TuningPoint::from_axes([
                    ("gpu_freq_mhz", point.freq_mhz as i64),
                    ("gpu_voltage_mv", point.voltage_mv as i64),
                ]);
                if let Err(e) = store.arm_boot_flag(&BootFlag::new(intent, "gpu_sweep")) {
                    warn!("GPU sweep: failed to arm boot-flag: {e}");
                }

                if let Err(e) = tuner.apply(point) {
                    // A real backend that can't write ends the sweep honestly.
                    warn!("GPU sweep: apply failed ({e}); aborting");
                    engine.abort();
                    let _ = store.clear_boot_flag();
                    publish(&progress, &engine);
                    return;
                }

                std::thread::sleep(DWELL); // dwell at thermal equilibrium (sim)
                let telemetry = sample_telemetry();
                let result = silicon.evaluate(point, &telemetry);

                // Cleared once a point survives; left armed only across a crash.
                if let Err(e) = store.clear_boot_flag() {
                    warn!("GPU sweep: failed to clear boot-flag: {e}");
                }

                engine.record(result);
                publish(&progress, &engine);
            }
            SweepCommand::Finished => {
                let _ = tuner.reset();
                let _ = store.clear_boot_flag();
                publish(&progress, &engine);
                if let Some(profiles) = engine.profiles() {
                    info!(
                        "GPU sweep done — Godforge {:?}, Brokkr's {:?}, Deep Calm {:?}",
                        profiles.godforge.point, profiles.brokkrs_best.point, profiles.deep_calm.point
                    );
                }
                return;
            }
        }
    }
}

fn publish(progress: &Arc<Mutex<GpuSweepProgress>>, engine: &GpuSweepEngine) {
    if let Ok(mut p) = progress.lock() {
        *p = engine.progress();
    }
}

/// Sample live GPU telemetry via NVML (used by a real evaluator; the simulated
/// silicon ignores it, but we read it so the value is real where it matters).
fn sample_telemetry() -> GpuTelemetry {
    match nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
        Some(r) => GpuTelemetry {
            temperature_c: r.temperature_c,
            power_w: r.power_w,
            core_clock_mhz: r.core_clock_mhz,
        },
        None => GpuTelemetry::default(),
    }
}
