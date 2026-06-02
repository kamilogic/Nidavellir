//! Power-target sweep — the core of power-limited tuning.
//!
//! On a power-capped card (e.g. 200 W) the binding limit under load is POWER,
//! not stability: the card power-throttles V & clock to fit the cap. Undervolting
//! makes a given clock cost less power (P ≈ C·f·V²), reclaiming that headroom so
//! the card sustains a higher clock within the budget.
//!
//! For a range of locked voltages we raise the clock to its max stable value and
//! **measure the sustained power it draws** under a heavy ALU load. That maps the
//! real (per-chip) clock↔power↔voltage relationship — far more accurate than any
//! `mV→W` formula — and lets us pick the **perf/watt knee**: the point just before
//! diminishing returns (near-max performance at much lower power), which on a
//! power-limited card is the sweet spot the user otherwise hunts for by hand.
//!
//! Windows-only (NVAPI/NVML).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::StabilityResult;
use nidavellir_core::ipc::{PowerSweepPoint, PowerSweepProgress};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

/// Voltages (mV) to characterize, low → high.
const VOLTAGES: &[u32] = &[825, 862, 900, 937, 975, 1012];
const STEP_MHZ: i32 = 30;
const CAP_MHZ: i32 = 300;
const DWELL_MS: u64 = 5000;

fn idle() -> PowerSweepProgress {
    PowerSweepProgress {
        running: false,
        phase: "idle".into(),
        log: Vec::new(),
        points: Vec::new(),
        power_limit_w: 0.0,
        target_w: 0.0,
        recommended: None,
        note: None,
    }
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

/// Run a heavy ALU load for ~`ms` while sampling NVML; returns the stability
/// verdict, the mean sustained core clock (MHz) and the mean power (W).
#[cfg(windows)]
fn load_and_measure(ctx: &nidavellir_gpu_stress::GpuCtx, ms: u64) -> (StabilityResult, u32, f32) {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let stop = Arc::new(AtomicBool::new(false));
    let clk = Arc::new(AtomicU64::new(0));
    let pw = Arc::new(AtomicU64::new(0));
    let n = Arc::new(AtomicU64::new(0));
    let (s2, c2, p2, n2) = (stop.clone(), clk.clone(), pw.clone(), n.clone());
    let sampler = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let (Some(c), Some(p)) = (r.core_clock_mhz, r.power_w) {
                    c2.fetch_add(c as u64, Ordering::SeqCst);
                    p2.fetch_add((p * 1000.0) as u64, Ordering::SeqCst);
                    n2.fetch_add(1, Ordering::SeqCst);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    });
    // Dense ALU is the most power-hungry load (no memory stalls) — best proxy for
    // the real per-voltage power ceiling.
    let res = match catch_unwind(AssertUnwindSafe(|| ctx.run_alu("power", 2_000_000, 200_000, ms))) {
        Ok(r) => r.result,
        Err(_) => StabilityResult::Crash,
    };
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();
    let cnt = n.load(Ordering::SeqCst).max(1);
    let clock = (clk.load(Ordering::SeqCst) / cnt) as u32;
    let power = (pw.load(Ordering::SeqCst) as f32 / cnt as f32) / 1000.0;
    (res, clock, power)
}

/// Find the perf/watt knee: the point of the (power, clock) curve farthest above
/// the line joining its endpoints — the elbow where more power stops buying much
/// more clock. Falls back to the best raw perf/watt for tiny sets.
#[cfg(windows)]
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

    let near_cap = if prog.power_limit_w > 0.0 { prog.power_limit_w * 0.97 } else { f32::MAX };

    'volts: for &v in VOLTAGES {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if gpu::lock_core_voltage_mv(v).is_err() {
            warn!("power sweep: lock {v}mV failed; skipping");
            continue;
        }
        let mut best: Option<(u32, f32)> = None;
        let mut offset = 0i32;
        loop {
            if stop.load(Ordering::SeqCst) {
                break 'volts;
            }
            let intent = TuningPoint::from_axes([
                ("gpu_voltage_mv", v as i64),
                ("gpu_offset_mhz", offset as i64),
            ]);
            let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_power_sweep"));
            let _ = gpu::set_core_offset_mhz(offset);
            let (res, clock, power) = load_and_measure(&ctx, DWELL_MS);
            prog.log.push(format!(
                "{v} mV +{offset} → {clock} MHz · {power:.0} W : {}",
                match res {
                    StabilityResult::Stable => "ok",
                    StabilityResult::SilentError => "erro silencioso",
                    StabilityResult::Crash => "device-lost",
                }
            ));
            set(&progress, prog.clone());

            match res {
                StabilityResult::Stable => {
                    let _ = store.clear_boot_flag();
                    best = Some((clock, power));
                    if power >= near_cap {
                        prog.log.push(format!("   {v} mV atingiu ~cap ({power:.0} W) — próxima tensão"));
                        break;
                    }
                    offset += STEP_MHZ;
                    if offset > CAP_MHZ {
                        break;
                    }
                }
                StabilityResult::SilentError => break,
                StabilityResult::Crash => {
                    warn!("power sweep: device lost at {v}mV +{offset} — recovering");
                    let _ = gpu::set_core_offset_mhz(0);
                    let _ = gpu::unlock_core_voltage();
                    match recover_ctx() {
                        Some(fresh) => {
                            ctx = fresh;
                            break;
                        }
                        None => {
                            prog.note = Some("GPU não recuperou — parando".into());
                            break 'volts;
                        }
                    }
                }
            }
        }
        if let Some((clock, power)) = best {
            let p = PowerSweepPoint {
                voltage_mv: v,
                clock_mhz: clock,
                power_w: power,
                stable: true,
                perf_per_watt: if power > 0.0 { clock as f64 / power as f64 } else { 0.0 },
            };
            prog.points.push(p);
            set(&progress, prog.clone());
        }
        let _ = gpu::set_core_offset_mhz(0);
    }

    let _ = gpu::unlock_core_voltage();
    let _ = gpu::reset_all();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = store.clear_boot_flag();

    // Recommend the perf/watt knee.
    if let Some(k) = knee(&prog.points) {
        prog.recommended = Some(k);
        prog.target_w = k.power_w;
        prog.note = Some(format!(
            "Recomendado (joelho perf/watt): {} MHz @ {} mV · {:.0} W de {:.0} W cap — confirme em jogo.",
            k.clock_mhz, k.voltage_mv, k.power_w, prog.power_limit_w
        ));
    } else {
        prog.note = Some("Nenhum ponto estável medido.".into());
    }
    prog.running = false;
    prog.phase = "done".into();
    set(&progress, prog);
    info!("Power sweep finished");
}
