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
use tracing::info;

// Long enough for power to RAMP UP and stabilize (real loads like Heaven take
// seconds to reach their sustained draw — it's a ramp, not a spike). We discard
// the ramp and take the WORST CASE (max), not the mean.
const DWELL_MS: u64 = 15000;
const RAMP_DISCARD_MS: u128 = 6000;
/// One exploration step (MHz of curve-flatten offset). ~9 mV/step here.
const EXPLORE_STEP: i32 = 15;
/// Stay this far (MHz offset) BELOW a learned hard-crash offset when exploring, so
/// we never re-probe the cliff. The +255 reboot taught us the crash region; with
/// this buffer the descent stops near a hand-tuned ~875 mV, well above it.
const CRASH_BUFFER: i32 = 60;
/// Exploration ceiling when no crash has ever been learned (first run, no memory).
/// Conservative: ~900 mV, the last value validated stable before the reboot.
const DEFAULT_CEILING: i32 = 150;
/// Hard cap regardless of memory — we never flatten the clock more than this.
const ABS_MAX_OFFSET: i32 = 240;

/// Persistent "stability frontier" memory (user's design): turn instability into
/// DATA. We remember the deepest offset that ran stable and the first offset that
/// crashed, so each run knows roughly where the cliff is and explores BELOW it
/// instead of rediscovering it with another reboot.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct BoundaryMemory {
    /// Deepest curve-flatten offset (MHz) observed STABLE.
    last_stable_offset: i32,
    /// Shallowest offset (MHz) that produced a crash/TDR/reboot, if any.
    first_crash_offset: Option<i32>,
}

#[cfg(windows)]
fn boundary_path() -> std::path::PathBuf {
    nidavellir_core::safe_loop::default_data_dir().join("gpu_boundary.json")
}

#[cfg(windows)]
fn load_boundary() -> BoundaryMemory {
    std::fs::read_to_string(boundary_path())
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
fn save_boundary(b: &BoundaryMemory) {
    let _ = std::fs::create_dir_all(nidavellir_core::safe_loop::default_data_dir());
    if let Ok(j) = serde_json::to_string_pretty(b) {
        let _ = std::fs::write(boundary_path(), j);
    }
}

/// How many steps PAST the deepest known-stable offset a single run may probe.
/// Incremental: each run extends the frontier only a little, so we creep toward the
/// efficiency optimum across runs instead of leaping at the cliff.
const PROBE_STEPS: i32 = 2;

/// Exploration ceiling (MHz offset). Probe only a couple of steps past the deepest
/// known-stable offset, NEVER within `CRASH_BUFFER` of a known crash, never past
/// `ABS_MAX_OFFSET`, and at least `DEFAULT_CEILING` (so a fresh memory still maps
/// the safe region). This is the user's "progressive exploration + permanent
/// learning": instability seen in a run lowers `first_crash` and pulls the ceiling
/// back next time.
#[cfg(windows)]
fn explore_ceiling(b: &BoundaryMemory) -> i32 {
    let probe = b.last_stable_offset + PROBE_STEPS * EXPLORE_STEP;
    let crash_cap = b
        .first_crash_offset
        .map(|c| c - CRASH_BUFFER)
        .unwrap_or(ABS_MAX_OFFSET);
    probe.min(crash_cap).min(ABS_MAX_OFFSET).max(DEFAULT_CEILING)
}

/// Stock-curve clock (MHz) at or below `v` mV — the card's natural clock there.
#[cfg(windows)]
#[allow(dead_code)]
fn curve_freq_at_v(pts: &[(u32, u32)], v: u32) -> u32 {
    pts.iter()
        .filter(|p| p.0 <= v)
        .max_by_key(|p| p.0)
        .or_else(|| pts.iter().min_by_key(|p| p.0))
        .map(|p| p.1)
        .unwrap_or(0)
}

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

/// 3-tier failure classification (user's spec) — distinguishes "the GPU computed
/// the wrong answer but the driver is fine" from "the workload context was lost but
/// recoverable" from "the driver could not re-initialize the device (hard TDR)".
/// Each tier recedes a different number of sweep steps.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailTier {
    /// L1: instability detected, driver healthy (silent error / wrong result).
    L1Instability,
    /// L2: workload/context lost, but the device was recreated successfully.
    L2WorkloadReset,
    /// L3: hard TDR — the driver could not re-initialize the device.
    L3HardTdr,
}

