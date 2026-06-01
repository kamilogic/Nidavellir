//! Real GPU undervolt sweep — the **lock-voltage / raise-clock** mechanic that
//! actually works on hardware (the earlier voltage-bisection reuse silently held
//! the wrong frequency). For each of a few voltages it locks that voltage and
//! raises the core clock offset until the compute-validation battery flags
//! instability, **measuring the real clock/voltage/temperature each step** and
//! reporting them. Stops at the first silent error, backs off a margin.
//!
//! Builds (frequency, voltage) points → synthesizes the three profiles. Safe
//! Loop armed per step; device-lost is caught; always resets to stock.
//! Windows-only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::{
    synthesize_profiles, GpuSweepProgress, StabilityResult, SweepPhase, TradeoffPoint, VfPoint,
};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

/// Sweep quality preset.
#[derive(Clone, Copy)]
pub struct Quality {
    /// Validation passes per step (more = longer dwell = catches drift).
    pub passes: u32,
    /// Clock offset step (MHz).
    pub step_mhz: i32,
    /// Frequency margin backed off from the measured cliff (MHz).
    pub margin_mhz: u32,
    /// Warm-up seconds to reach thermal equilibrium before sweeping.
    pub warmup_s: u64,
    /// Voltages (mV) to characterize, high → low.
    pub voltages: &'static [u32],
    /// Offset ceiling (MHz) — hard safety cap.
    pub cap_mhz: i32,
}

impl Quality {
    pub fn fast() -> Self {
        Self { passes: 1, step_mhz: 30, margin_mhz: 60, warmup_s: 5, voltages: &[925, 875], cap_mhz: 330 }
    }
    pub fn thorough() -> Self {
        Self { passes: 3, step_mhz: 15, margin_mhz: 90, warmup_s: 12, voltages: &[950, 925, 900, 875], cap_mhz: 390 }
    }
}

fn idle() -> GpuSweepProgress {
    GpuSweepProgress {
        phase: SweepPhase::Idle,
        current: None,
        freq_index: 0,
        total_freqs: 0,
        tradeoffs: Vec::new(),
        profiles: None,
        simulated: false,
        measured_mhz: None,
        gpu_temp_c: None,
        last_result: None,
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
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn start(&self, store: SafeLoopStore, quality: Quality) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            #[cfg(windows)]
            run_real_sweep(progress, stop, store, quality);
            #[cfg(not(windows))]
            {
                let _ = (&progress, &stop, &store, &quality);
            }
            running.store(false, Ordering::SeqCst);
        });
        true
    }
}

#[cfg(windows)]
fn set_progress(progress: &Arc<Mutex<GpuSweepProgress>>, p: GpuSweepProgress) {
    if let Ok(mut g) = progress.lock() {
        *g = p;
    }
}

/// Peak core clock + temperature sampled via NVML while `body` runs (the load).
#[cfg(windows)]
fn measure_during<R>(body: impl FnOnce() -> R) -> (R, u32, Option<f32>) {
    use std::sync::atomic::AtomicU32;
    let peak = Arc::new(AtomicU32::new(0));
    let temp = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (p2, t2, s2) = (peak.clone(), temp.clone(), stop.clone());
    let sampler = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let Some(c) = r.core_clock_mhz {
                    p2.fetch_max(c, Ordering::SeqCst);
                }
                if let Some(t) = r.temperature_c {
                    t2.store(t as u32, Ordering::SeqCst);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    });
    let r = body();
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();
    let tc = temp.load(Ordering::SeqCst);
    (r, peak.load(Ordering::SeqCst), if tc > 0 { Some(tc as f32) } else { None })
}

#[cfg(windows)]
fn validate_pass(ctx: &nidavellir_gpu_stress::GpuCtx, passes: u32) -> StabilityResult {
    fn worst(a: StabilityResult, b: StabilityResult) -> StabilityResult {
        use StabilityResult::*;
        match (a, b) {
            (Crash, _) | (_, Crash) => Crash,
            (SilentError, _) | (_, SilentError) => SilentError,
            _ => Stable,
        }
    }
    // Dwell scales with quality (passes): longer sustained load gives marginal
    // instability time to surface as a silent error *before* a hard hang.
    let ms = 2200u64 * passes.max(1) as u64;
    let a = ctx.run_alu("alu", 1_000_000, 1_000_000, ms).result;
    if !a.is_stable() {
        return a;
    }
    let m = ctx.run_memory("mem", 262_144, 2_048, ms).result;
    worst(a, m)
}

