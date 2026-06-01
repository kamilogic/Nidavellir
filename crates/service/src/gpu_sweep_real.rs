//! Real GPU undervolt sweep — drives the tested `GpuSweepEngine` against actual
//! hardware: frequency-anchored, voltage-descending (the gentle direction that
//! fails as silent errors, not TDRs). Each step locks the voltage, offsets the
//! clock to hold the target frequency, runs the compute-validation battery, and
//! stops on the first instability. Safe Loop armed per step; always resets.
//!
//! Windows-only (NVAPI). On other targets the handle is inert.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::{
    GpuSweepConfig, GpuSweepEngine, GpuSweepProgress, StabilityResult, SweepCommand, SweepPhase,
};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

fn idle() -> GpuSweepProgress {
    GpuSweepProgress {
        phase: SweepPhase::Idle,
        current: None,
        freq_index: 0,
        total_freqs: 0,
        tradeoffs: Vec::new(),
        profiles: None,
        simulated: false,
    }
}

#[derive(Clone)]
pub struct RealSweepHandle {
    progress: Arc<Mutex<GpuSweepProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for RealSweepHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl RealSweepHandle {
    pub fn progress(&self) -> GpuSweepProgress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_else(|_| idle())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn start(&self, store: SafeLoopStore) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            #[cfg(windows)]
            run_real_sweep(progress, stop, store);
            #[cfg(not(windows))]
            {
                let _ = (&progress, &stop, &store);
            }
            running.store(false, Ordering::SeqCst);
        });
        true
    }
}

#[cfg(windows)]
fn publish(progress: &Arc<Mutex<GpuSweepProgress>>, p: GpuSweepProgress) {
    if let Ok(mut g) = progress.lock() {
        *g = p;
    }
}

/// Stock curve frequency (MHz) at or just below a given voltage (mV).
#[cfg(windows)]
fn curve_freq_at(curve: &[(u32, u32)], voltage_mv: u32) -> Option<u32> {
    curve
        .iter()
        .filter(|(v, _)| *v <= voltage_mv)
        .max_by_key(|(v, _)| *v)
        .map(|(_, f)| *f)
        .or_else(|| curve.iter().min_by_key(|(v, _)| *v).map(|(_, f)| *f))
}

#[cfg(windows)]
fn run_real_sweep(
    progress: Arc<Mutex<GpuSweepProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
) {
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    info!("Real GPU sweep starting");

    // Read the stock V/F curve to map target frequency -> required offset.
    let curve_snapshot = match gpu::read_curve() {
        Ok(c) => c,
        Err(e) => {
            warn!("real sweep: read_curve failed: {e}");
            publish(&progress, idle());
            return;
        }
    };
    let curve: Vec<(u32, u32)> =
        curve_snapshot.points.iter().map(|p| (p.voltage_mv, p.freq_mhz)).collect();
    let plateau = curve_snapshot.plateau().map(|p| p.freq_mhz).unwrap_or(1800);

    // Target the top of the curve (where performance lives), in 15 MHz Ampere
    // steps; bisect voltage downward for each.
    let config = GpuSweepConfig {
        target_freqs_mhz: vec![plateau, plateau.saturating_sub(15), plateau.saturating_sub(30)],
        stock_mv: 1000,
        floor_mv: 850, // don't undervolt a high clock below this — safety
        descent_step_mv: 25,
        min_step_mv: 7, // ~ the 6.25 mV native step
        fixed_margin_mv: 20,
        temp_coeff_mv_per_c: 1,
        temp_headroom_c: 15,
    };

    let ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("real sweep: GpuCtx init failed: {e}");
            publish(&progress, idle());
            return;
        }
    };

    let _ = gpu::reset_all();
    let mut engine = GpuSweepEngine::new(config, plateau, false);

    loop {
        if stop.load(Ordering::SeqCst) {
            engine.abort();
            break;
        }
        match engine.next_command() {
            SweepCommand::ApplyAndTest { point } => {
                // Safe Loop: arm before touching hardware.
                let intent = TuningPoint::from_axes([
                    ("gpu_freq_mhz", point.freq_mhz as i64),
                    ("gpu_voltage_mv", point.voltage_mv as i64),
                ]);
                let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_real_sweep"));

                // Realize (freq, voltage): lock V, offset = target - curve(V).
                let base = curve_freq_at(&curve, point.voltage_mv).unwrap_or(point.freq_mhz);
                let offset = (point.freq_mhz as i64 - base as i64).clamp(-200, 300) as i32;
                let applied = gpu::lock_core_voltage_mv(point.voltage_mv)
                    .and_then(|_| gpu::set_core_offset_mhz(offset));
                if let Err(e) = applied {
                    warn!("real sweep: apply failed: {e}; aborting");
                    engine.abort();
                    break;
                }

                // Validate (battery). A device-lost panics wgpu — catch it.
                let result = match catch_unwind(AssertUnwindSafe(|| validate_step(&ctx))) {
                    Ok(r) => r,
                    Err(_) => {
                        warn!("real sweep: device lost during validation (treated as Crash)");
                        StabilityResult::Crash
                    }
                };

                if result.is_stable() {
                    let _ = store.clear_boot_flag();
                }

                engine.record(result);
                let mut p = engine.progress();
                p.simulated = false;
                publish(&progress, p);

                if matches!(result, StabilityResult::Crash) {
                    // The validation device is gone; can't continue safely.
                    engine.abort();
                    break;
                }
            }
            SweepCommand::Finished => break,
        }
    }

    // Always restore stock.
    let _ = gpu::reset_all();
    let _ = store.clear_boot_flag();
    let mut p = engine.progress();
    p.simulated = false;
    publish(&progress, p);
    info!("Real GPU sweep finished: {:?}", engine.phase());
}

/// One validation pass: ALU + memory + burst; worst verdict wins.
#[cfg(windows)]
fn validate_step(ctx: &nidavellir_gpu_stress::GpuCtx) -> StabilityResult {
    fn worst(a: StabilityResult, b: StabilityResult) -> StabilityResult {
        use StabilityResult::*;
        match (a, b) {
            (Crash, _) | (_, Crash) => Crash,
            (SilentError, _) | (_, SilentError) => SilentError,
            _ => Stable,
        }
    }
    let a = ctx.run_alu("alu", 400_000, 8_000, 1).result;
    let m = ctx.run_memory("mem", 300_000, 3_000).result;
    let b = ctx.run_alu("burst", 400_000, 1_500, 8).result;
    worst(worst(a, m), b)
}
