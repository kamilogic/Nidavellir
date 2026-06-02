//! Power-target sweep — the core of power-limited tuning.
//!
//! On a power-capped card (e.g. 200 W) the binding limit under load is POWER,
//! not stability: the card power-throttles V & clock to fit the cap. Undervolting
//! makes a given clock cost less power (P ≈ C·f·V²), reclaiming that headroom so
//! the card sustains a higher clock within the budget.
//!
//! For a range of locked voltages we measure, under a heavy ALU load, the clock
//! the card naturally runs and the **sustained power it draws** — at each card's
//! own stock V/F point (offset 0), which are inherently stable. That maps the
//! real (per-chip) clock↔power↔voltage relationship — far more accurate than any
//! `mV→W` formula — and lets us pick the **perf/watt knee**: the most efficient
//! operating voltage, which on a power-limited card is the sweet spot the user
//! otherwise hunts for by hand. The performance win comes from *locking* that
//! efficient voltage (+ clock cap), NOT from overclocking — so this sweep never
//! pushes the clock past stock and won't hard-lock the machine.
//!
//! Windows-only (NVAPI/NVML).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::StabilityResult;
use nidavellir_core::ipc::{PowerSweepPoint, PowerSweepProgress};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

const DWELL_MS: u64 = 5000;

fn idle() -> PowerSweepProgress {
    PowerSweepProgress { phase: "idle".into(), ..Default::default() }
}

