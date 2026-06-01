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
        validation_note: None,
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

/// Recover the GPU context after a TDR / device-lost. The driver needs a few
/// seconds to reset the adapter; recreating the device immediately fails with
/// "lost during initialization", so we wait and retry a handful of times.
#[cfg(windows)]
fn recover_gpu_ctx() -> Option<nidavellir_gpu_stress::GpuCtx> {
    use nidavellir_gpu_stress::GpuCtx;
    for attempt in 1..=6 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        match GpuCtx::new() {
            Ok(c) => {
                info!("real sweep: GPU device recovered after TDR (attempt {attempt})");
                return Some(c);
            }
            Err(e) => warn!("real sweep: recovery attempt {attempt}/6 failed: {e}"),
        }
    }
    None
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
    // Combined core+memory load each dwell — realistic (shared voltage rail,
    // power, thermals) like a game. Dwell scales with quality so marginal
    // instability surfaces as a silent error before a hard hang.
    let ms = 2500u64 * passes.max(1) as u64;
    ctx.run_combined(ms).result
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

    let mut ctx = match GpuCtx::new() {
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
    // Best stable (voltage, offset) at the top voltage — the candidate to soak.
    let mut top_candidate: Option<(u32, i32)> = None;
    // Lowest *real* clock (MHz) ever proven unstable (silent error or device
    // lost), across all voltages. Climbing clock fails monotonically with the
    // real boost clock, so once we know a cliff we slow the approach near it to
    // surface a silent error before a hard TDR.
    let mut min_unstable_real: Option<u32> = None;
    // How close (MHz) to a known cliff before we shrink the step.
    const CLIFF_APPROACH_MHZ: u32 = 75;
    let fine_step = (q.step_mhz / 3).max(5);

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
        let mut step = q.step_mhz;

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
                    if vi == 0 {
                        top_candidate = Some((v, offset));
                    }
                    // Near a known cliff? Shrink the step so the next probe is
                    // more likely to land in the silent-error zone than to TDR.
                    if let Some(u) = min_unstable_real {
                        if peak + CLIFF_APPROACH_MHZ >= u {
                            step = fine_step;
                        }
                    }
                    offset += step;
                    if offset > q.cap_mhz {
                        break;
                    }
                }
                StabilityResult::SilentError => {
                    // Gentle cliff — record it and back off with margin.
                    min_unstable_real =
                        Some(min_unstable_real.map_or(peak, |u| u.min(peak)));
                    break;
                }
                StabilityResult::Crash => {
                    // Hard cliff (device lost / TDR). The ceiling for this
                    // voltage is the previous stable reading (already in
                    // best_stable). Do NOT abort the whole sweep: recover the
                    // GPU device and carry on to the remaining voltages + the
                    // long validation, so we still deliver a profile.
                    warn!("real sweep: device lost at {v}mV +{offset}MHz — recovering");
                    min_unstable_real =
                        Some(min_unstable_real.map_or(peak, |u| u.min(peak)));
                    let _ = gpu::set_core_offset_mhz(0);
                    let _ = gpu::unlock_core_voltage();
                    match recover_gpu_ctx() {
                        Some(fresh) => {
                            ctx = fresh;
                            break; // move on to the next voltage
                        }
                        None => {
                            warn!("real sweep: device unrecoverable after retries — stopping");
                            crashed = true;
                            break 'voltages;
                        }
                    }
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
    prog.tradeoffs = points.clone();
    prog.current = None;
    set_progress(&progress, prog.clone());

    // Phase E — arduous validation: re-apply the top candidate (backed off by
    // margin) and soak it hard until we trust it (or it fails → back off more).
    if !crashed && !stop.load(Ordering::SeqCst) {
        if let Some((v0, off0)) = top_candidate {
            prog.phase = SweepPhase::Synthesis;
            prog.validation_note = Some("Validação longa em andamento…".into());
            set_progress(&progress, prog.clone());

            let mut val_off = (off0 - q.margin_mhz as i32).max(0);
            let mut note = "Falhou na validação longa".to_string();
            for attempt in 0..2 {
                let intent = TuningPoint::from_axes([
                    ("gpu_voltage_mv", v0 as i64),
                    ("gpu_offset_mhz", val_off as i64),
                ]);
                let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_real_validate"));
                let _ = gpu::lock_core_voltage_mv(v0);
                if gpu::set_core_offset_mhz(val_off).is_err() {
                    break;
                }
                let (res, peak, temp) =
                    measure_during(|| match catch_unwind(AssertUnwindSafe(|| arduous_validate(&ctx))) {
                        Ok(r) => r,
                        Err(_) => StabilityResult::Crash,
                    });
                prog.measured_mhz = Some(peak);
                prog.gpu_temp_c = temp;
                prog.last_result = Some(res);
                set_progress(&progress, prog.clone());
                if res.is_stable() {
                    let _ = store.clear_boot_flag();
                    note = format!(
                        "Validado (Silver): {peak} MHz @ {v0} mV estável no soak longo — confirme em jogo",
                    );
                    break;
                } else if matches!(res, StabilityResult::Crash) {
                    crashed = true;
                    note = "Travou na validação longa — recue mais".into();
                    break;
                } else {
                    note = format!("Erro silencioso no soak — recuando (tentativa {})", attempt + 1);
                    val_off = (val_off - q.step_mhz * 2).max(0);
                }
            }
            prog.validation_note = Some(note);
        }
    }

    let _ = gpu::reset_all();
    let _ = store.clear_boot_flag();
    prog.current = None;
    prog.phase = if crashed { SweepPhase::Aborted } else { SweepPhase::Done };
    prog.freq_index = prog.total_freqs;
    set_progress(&progress, prog);
    info!("Real GPU sweep finished (crashed={crashed})");
}

/// Long, hard confirmation soak for the chosen profile (Phase E) — combined
/// core+memory load, like sustained gaming.
#[cfg(windows)]
fn arduous_validate(ctx: &nidavellir_gpu_stress::GpuCtx) -> StabilityResult {
    ctx.run_combined(60_000).result
}