#[cfg(windows)]
impl FailTier {
    fn label(self) -> &'static str {
        match self {
            FailTier::L1Instability => "L1 FAIL · instabilidade (driver ok)",
            FailTier::L2WorkloadReset => "L2 FAIL · Workload-Reset (device recriado)",
            FailTier::L3HardTdr => "L3 FAIL · HARD TDR (driver não reinicializou)",
        }
    }
    /// How many sweep steps to recede (toward higher voltage / safety) for this tier.
    fn backoff_steps(self) -> u32 {
        match self {
            FailTier::L1Instability => 1,
            FailTier::L2WorkloadReset => 2,
            FailTier::L3HardTdr => 4,
        }
    }
}

/// Classify a non-stable result into the 3-tier model, recovering the device on a
/// context loss. On a Crash the offset/clock are first reset to stock, then the
/// GpuCtx is recreated; if recreation fails it's a hard TDR (L3) and `*ctx` is left
/// as-is (the caller must abort — there is no working device to run more loads on).
#[cfg(windows)]
fn classify_failure(
    res: StabilityResult,
    ctx: &mut nidavellir_gpu_stress::GpuCtx,
) -> FailTier {
    match res {
        StabilityResult::SilentError => FailTier::L1Instability,
        StabilityResult::Stable => FailTier::L1Instability, // not expected; treat as mild
        StabilityResult::Crash => {
            let _ = nidavellir_gpu_nvapi::set_core_offset_mhz(0);
            let _ = nidavellir_gpu_nvapi::reset_vf_curve();
            let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
            match recover_ctx() {
                Some(c) => {
                    *ctx = c;
                    FailTier::L2WorkloadReset
                }
                None => FailTier::L3HardTdr,
            }
        }
    }
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
    /// Max core voltage observed under load (mV) — for the descent's safe start.
    volt_mv: u32,
}