#[derive(Clone)]
pub struct PowerSweepHandle {
    progress: Arc<Mutex<PowerSweepProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for PowerSweepHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl PowerSweepHandle {
    pub fn progress(&self) -> PowerSweepProgress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_else(|_| idle())
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
            run_power_sweep(progress, stop, store);
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
fn set(progress: &Arc<Mutex<PowerSweepProgress>>, p: PowerSweepProgress) {
    if let Ok(mut g) = progress.lock() {
        *g = p;
    }
}

#[cfg(windows)]
fn recover_ctx() -> Option<nidavellir_gpu_stress::GpuCtx> {
    use nidavellir_gpu_stress::GpuCtx;
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if let Ok(c) = GpuCtx::new() {
            return Some(c);
        }
    }
    None
}

/// Precise per-dwell measurement under the max-power load. Returns the stability
/// verdict and steady-state stats: (mean_clock, mean_power, max_power, std_power,
/// power_capped_fraction). The first ~1.5 s of samples are discarded so only the
/// thermally/clock-settled steady state is measured (precision for the knee +
/// Brokkr's headroom calc).
#[cfg(windows)]
struct Measured {
    result: StabilityResult,
    clock_mhz: u32,
    power_w: f32,
    max_power_w: f32,
    power_std_w: f32,
    capped_frac: f32,
}

#[cfg(windows)]
fn load_and_measure(ctx: &nidavellir_gpu_stress::GpuCtx, ms: u64) -> Measured {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let stop = Arc::new(AtomicBool::new(false));
    // Collect raw samples in the sampler thread for precise stats (mean/max/std).
    let samples: Arc<Mutex<Vec<(u32, f32, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let (s2, smp) = (stop.clone(), samples.clone());
    let t0 = std::time::Instant::now();
    let sampler = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let (Some(c), Some(p)) = (r.core_clock_mhz, r.power_w) {
                    // Discard the first 1.5 s (ramp-up) — steady state only.
                    if t0.elapsed().as_millis() >= 1500 {
                        if let Ok(mut v) = smp.lock() {
                            v.push((c, p, r.power_capped == Some(true)));
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
    });
    let res = match catch_unwind(AssertUnwindSafe(|| ctx.run_power_load(1_000_000, 10_000, ms))) {
        Ok(r) => r.result,
        Err(_) => StabilityResult::Crash,
    };
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();

    let v = samples.lock().map(|g| g.clone()).unwrap_or_default();
    if v.is_empty() {
        return Measured { result: res, clock_mhz: 0, power_w: 0.0, max_power_w: 0.0, power_std_w: 0.0, capped_frac: 0.0 };
    }
    let n = v.len() as f32;
    let clock = (v.iter().map(|s| s.0 as u64).sum::<u64>() / v.len() as u64) as u32;
    let mean_p = v.iter().map(|s| s.1).sum::<f32>() / n;
    let max_p = v.iter().map(|s| s.1).fold(0.0f32, f32::max);
    let var = v.iter().map(|s| (s.1 - mean_p).powi(2)).sum::<f32>() / n;
    let std_p = var.sqrt();
    let capped = v.iter().filter(|s| s.2).count() as f32 / n;
    Measured {
        result: res,
        clock_mhz: clock,
        power_w: mean_p,
        max_power_w: max_p,
        power_std_w: std_p,
        capped_frac: capped,
    }
}

/// Find the perf/watt knee: the point of the (power, clock) curve farthest above
/// the line joining its endpoints — the elbow where more power stops buying much
/// more clock. Falls back to the best raw perf/watt for tiny sets.
#[cfg(windows)]
#[allow(dead_code)]
fn knee(points: &[PowerSweepPoint]) -> Option<PowerSweepPoint> {
    let pts: Vec<&PowerSweepPoint> = points.iter().filter(|p| p.stable && p.power_w > 0.0).collect();
    if pts.is_empty() {
        return None;
    }
    if pts.len() < 3 {
        return pts
            .iter()
            .max_by(|a, b| a.perf_per_watt.partial_cmp(&b.perf_per_watt).unwrap())
            .map(|p| **p);
    }
    let mut sorted = pts.clone();
    sorted.sort_by(|a, b| a.power_w.partial_cmp(&b.power_w).unwrap());
    let (x0, y0) = (sorted[0].power_w as f64, sorted[0].clock_mhz as f64);
    let (x1, y1) = (
        sorted[sorted.len() - 1].power_w as f64,
        sorted[sorted.len() - 1].clock_mhz as f64,
    );
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let mut best = *sorted[0];
    let mut best_d = -1.0f64;
    for p in &sorted {
        // Perpendicular distance, signed so only points ABOVE the line (the
        // concave elbow) count.
        let d = ((p.clock_mhz as f64 - y0) * dx - (p.power_w as f64 - x0) * dy) / len;
        if d > best_d {
            best_d = d;
            best = **p;
        }
    }
    Some(best)
}

#[cfg(windows)]
fn run_power_sweep(
    progress: Arc<Mutex<PowerSweepProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
) {
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;

    info!("Power sweep starting (voltage → max-stable-clock → power)");
    let mut prog = idle();
    prog.running = true;
    prog.phase = "power".into();
    prog.power_limit_w = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
        .and_then(|r| r.power_limit_w)
        .unwrap_or(0.0);
    prog.log.push(format!(
        "Power sweep — cap {:.0} W. Mapeando tensão → clock estável → potência…",
        prog.power_limit_w
    ));
    set(&progress, prog.clone());

    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = gpu::reset_all();

    let mut ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            prog.running = false;
            prog.phase = "done".into();
            prog.note = Some(format!("Falha ao iniciar GPU: {e}"));
            set(&progress, prog);
            return;
        }
    };

    // Voltage range from the card's REAL V/F curve (adapts to any GPU). We never
    // undervolt more than ~250 mV below the top, to bound how far past the cliff
    // a step can go.
    let (vmax, vfloor) = match gpu::read_curve() {
        Ok(c) if c.points.len() >= 2 => {
            let vx = c.points.iter().map(|p| p.voltage_mv).max().unwrap_or(1050);
            let vn = c.points.iter().map(|p| p.voltage_mv).min().unwrap_or(800);
            (vx, vn.max(vx.saturating_sub(250)))
        }
        _ => (1050u32, 850u32),
    };

    let cap = prog.power_limit_w;

    // Stock baseline (unlocked) under the max load → the clock we KEEP (target),
    // and the calibration of how much of the cap the load saturates.
    let _ = gpu::set_core_offset_mhz(0);
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let sm = load_and_measure(&ctx, DWELL_MS);
    prog.stock_clock_mhz = sm.clock_mhz;
    let target = sm.clock_mhz;
    let sat_pct = if cap > 0.0 { sm.max_power_w / cap * 100.0 } else { 0.0 };
    prog.log.push(format!(
        "Stock → {} MHz · {:.0} W (pico {:.0}, {:.0}% do cap){}",
        sm.clock_mhz, sm.power_w, sm.max_power_w, sat_pct,
        if sm.capped_frac > 0.1 { " · power-cap ✓" } else { "" }
    ));
    if target == 0 {
        let _ = gpu::reset_all();
        prog.running = false;
        prog.phase = "done".into();
        prog.note = Some("Não foi possível ler o clock do stock.".into());
        set(&progress, prog);
        return;
    }
    prog.log.push(format!(
        "Alvo: manter {target} MHz e baixar a tensão de {vmax} mV ↓ até a mínima estável."
    ));
    set(&progress, prog.clone());

    // REAL undervolt: pin the clock at the stock target and descend the voltage
    // in small steps from a safe high one, measuring the (now realistic) power at
    // each voltage. Stop at the first instability — that's the cliff; the last
    // stable voltage is the undervolt limit. Descending from a known-stable high
    // voltage means we approach the cliff gently (vs the old clock-ascent that
    // hard-crashed the PC).
    const STEP: u32 = 20;
    let mut v = vmax;
    while v >= vfloor && !stop.load(Ordering::SeqCst) {
        if nidavellir_core::nvml_gpu::pin_core_clock_mhz(target).is_err() {
            prog.log.push("Falha ao fixar o clock (NVML) — abortando.".into());
            break;
        }
        if gpu::lock_core_voltage_mv(v).is_err() {
            warn!("power sweep: lock {v}mV failed; skipping");
            v = v.saturating_sub(STEP);
            continue;
        }
        let intent =
            TuningPoint::from_axes([("gpu_voltage_mv", v as i64), ("gpu_clock_mhz", target as i64)]);
        let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_power_sweep"));
        let m = load_and_measure(&ctx, DWELL_MS);
        let _ = store.clear_boot_flag();
        prog.log.push(format!(
            "{v} mV @ {target} MHz → {} MHz · {:.0} W (máx {:.0}{}) : {}",
            m.clock_mhz, m.power_w, m.max_power_w,
            if m.capped_frac > 0.02 { format!(", cap {:.0}%", m.capped_frac * 100.0) } else { String::new() },
            match m.result {
                StabilityResult::Stable => "ok",
                StabilityResult::SilentError => "erro silencioso (limite)",
                StabilityResult::Crash => "instável",
            }
        ));
        match m.result {
            StabilityResult::Stable if m.clock_mhz > 0 && m.power_w > 0.0 => {
                prog.points.push(PowerSweepPoint {
                    voltage_mv: v,
                    clock_mhz: m.clock_mhz,
                    power_w: m.power_w,
                    max_power_w: m.max_power_w,
                    power_std_w: m.power_std_w,
                    power_capped_frac: m.capped_frac,
                    stable: true,
                    perf_per_watt: m.clock_mhz as f64 / m.power_w as f64,
                });
                set(&progress, prog.clone());
                v = v.saturating_sub(STEP);
            }
            StabilityResult::Stable => {
                v = v.saturating_sub(STEP);
            }
            StabilityResult::SilentError => {
                prog.log.push(format!("Limite de undervolt em ~{v} mV — parando a descida."));
                break;
            }
            StabilityResult::Crash => {
                let _ = gpu::unlock_core_voltage();
                let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
                if recover_ctx().map(|c| ctx = c).is_none() {
                    prog.note = Some("GPU não recuperou — parando".into());
                }
                break;
            }
        }
    }

    let _ = gpu::unlock_core_voltage();
    let _ = gpu::reset_all();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = store.clear_boot_flag();

    // --- Synthesize the three profiles (all hold the stock target clock; they
    // differ in voltage → power. No core OC here, so performance = stock; the
    // profiles trade power/stability-margin). -----------------------------
    use std::cmp::Ordering as Ord;
    let max_std = prog.points.iter().map(|p| p.power_std_w).fold(0.0f32, f32::max);
    let headroom = (0.10 * cap).max(2.0 * max_std);
    let brokkr_target = if cap > 0.0 { cap - headroom } else { f32::MAX };
    prog.target_w = brokkr_target;

    // Godforge: highest voltage held → most power / most stability margin (stock perf).
    prog.godforge = prog.points.iter().copied().max_by_key(|p| p.voltage_mv);
    // Brokkr's Best: highest voltage whose sustained power stays under the target
    // (stock perf, comfortably off the cap). Falls back to the most efficient.
    prog.brokkrs = prog
        .points
        .iter()
        .filter(|p| p.power_w <= brokkr_target)
        .copied()
        .max_by_key(|p| p.voltage_mv)
        .or_else(|| prog.points.iter().copied().min_by_key(|p| p.voltage_mv));
    // Deep Calm: best perf/watt = lowest stable voltage = least power (same perf).
    prog.deep_calm = prog
        .points
        .iter()
        .copied()
        .max_by(|a, b| a.perf_per_watt.partial_cmp(&b.perf_per_watt).unwrap_or(Ord::Equal));
    prog.recommended = prog.deep_calm;

    if prog.godforge.is_some() {
        let fmt = |o: Option<PowerSweepPoint>| match o {
            Some(p) => format!("{} MHz @ {} mV ({:.0} W)", p.clock_mhz, p.voltage_mv, p.power_w),
            None => "—".into(),
        };
        let dc_eff = match prog.deep_calm {
            Some(p) if sm.power_w > 0.0 => {
                format!(" (Deep Calm mantém o clock economizando {:.0} W)", (sm.power_w - p.power_w).max(0.0))
            }
            _ => String::new(),
        };
        prog.note = Some(format!(
            "Mantendo {target} MHz · cap {cap:.0} W (alvo Brokkr's ≤ {brokkr_target:.0} W) · Godforge {} · Brokkr's {} · Deep Calm {}{dc_eff} — confirme em jogo.",
            fmt(prog.godforge), fmt(prog.brokkrs), fmt(prog.deep_calm)
        ));
    } else {
        prog.note = Some("Nenhum ponto de undervolt estável encontrado.".into());
    }
    prog.running = false;
    prog.phase = "done".into();
    set(&progress, prog);
    info!("Power sweep finished");
}