#[cfg(windows)]
fn run_real_sweep(
    progress: Arc<Mutex<GpuSweepProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
    q: Quality,
) {
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    info!("Real GPU sweep starting (lock-voltage / raise-clock)");

    let ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("real sweep: GpuCtx init failed: {e}");
            set_progress(&progress, idle());
            return;
        }
    };

    let _ = gpu::reset_all();

    let mut prog = idle();
    prog.total_freqs = q.voltages.len();

    // Phase 1 gate: VRAM must be sound at stock, else tuning the core is moot.
    prog.phase = SweepPhase::VramDiagnostic;
    set_progress(&progress, prog.clone());
    let vram = match catch_unwind(AssertUnwindSafe(|| ctx.run_vram_check(4 * 1024 * 1024 * 1024, 2))) {
        Ok(r) => r.result,
        Err(_) => StabilityResult::Crash,
    };
    if !vram.is_stable() {
        warn!("real sweep: VRAM check failed at stock ({vram:?}) — aborting before tuning");
        let _ = gpu::reset_all();
        prog.phase = SweepPhase::Aborted;
        prog.last_result = Some(vram);
        set_progress(&progress, prog);
        return;
    }

    // Warm up to thermal equilibrium (the stability frontier moves with temp).
    prog.phase = SweepPhase::Baseline;
    set_progress(&progress, prog.clone());
    let warm_deadline = std::time::Instant::now() + std::time::Duration::from_secs(q.warmup_s);
    while std::time::Instant::now() < warm_deadline && !stop.load(Ordering::SeqCst) {
        let _ = catch_unwind(AssertUnwindSafe(|| ctx.run_alu("warmup", 600_000, 6_000, 1)));
    }

    let mut points: Vec<TradeoffPoint> = Vec::new();
    let mut crashed = false;

    'voltages: for (vi, &v) in q.voltages.iter().enumerate() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = gpu::lock_core_voltage_mv(v) {
            warn!("real sweep: lock {v}mV failed: {e}; aborting");
            break;
        }
        prog.phase = SweepPhase::VoltageBisection;
        prog.freq_index = vi;

        let mut best_stable: Option<u32> = None;
        let mut offset = 0i32;

        loop {
            if stop.load(Ordering::SeqCst) {
                break 'voltages;
            }
            // Arm Safe Loop before touching hardware.
            let intent = TuningPoint::from_axes([
                ("gpu_voltage_mv", v as i64),
                ("gpu_offset_mhz", offset as i64),
            ]);
            let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_real_sweep"));

            if let Err(e) = gpu::set_core_offset_mhz(offset) {
                warn!("real sweep: set offset failed: {e}");
                break;
            }

            let (result, peak, temp) =
                measure_during(|| match catch_unwind(AssertUnwindSafe(|| validate_pass(&ctx, q.passes))) {
                    Ok(r) => r,
                    Err(_) => StabilityResult::Crash,
                });

            let v_meas = gpu::read_core_voltage_mv();
            prog.current = Some(VfPoint { freq_mhz: peak, voltage_mv: v });
            prog.measured_mhz = Some(peak);
            prog.gpu_temp_c = temp;
            prog.last_result = Some(result);
            set_progress(&progress, prog.clone());
            info!("real sweep: {v}mV +{offset}MHz -> {peak}MHz (vmeas {v_meas:?}) : {result:?}");

            match result {
                StabilityResult::Stable => {
                    let _ = store.clear_boot_flag();
                    best_stable = Some(peak);
                    offset += q.step_mhz;
                    if offset > q.cap_mhz {
                        break;
                    }
                }
                StabilityResult::SilentError => break, // cliff (gentle) — back off with margin
                StabilityResult::Crash => {
                    warn!("real sweep: device lost at {v}mV +{offset}MHz");
                    crashed = true;
                    break 'voltages;
                }
            }
        }

        // Record the safe point for this voltage: max stable clock minus margin.
        if let Some(best) = best_stable {
            let freq = best.saturating_sub(q.margin_mhz);
            if freq > 0 {
                points.push(TradeoffPoint { freq_mhz: freq, vmin_mv: v });
            }
        }
        prog.tradeoffs = points.clone();
        set_progress(&progress, prog.clone());

        // Drop the clock offset between voltages (re-lock next iteration).
        let _ = gpu::set_core_offset_mhz(0);
    }

    let _ = gpu::reset_all();
    let _ = store.clear_boot_flag();

    let baseline = points.iter().map(|p| p.freq_mhz).max().unwrap_or(0);
    prog.profiles = synthesize_profiles(baseline, &points);
    prog.tradeoffs = points;
    prog.current = None;
    prog.phase = if crashed { SweepPhase::Aborted } else { SweepPhase::Done };
    prog.freq_index = prog.total_freqs;
    set_progress(&progress, prog);
    info!("Real GPU sweep finished (crashed={crashed})");
}