#[cfg(windows)]
fn load_and_measure(ms: u64) -> Measured {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::AtomicU32;
    // FRESH wgpu context per measurement. The FurMark-class render reliably runs
    // ONCE on a fresh device but TDRs (device lost, unrecoverable) if a SECOND
    // heavy render is issued on the SAME GpuCtx — so we create + drop a context per
    // dwell. The clock offset is applied via NVAPI on the hardware, independent of
    // the wgpu device, so a fresh context still measures the applied operating point.
    let ctx = match nidavellir_gpu_stress::GpuCtx::new() {
        Ok(c) => c,
        Err(_) => {
            return Measured {
                result: StabilityResult::Crash,
                clock_mhz: 0, power_w: 0.0, max_power_w: 0.0, power_std_w: 0.0,
                capped_frac: 0.0, volt_mv: 0,
            }
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    // Collect raw samples in the sampler thread for precise stats (mean/max/std).
    let samples: Arc<Mutex<Vec<(u32, f32, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let volt = Arc::new(AtomicU32::new(0));
    let (s2, smp, vlt) = (stop.clone(), samples.clone(), volt.clone());
    let t0 = std::time::Instant::now();
    let sampler = std::thread::spawn(move || {
        let mut tick: u32 = 0;
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let (Some(c), Some(p)) = (r.core_clock_mhz, r.power_w) {
                    // Discard the ramp-up — steady state only.
                    if t0.elapsed().as_millis() >= RAMP_DISCARD_MS {
                        if let Ok(mut v) = smp.lock() {
                            v.push((c, p, r.power_capped == Some(true)));
                        }
                    }
                }
            }
            // Voltage via NVAPI is heavier (re-inits), so sample it sparsely.
            tick += 1;
            if tick % 16 == 0 {
                if let Some(mv) = nidavellir_gpu_nvapi::read_core_voltage_mv() {
                    vlt.fetch_max(mv, Ordering::SeqCst);
                }
            }
            // Fast sampling to catch short power spikes the cap reacts to (NVML
            // at ~80ms missed the peaks that still hit the 200W cap in Heaven).
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    });
    // FurMark-class TEXTURED RENDER for the dwell — it exercises the full graphics
    // pipeline (texturing, heavy overdraw, FP fragment) so it draws TRUE GAME POWER
    // (~199 W on a 3060 Ti, like Overwatch) and SATURATES THE POWER CAP, which a
    // pure-ALU compute kernel never does (~159 W, never cap-limited → wrong regime).
    // It still detects silent errors (per-frame reduction checksum) and crashes.
    // Safe here: the measurement loop applies only a clock OFFSET (no rigid clock
    // pin / voltage lock), so the card stays power-managed and throttles to fit the
    // cap instead of TDRing — measuring the real power-limited regime the undervolt
    // actually helps in.
    let res = match catch_unwind(AssertUnwindSafe(|| ctx.run_render_stress(ms))) {
        Ok(r) => r.result,
        Err(_) => StabilityResult::Crash,
    };
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();

    let volt_mv = volt.load(Ordering::SeqCst);
    let v = samples.lock().map(|g| g.clone()).unwrap_or_default();
    if v.is_empty() {
        return Measured { result: res, clock_mhz: 0, power_w: 0.0, max_power_w: 0.0, power_std_w: 0.0, capped_frac: 0.0, volt_mv };
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
        volt_mv,
    }
}

/// Find the perf/watt knee: the point of the (power, clock) curve farthest above
/// the line joining its endpoints — the elbow where more power stops buying much
/// more clock. Falls back to the best raw perf/watt for tiny sets.
/// Arduous re-validation of a chosen undervolt point: cap the clock at target +
/// apply the point's offset (NO voltage lock) and run a LONG max-power soak. The
/// quick dwell only approximates the cliff; a marginal undervolt can pass it yet
/// fail in a game. If the soak fails, step to a LOWER offset (less undervolt =
/// higher voltage = safer) and retry until stable — so the profile is solid.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn arduous_validate(
    ctx: &mut nidavellir_gpu_stress::GpuCtx,
    store: &SafeLoopStore,
    target: u32,
    start: PowerSweepPoint,
    points: &[PowerSweepPoint],
    stop: &Arc<AtomicBool>,
    label: &str,
    progress: &Arc<Mutex<PowerSweepProgress>>,
    prog: &mut PowerSweepProgress,
) -> Option<PowerSweepPoint> {
    let mut cand = start;
    for _ in 0..6 {
        if stop.load(Ordering::SeqCst) {
            return Some(cand);
        }
        prog.log.push(format!(
            "Validação árdua {label}: +{} MHz (~{} mV @ {target} MHz, ~35s, carga de jogo)…",
            cand.offset_mhz, cand.voltage_mv
        ));
        set(progress, prog.clone());
        // Soak under the SAME game-power render the sweep measured with (no rigid
        // NVML clock pin — the cap + curve limit the clock naturally, exactly like
        // the measurement). A marginal undervolt that passed the short dwell can
        // still fail this long soak; if so, back off to a higher voltage.
        let _ = nidavellir_gpu_nvapi::set_core_offset_mhz(cand.offset_mhz);
        let _ = store.arm_boot_flag(&BootFlag::new(
            TuningPoint::from_axes([
                ("gpu_offset_mhz", cand.offset_mhz as i64),
                ("gpu_clock_mhz", target as i64),
            ]),
            "gpu_power_validate",
        ));
        let res = load_and_measure(35_000).result;
        let _ = store.clear_boot_flag();
        if matches!(res, StabilityResult::Stable) {
            prog.log.push(format!("✓ {label} validado: +{} MHz (~{} mV)", cand.offset_mhz, cand.voltage_mv));
            set(progress, prog.clone());
            return Some(cand);
        }
        let tier = classify_failure(res, ctx);
        prog.log.push(format!("✗ {label}: {} em +{} MHz", tier.label(), cand.offset_mhz));
        set(progress, prog.clone());
        if tier == FailTier::L3HardTdr {
            prog.log.push(format!("Abortando {label}: device não recuperou (hard TDR)."));
            set(progress, prog.clone());
            return None;
        }
        // Recede this tier's number of steps toward a LOWER offset (less undervolt
        // → higher voltage → safer): L1 backs off 1, L2 backs off 2.
        for _ in 0..tier.backoff_steps() {
            match points.iter().filter(|p| p.offset_mhz < cand.offset_mhz).max_by_key(|p| p.offset_mhz).copied() {
                Some(n) => cand = n,
                None => {
                    prog.log.push(format!("Sem ponto mais conservador para {label}."));
                    return None;
                }
            }
        }
    }
    Some(cand)
}

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

    let cap = prog.power_limit_w;

    // Stock baseline (unlocked) under the max load → the clock we KEEP (target),
    // and the calibration of how much of the cap the load saturates.
    let _ = gpu::set_core_offset_mhz(0);
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let sm = load_and_measure(DWELL_MS);
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

    // FLATTEN-based undervolt — NO hard voltage lock. Hard-locking the voltage
    // (set_vfp_locks) under a game-realistic ≈cap load TDRs: the card can't manage
    // power. Instead we cap the clock at the stock target and RAISE the offset;
    // more offset makes the card reach the target at a LOWER voltage (it picks the
    // voltage itself, keeping power management), drawing less power. Voltage is the
    // measured OUTPUT. Sweep offset up until instability = the undervolt limit.
    prog.log.push(format!(
        "Mantendo {target} MHz; subindo o offset (flatten, sem travar tensão) até o limite estável."
    ));
    set(&progress, prog.clone());
    let _ = nidavellir_core::nvml_gpu::lock_core_clock_max_mhz(target);

    // Learned stability frontier (user's design): explore only up to a safe ceiling
    // BELOW the known crash — turn instability into DATA instead of rediscovering
    // the cliff with another reboot. The frontier delimits the SEARCH; the result
    // is the best-efficiency point, chosen later with a safety margin.
    let mut boundary = load_boundary();
    let ceiling = explore_ceiling(&boundary);
    prog.log.push(format!(
        "Fronteira aprendida: estável ≤ +{} MHz · crash @ {} · explorando até +{} MHz (margem {} do penhasco).",
        boundary.last_stable_offset,
        boundary.first_crash_offset.map(|c| format!("+{c}")).unwrap_or_else(|| "?".into()),
        ceiling, CRASH_BUFFER
    ));
    set(&progress, prog.clone());

    let mut offset = 0i32;
    while offset <= ceiling && !stop.load(Ordering::SeqCst) {
        let _ = gpu::set_core_offset_mhz(offset);
        let intent =
            TuningPoint::from_axes([("gpu_offset_mhz", offset as i64), ("gpu_clock_mhz", target as i64)]);
        let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_power_sweep"));
        let m = load_and_measure(DWELL_MS);
        let _ = store.clear_boot_flag();
        prog.log.push(format!(
            "+{offset} MHz → {} mV · {} MHz · {:.0} W (máx {:.0}{}) : {}",
            m.volt_mv, m.clock_mhz, m.power_w, m.max_power_w,
            if m.capped_frac > 0.02 { format!(", cap {:.0}%", m.capped_frac * 100.0) } else { String::new() },
            match m.result {
                StabilityResult::Stable => "ok",
                StabilityResult::SilentError => "erro silencioso (limite)",
                StabilityResult::Crash => "instável",
            }
        ));
        match m.result {
            StabilityResult::Stable if m.clock_mhz > 0 && m.power_w > 0.0 && m.volt_mv > 0 => {
                prog.points.push(PowerSweepPoint {
                    voltage_mv: m.volt_mv,
                    clock_mhz: m.clock_mhz,
                    offset_mhz: offset,
                    power_w: m.power_w,
                    max_power_w: m.max_power_w,
                    power_std_w: m.power_std_w,
                    power_capped_frac: m.capped_frac,
                    stable: true,
                    perf_per_watt: m.clock_mhz as f64 / m.power_w as f64,
                });
                // Learn: this offset ran stable — remember it for future runs.
                if offset > boundary.last_stable_offset {
                    boundary.last_stable_offset = offset;
                    save_boundary(&boundary);
                }
                set(&progress, prog.clone());
                offset += EXPLORE_STEP;
            }
            StabilityResult::Stable => {
                offset += EXPLORE_STEP;
            }
            StabilityResult::SilentError => {
                // First instability = the frontier. Record it as data; the best
                // EFFICIENCY (not this edge) is the result, chosen below with margin.
                boundary.first_crash_offset =
                    Some(boundary.first_crash_offset.map_or(offset, |c| c.min(offset)));
                save_boundary(&boundary);
                prog.log.push(format!(
                    "Fronteira de instabilidade em +{offset} MHz — {} — registrado, parando.",
                    FailTier::L1Instability.label()
                ));
                break;
            }
            StabilityResult::Crash => {
                let _ = gpu::set_core_offset_mhz(0);
                let tier = classify_failure(StabilityResult::Crash, &mut ctx);
                boundary.first_crash_offset =
                    Some(boundary.first_crash_offset.map_or(offset, |c| c.min(offset)));
                save_boundary(&boundary);
                prog.log.push(format!(
                    "Fronteira (crash) em +{offset} MHz — {} — registrado, parando.",
                    tier.label()
                ));
                break;
            }
        }
    }

    let _ = gpu::set_core_offset_mhz(0);
    let _ = gpu::reset_all();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = store.clear_boot_flag();

    // --- Synthesize the profiles --------------------------------------------
    // Brokkr's objective (user's redefinition, 2026-06-03): MAXIMIZE efficiency
    // (MHz per Watt) — the perf/watt KNEE — NOT minimize voltage / chase the
    // deepest undervolt. The deepest-undervolt question walks toward the stability
    // cliff (a +255 offset / ~855 mV hard-crashed the PC); the best efficiency sits
    // well before it. We return the BEST EFFICIENCY OBSERVED among stable points,
    // and the offset sweep is bounded (MAX_OFFSET) to stay out of the cliff region.
    use std::cmp::Ordering as Ord;
    let max_std = prog.points.iter().map(|p| p.power_std_w).fold(0.0f32, f32::max);
    let headroom = (0.10 * cap).max(2.0 * max_std);
    let brokkr_target = if cap > 0.0 { cap - headroom } else { f32::MAX };
    prog.target_w = brokkr_target;

    // Godforge = max stable performance (least undervolt / highest voltage; OC-
    // oriented, refined later). Deep Calm removed (converged with Brokkr's).
    prog.godforge = prog.points.iter().copied().max_by_key(|p| p.voltage_mv);
    // Brokkr's Best = best efficiency (MHz/W) among points that ran OFF the cap
    // (power_capped_frac < 5%) — a capped profile dips its clock in-game, the
    // inconsistency we eliminate. Among the off-cap points this is the efficiency
    // knee; fall back to the least-capped point only if none ran off-cap.
    let off_cap: Vec<PowerSweepPoint> = prog
        .points
        .iter()
        .copied()
        .filter(|p| p.power_capped_frac < 0.05)
        .collect();
    prog.brokkrs = if off_cap.is_empty() {
        prog.points
            .iter()
            .copied()
            .min_by(|a, b| a.power_capped_frac.partial_cmp(&b.power_capped_frac).unwrap_or(Ord::Equal))
    } else {
        off_cap
            .iter()
            .copied()
            .max_by(|a, b| a.perf_per_watt.partial_cmp(&b.perf_per_watt).unwrap_or(Ord::Equal))
    };
    prog.deep_calm = None;
    if let Some(b) = prog.brokkrs {
        prog.log.push(format!(
            "Melhor eficiência (MHz/W): {:.2} @ {} mV · {} MHz · {:.0} W (off-cap) — não o menor mV, o melhor perf/watt.",
            b.perf_per_watt, b.voltage_mv, b.clock_mhz, b.power_w
        ));
        set(&progress, prog.clone());
    }

    // --- Arduous validation of each pick (long soak + back-off) -----------
    let pts = prog.points.clone();
    prog.phase = "validate".into();
    set(&progress, prog.clone());
    if let Some(p) = prog.godforge {
        prog.godforge = arduous_validate(&mut ctx, &store, target, p, &pts, &stop, "Godforge", &progress, &mut prog);
    }
    if let Some(p) = prog.brokkrs {
        prog.brokkrs = arduous_validate(&mut ctx, &store, target, p, &pts, &stop, "Brokkr's", &progress, &mut prog);
    }
    prog.recommended = prog.brokkrs;

    let _ = nidavellir_gpu_nvapi::unlock_core_voltage();
    let _ = gpu::reset_all();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = store.clear_boot_flag();

    if prog.godforge.is_some() {
        let fmt = |o: Option<PowerSweepPoint>| match o {
            Some(p) => format!("{} MHz @ {} mV ({:.0} W)", p.clock_mhz, p.voltage_mv, p.power_w),
            None => "—".into(),
        };
        let bk_eff = match prog.brokkrs {
            Some(p) if sm.power_w > 0.0 => format!(
                " ({:.2} MHz/W, −{:.0} W vs stock, off-cap)",
                p.perf_per_watt,
                (sm.power_w - p.power_w).max(0.0)
            ),
            _ => String::new(),
        };
        prog.note = Some(format!(
            "Mantendo {target} MHz · cap {cap:.0} W · Godforge {} · Brokkr's (melhor eficiência) {}{bk_eff} — confirme em jogo.",
            fmt(prog.godforge), fmt(prog.brokkrs)
        ));
    } else {
        prog.note = Some("Nenhum ponto de undervolt estável encontrado.".into());
    }
    prog.running = false;
    prog.phase = "done".into();
    set(&progress, prog);
    info!("Power sweep finished");
}
