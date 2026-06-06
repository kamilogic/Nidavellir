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

// Long enough for power to RAMP UP and stabilize (real loads like Heaven take
// seconds to reach their sustained draw — it's a ramp, not a spike). We discard
// the ramp and take the WORST CASE (max), not the mean.
const DWELL_MS: u64 = 15000;
const RAMP_DISCARD_MS: u128 = 6000;
/// One exploration step (MHz of curve-flatten offset). ~9 mV/step here.
const EXPLORE_STEP: i32 = 15;
/// Exploration ceiling when nothing has been learned yet (fresh per-GPU knowledge).
/// Conservative: ~900 mV, the value validated stable before the +255 reboot.
const DEFAULT_CEILING: i32 = 150;
/// Hard cap regardless of knowledge — we never flatten the clock more than this.
const ABS_MAX_OFFSET: i32 = 240;
/// How many steps PAST the deepest known-clean offset a single run may probe, so we
/// creep toward the optimum across runs instead of leaping at the cliff.
const PROBE_STEPS: i32 = 2;
/// V1 "Conservative" search margin as a FRACTION of the discovered zone width
/// (highest_clean → lowest_failure) — NOT a fixed MHz. A wide zone ⇒ wider margin;
/// a narrow (refined) zone ⇒ tight margin. Adapts to each GPU's curve. (V2 replaces
/// this with a Wilson-lower-bound confidence gate; the per-point stats below feed it.)
const CONSERVATIVE_MARGIN_FRAC: f64 = 0.30;

/// Brokkr's V2 selection profiles. The threshold is the minimum stability
/// confidence (Wilson lower bound over a point's accumulated trials) a candidate
/// must clear to be eligible. Higher = more evidence required = safer.
#[cfg(windows)]
#[allow(dead_code)] // Conservative/Aggressive are selectable; only Balanced is wired for now
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SweepProfile {
    Conservative,
    Balanced,
    Aggressive,
}

#[cfg(windows)]
impl SweepProfile {
    /// Minimum Wilson lower-bound stability confidence to accept a candidate.
    fn threshold(self) -> f64 {
        match self {
            SweepProfile::Conservative => 0.95,
            SweepProfile::Balanced => 0.85,
            SweepProfile::Aggressive => 0.70,
        }
    }
}

/// Active selection profile. Hard-coded for now (the sweep IPC is param-free);
/// exposing it per-request via IPC/UI is a follow-up.
#[cfg(windows)]
const ACTIVE_PROFILE: SweepProfile = SweepProfile::Balanced;

/// Instability severity, ordered (mirrors the L1/L2/L3 fail tiers). Stored per point
/// and per frontier so the algorithm can weigh a cheap SilentError differently from
/// an expensive HardReboot.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
enum FailSeverity {
    #[default]
    None,
    SilentError,
    Tdr,
    Reboot,
}

/// Accumulated statistics for ONE offset, summed across ALL runs (continuous
/// learning). The stable aggregates give a mean score; trials/failures feed the
/// (future) Wilson confidence — a point with 50 clean trials must outrank one with 1.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PointStat {
    trials: u32,
    failures: u32,
    worst_severity: FailSeverity,
    stable_trials: u32,
    clock_mhz_sum: u64,
    power_w_sum: f64,
    voltage_mv_sum: u64,
}

#[allow(dead_code)] // mean_voltage_mv() reserved for V3; score()/confidence() drive V2 selection
impl PointStat {
    /// Mean efficiency (MHz/W) over the stable trials, or 0 if never stable.
    fn score(&self) -> f64 {
        if self.stable_trials == 0 || self.power_w_sum <= 0.0 {
            0.0
        } else {
            self.clock_mhz_sum as f64 / self.power_w_sum
        }
    }
    /// Stability confidence: Wilson lower bound (z=1.96, 95%) of the stable rate
    /// over accumulated trials. Few trials ⇒ low confidence even at a 100% rate
    /// (1/1 ≈ 0.21, 50/50 ≈ 0.93), so a barely-tested point can't win on luck.
    fn confidence(&self) -> f64 {
        wilson_lower_bound(self.stable_trials, self.trials, 1.96)
    }
    fn mean_voltage_mv(&self) -> u32 {
        if self.stable_trials == 0 { 0 } else { (self.voltage_mv_sum / self.stable_trials as u64) as u32 }
    }
}

/// Wilson score-interval lower bound for a binomial success rate — a sample-size-
/// aware confidence floor: `successes`/`trials` stable observations, `z` the normal
/// quantile (1.96 ≈ 95%). Returns 0 when there are no trials.
#[allow(dead_code)] // live on Windows (drives selection); kept for non-Windows builds
fn wilson_lower_bound(successes: u32, trials: u32, z: f64) -> f64 {
    if trials == 0 {
        return 0.0;
    }
    let n = trials as f64;
    let phat = successes as f64 / n;
    let z2 = z * z;
    let center = phat + z2 / (2.0 * n);
    let margin = z * ((phat * (1.0 - phat) / n) + z2 / (4.0 * n * n)).sqrt();
    let denom = 1.0 + z2 / n;
    ((center - margin) / denom).max(0.0)
}

/// Severity-differentiated stability frontier (user's design): keep the shallowest
/// offset of each failure class separately, never collapsing them to one number.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct BoundaryKnowledge {
    /// Deepest offset (MHz) observed fully stable.
    highest_clean: i32,
    lowest_silent_error: Option<i32>,
    lowest_tdr: Option<i32>,
    lowest_reboot: Option<i32>,
}

impl BoundaryKnowledge {
    /// Shallowest offset that ever failed, of ANY severity — the search bound.
    fn lowest_failure(&self) -> Option<i32> {
        [self.lowest_silent_error, self.lowest_tdr, self.lowest_reboot]
            .into_iter()
            .flatten()
            .min()
    }
}

/// Per-GPU continuous knowledge base: the stability curve this specific GPU is
/// learning about itself, accumulated across runs. Keyed by GPU identity so a
/// hardware change starts fresh.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct GpuKnowledge {
    gpu_key: String,
    target_clock_mhz: u32,
    boundary: BoundaryKnowledge,
    /// offset (MHz) → accumulated stats.
    points: std::collections::BTreeMap<i32, PointStat>,
    schema_version: u32,
}

impl GpuKnowledge {
    fn record_stable(&mut self, offset: i32, clock_mhz: u32, power_w: f32, voltage_mv: u32) {
        let e = self.points.entry(offset).or_default();
        e.trials += 1;
        e.stable_trials += 1;
        e.clock_mhz_sum += clock_mhz as u64;
        e.power_w_sum += power_w as f64;
        e.voltage_mv_sum += voltage_mv as u64;
        if offset > self.boundary.highest_clean {
            self.boundary.highest_clean = offset;
        }
    }
    fn record_failure(&mut self, offset: i32, sev: FailSeverity) {
        let e = self.points.entry(offset).or_default();
        e.trials += 1;
        e.failures += 1;
        if sev > e.worst_severity {
            e.worst_severity = sev;
        }
        let slot = match sev {
            FailSeverity::SilentError => &mut self.boundary.lowest_silent_error,
            FailSeverity::Tdr => &mut self.boundary.lowest_tdr,
            FailSeverity::Reboot => &mut self.boundary.lowest_reboot,
            FailSeverity::None => return,
        };
        *slot = Some(slot.map_or(offset, |x| x.min(offset)));
    }
}

#[cfg(windows)]
fn knowledge_path() -> std::path::PathBuf {
    nidavellir_core::safe_loop::default_data_dir().join("gpu_knowledge.json")
}

/// Load the per-GPU knowledge; if the stored key doesn't match this GPU, start fresh
/// (but keep the identity so the first save stamps it).
#[cfg(windows)]
fn load_knowledge(gpu_key: &str) -> GpuKnowledge {
    let mut k: GpuKnowledge = std::fs::read_to_string(knowledge_path())
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default();
    if k.gpu_key != gpu_key {
        k = GpuKnowledge::default();
        k.gpu_key = gpu_key.to_string();
    }
    k
}

#[cfg(windows)]
fn save_knowledge(k: &GpuKnowledge) {
    let _ = std::fs::create_dir_all(nidavellir_core::safe_loop::default_data_dir());
    if let Ok(j) = serde_json::to_string_pretty(k) {
        let _ = std::fs::write(knowledge_path(), j);
    }
}

/// Exploration ceiling (MHz offset), DATA-DRIVEN (no fixed margin):
/// - probe only `PROBE_STEPS` past the deepest known-clean offset (incremental), and
/// - never reach a known failure — back off a margin RELATIVE to the zone width
///   (`highest_clean → lowest_failure`), keeping at least one step of gap.
/// As runs refine the zone, the relative margin tightens around the true frontier.
#[cfg(windows)]
fn explore_ceiling(k: &GpuKnowledge) -> i32 {
    let hc = k.boundary.highest_clean;
    let incremental = hc + PROBE_STEPS * EXPLORE_STEP;
    let by_failure = match k.boundary.lowest_failure() {
        Some(f) => {
            let width = (f - hc).max(EXPLORE_STEP);
            let margin = ((width as f64) * CONSERVATIVE_MARGIN_FRAC).round() as i32;
            (f - margin.max(EXPLORE_STEP)).min(f - EXPLORE_STEP)
        }
        None => ABS_MAX_OFFSET,
    };
    let raw = incremental.min(by_failure).min(ABS_MAX_OFFSET);
    raw.max(DEFAULT_CEILING.min(by_failure.max(EXPLORE_STEP)))
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

// ---------------------------------------------------------------------------
// Forge-state persistence — restore the completed sweep result across restarts.
//
// The runtime `PowerSweepProgress` (profiles, points, knowledge summary) is the
// only place the forged state lives; on restart the handle is rebuilt empty, so
// the UI sees an unforged GPU even though `gpu_knowledge.json` persisted. We
// snapshot the *completed* progress to `forge_state.json` and seed the handle
// from it on startup. This does NOT recompute anything from knowledge and does
// not alter the IPC `PowerSweepProgress` type (the wrapper is service-internal).
// ---------------------------------------------------------------------------

/// Bump when the persisted shape changes incompatibly; older files are ignored.
const FORGE_STATE_SCHEMA: u32 = 1;
/// Keep only the last N log lines in the snapshot (the live log can be long).
const FORGE_STATE_LOG_TAIL: usize = 40;

/// On-disk wrapper around a completed `PowerSweepProgress`, tagged with the GPU
/// it was forged on and a schema version for forward-compatible rejection.
#[derive(serde::Serialize, serde::Deserialize)]
struct ForgeStateFile {
    schema_version: u32,
    gpu_key: String,
    progress: PowerSweepProgress,
}

/// Outcome of decoding a `forge_state.json` payload — kept separate from the FS
/// layer so the validation rules are unit-testable without touching ProgramData.
#[derive(Debug)]
enum ForgeStateLoad {
    Loaded(Box<PowerSweepProgress>),
    GpuMismatch { stored: String },
    SchemaMismatch { found: u32 },
    Corrupt,
}

/// Serialize a completed progress for persistence: force `running=false` and trim
/// the log to its tail so the file stays small. Returns `None` if serialization
/// fails (caller simply skips the write).
fn encode_forge_state(gpu_key: &str, prog: &PowerSweepProgress) -> Option<String> {
    let mut snapshot = prog.clone();
    snapshot.running = false;
    if snapshot.log.len() > FORGE_STATE_LOG_TAIL {
        let drop = snapshot.log.len() - FORGE_STATE_LOG_TAIL;
        snapshot.log.drain(0..drop);
    }
    let file = ForgeStateFile {
        schema_version: FORGE_STATE_SCHEMA,
        gpu_key: gpu_key.to_string(),
        progress: snapshot,
    };
    serde_json::to_string_pretty(&file).ok()
}

/// Validate and decode a persisted payload against the detected GPU. Restored
/// progress always has `running=false` (we never resurrect a "running" sweep).
fn decode_forge_state(json: &str, gpu_key: &str) -> ForgeStateLoad {
    let file: ForgeStateFile = match serde_json::from_str(json.trim_start_matches('\u{feff}')) {
        Ok(f) => f,
        Err(_) => return ForgeStateLoad::Corrupt,
    };
    if file.schema_version != FORGE_STATE_SCHEMA {
        return ForgeStateLoad::SchemaMismatch { found: file.schema_version };
    }
    if file.gpu_key != gpu_key {
        return ForgeStateLoad::GpuMismatch { stored: file.gpu_key };
    }
    let mut prog = file.progress;
    prog.running = false;
    ForgeStateLoad::Loaded(Box::new(prog))
}

#[cfg(windows)]
fn forge_state_path() -> std::path::PathBuf {
    nidavellir_core::safe_loop::default_data_dir().join("forge_state.json")
}

/// Persist the completed sweep result. Best-effort: any failure is logged and
/// ignored so it can never interfere with the sweep finishing.
#[cfg(windows)]
fn save_forge_state(gpu_key: &str, prog: &PowerSweepProgress) {
    let _ = std::fs::create_dir_all(nidavellir_core::safe_loop::default_data_dir());
    match encode_forge_state(gpu_key, prog) {
        Some(j) => match std::fs::write(forge_state_path(), j) {
            Ok(()) => info!(
                "forge_state saved (gpu='{}', {} points)",
                gpu_key,
                prog.points.len()
            ),
            Err(e) => warn!("forge_state save failed: {e}"),
        },
        None => warn!("forge_state serialize failed — not saved"),
    }
}

/// Load the persisted forge result for `gpu_key`, or `None` to start idle.
/// Logs each outcome (loaded / missing / GPU mismatch / failure) per spec.
#[cfg(windows)]
fn load_forge_state(gpu_key: &str) -> Option<PowerSweepProgress> {
    let json = match std::fs::read_to_string(forge_state_path()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("forge_state missing — starting with idle forge state");
            return None;
        }
        Err(e) => {
            warn!("forge_state read failed ({e}) — starting idle");
            return None;
        }
    };
    match decode_forge_state(&json, gpu_key) {
        ForgeStateLoad::Loaded(prog) => {
            info!(
                "forge_state loaded (gpu='{}', {} points)",
                gpu_key,
                prog.points.len()
            );
            Some(*prog)
        }
        ForgeStateLoad::GpuMismatch { stored } => {
            info!(
                "forge_state ignored — GPU mismatch (stored '{stored}', detected '{gpu_key}')"
            );
            None
        }
        ForgeStateLoad::SchemaMismatch { found } => {
            warn!(
                "forge_state ignored — schema {found} != expected {FORGE_STATE_SCHEMA}"
            );
            None
        }
        ForgeStateLoad::Corrupt => {
            warn!("forge_state load failure — payload unreadable, starting idle");
            None
        }
    }
}

/// Build the power-sweep handle, seeded from the persisted forge result when one
/// matches this GPU. The GPU key is derived the same way the sweep keys its
/// knowledge (`read_curve().name`), so keys match reliably.
#[cfg(windows)]
pub fn restore_handle() -> PowerSweepHandle {
    let handle = PowerSweepHandle::default();
    let gpu_key = nidavellir_gpu_nvapi::read_curve()
        .map(|c| c.name)
        .unwrap_or_else(|_| "unknown-gpu".into());
    if let Some(prog) = load_forge_state(&gpu_key) {
        if let Ok(mut g) = handle.progress.lock() {
            *g = prog;
        }
    }
    handle
}

#[cfg(not(windows))]
pub fn restore_handle() -> PowerSweepHandle {
    PowerSweepHandle::default()
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

/// Brokkr's V2 selection: among the off-cap candidates, choose the highest
/// accumulated efficiency (`score`, MHz/W) whose stability confidence (Wilson
/// lower bound over accumulated trials) clears the active profile's threshold.
/// When none qualifies — the usual case until a point is re-tested across runs —
/// fall back to the V1 strategy (best off-cap perf/watt, else least-capped), so
/// selection never returns "no solution". Joins each candidate to `know.points`
/// by offset for its accumulated confidence; data collection is untouched.
#[cfg(windows)]
fn select_brokkrs_v2(
    all_points: &[PowerSweepPoint],
    off_cap: &[PowerSweepPoint],
    know: &GpuKnowledge,
    profile: SweepProfile,
) -> (Option<PowerSweepPoint>, Vec<String>) {
    use std::cmp::Ordering as Ord;
    let threshold = profile.threshold();
    let mut log = Vec::new();

    // Off-cap candidates whose accumulated confidence (Wilson-LB) clears the gate.
    let mut gated: Vec<(PowerSweepPoint, f64, f64)> = off_cap
        .iter()
        .filter_map(|p| {
            let ps = know.points.get(&p.offset_mhz)?;
            Some((*p, ps.confidence(), ps.score()))
        })
        .filter(|(_, conf, _)| *conf >= threshold)
        .collect();

    if !gated.is_empty() {
        // Among the trusted points, take the best accumulated efficiency (MHz/W).
        gated.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ord::Equal));
        let (best, conf, score) = gated[0];
        log.push(format!(
            "BROKKRS V2: candidate=+{} score={:.2} confidence={:.2} threshold={:.2} decision=accepted",
            best.offset_mhz, score, conf, threshold
        ));
        return (Some(best), log);
    }

    // Confidence not yet mature → fall back to the V1 selection.
    let best_conf = off_cap
        .iter()
        .filter_map(|p| know.points.get(&p.offset_mhz).map(|ps| ps.confidence()))
        .fold(0.0_f64, f64::max);
    log.push(format!(
        "BROKKRS V2: no candidate met threshold best_confidence={:.2} threshold={:.2} fallback=V1",
        best_conf, threshold
    ));
    let v1 = if off_cap.is_empty() {
        all_points
            .iter()
            .copied()
            .min_by(|a, b| a.power_capped_frac.partial_cmp(&b.power_capped_frac).unwrap_or(Ord::Equal))
    } else {
        off_cap
            .iter()
            .copied()
            .max_by(|a, b| a.perf_per_watt.partial_cmp(&b.perf_per_watt).unwrap_or(Ord::Equal))
    };
    (v1, log)
}

/// The three forge profiles synthesized from a power frontier (F1 product model).
#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b (multi-clock measurement)
struct ForgeProfiles {
    godforge: Option<PowerSweepPoint>,
    brokkrs: Option<PowerSweepPoint>,
    deep_calm: Option<PowerSweepPoint>,
    log: Vec<String>,
}

/// Synthesize the three forge profiles from a power frontier — each entry a measured
/// operating point plus its accumulated stability confidence (Wilson LB):
/// - **Godforge**  = highest sustained clock (performance).
/// - **Brokkr's**  = best benefit/cost `R = %power_saved ÷ %clock_lost` vs Godforge
///   (balance) — deliberately NOT simply the best MHz/W.
/// - **Deep Calm** = best MHz/W (efficiency).
/// Only points with confidence ≥ `threshold` are eligible; if none qualify the gate
/// is dropped (best-effort) and logged, so synthesis never returns nothing. Pure +
/// unit-tested; the multi-clock frontier that feeds it is produced by F1b.
#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b (multi-clock measurement)
fn synthesize_forge_profiles(frontier: &[(PowerSweepPoint, f64)], threshold: f64) -> ForgeProfiles {
    use std::cmp::Ordering as Ord;
    let mut log = Vec::new();

    // Confidence gate (reuses V2): trust only well-tested points; else best-effort.
    let pool: Vec<(PowerSweepPoint, f64)> = {
        let trusted: Vec<(PowerSweepPoint, f64)> =
            frontier.iter().copied().filter(|(_, c)| *c >= threshold).collect();
        if trusted.is_empty() {
            let best = frontier.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
            log.push(format!(
                "FORGE: no point met confidence ≥ {threshold:.2} (best {best:.2}) — best-effort synthesis"
            ));
            frontier.to_vec()
        } else {
            trusted
        }
    };
    if pool.is_empty() {
        return ForgeProfiles { godforge: None, brokkrs: None, deep_calm: None, log };
    }

    // Godforge = highest sustained clock (ties → the lowest power that holds it).
    let godforge = pool
        .iter()
        .copied()
        .max_by(|a, b| {
            a.0.clock_mhz
                .cmp(&b.0.clock_mhz)
                .then(b.0.power_w.partial_cmp(&a.0.power_w).unwrap_or(Ord::Equal))
        })
        .unwrap();

    // Deep Calm = best efficiency (MHz/W).
    let deep_calm = pool
        .iter()
        .copied()
        .max_by(|a, b| a.0.perf_per_watt.partial_cmp(&b.0.perf_per_watt).unwrap_or(Ord::Equal))
        .unwrap();

    // Brokkr's = best R = %power_saved ÷ %clock_lost vs Godforge, among points that
    // trade some clock for a power win; falls back to Godforge if no such trade exists.
    let gc = godforge.0.clock_mhz as f64;
    let gp = godforge.0.power_w as f64;
    let r_of = |p: &PowerSweepPoint| -> f64 {
        let clk_lost = (gc - p.clock_mhz as f64) / gc;
        let pwr_saved = (gp - p.power_w as f64) / gp;
        if clk_lost > 0.0 { pwr_saved / clk_lost } else { 0.0 }
    };
    let brokkrs = pool
        .iter()
        .copied()
        .filter(|(p, _)| (p.clock_mhz as f64) < gc && (p.power_w as f64) < gp)
        .max_by(|a, b| r_of(&a.0).partial_cmp(&r_of(&b.0)).unwrap_or(Ord::Equal))
        .unwrap_or(godforge);

    log.push(format!(
        "FORGE: Godforge {}MHz/{:.0}W · Brokkr's {}MHz/{:.0}W (R={:.2}) · Deep Calm {}MHz/{:.0}W ({:.2} MHz/W)",
        godforge.0.clock_mhz, godforge.0.power_w,
        brokkrs.0.clock_mhz, brokkrs.0.power_w, r_of(&brokkrs.0),
        deep_calm.0.clock_mhz, deep_calm.0.power_w, deep_calm.0.perf_per_watt
    ));

    ForgeProfiles {
        godforge: Some(godforge.0),
        brokkrs: Some(brokkrs.0),
        deep_calm: Some(deep_calm.0),
        log,
    }
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
    let gpu_key = nidavellir_gpu_nvapi::read_curve()
        .map(|c| c.name)
        .unwrap_or_else(|_| "unknown-gpu".into());
    let mut know = load_knowledge(&gpu_key);
    know.target_clock_mhz = target;
    let ceiling = explore_ceiling(&know);
    let fmt_opt = |o: Option<i32>| o.map(|v| format!("+{v}")).unwrap_or_else(|| "—".into());
    prog.log.push(format!(
        "Conhecimento [{}]: limpo ≤ +{} · SilentError {} · TDR {} · Reboot {} → explorando até +{} (margem {:.0}% da zona).",
        know.gpu_key, know.boundary.highest_clean,
        fmt_opt(know.boundary.lowest_silent_error),
        fmt_opt(know.boundary.lowest_tdr),
        fmt_opt(know.boundary.lowest_reboot),
        ceiling, CONSERVATIVE_MARGIN_FRAC * 100.0
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
                // Snap the measured dwell voltage to a deterministic VF-table bin so
                // the point carries BOTH concepts: measured telemetry AND the
                // deterministic apply/frontier key (decisions.md: voltage split).
                // Read-only; once per stable step (not in the hot sampling loop).
                let vf_curve = nidavellir_gpu_nvapi::read_vf_curve_modern();
                let vf_table_voltage_mv =
                    nidavellir_gpu_nvapi::nearest_vf_bin_at_or_above(&vf_curve, m.volt_mv)
                        .map(|(_, mv)| mv);
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
                    measured_voltage_mv: Some(m.volt_mv),
                    vf_table_voltage_mv,
                });
                // Continuous learning: accumulate this offset's stats + raise the
                // clean frontier. Persisted, so confidence grows across runs.
                know.record_stable(offset, m.clock_mhz, m.power_w, m.volt_mv);
                save_knowledge(&know);
                set(&progress, prog.clone());
                offset += EXPLORE_STEP;
            }
            StabilityResult::Stable => {
                offset += EXPLORE_STEP;
            }
            StabilityResult::SilentError => {
                // First instability = a frontier observation, recorded by SEVERITY.
                // The best EFFICIENCY (not this edge) is the result, chosen below.
                know.record_failure(offset, FailSeverity::SilentError);
                save_knowledge(&know);
                prog.log.push(format!(
                    "Fronteira em +{offset} MHz — {} — registrado (SilentError), parando.",
                    FailTier::L1Instability.label()
                ));
                break;
            }
            StabilityResult::Crash => {
                let _ = gpu::set_core_offset_mhz(0);
                let tier = classify_failure(StabilityResult::Crash, &mut ctx);
                // In-sweep we can only observe up to a (recovered/unrecovered) TDR;
                // a true reboot is learned post-boot via the Safe Loop boot-flag.
                know.record_failure(offset, FailSeverity::Tdr);
                save_knowledge(&know);
                prog.log.push(format!(
                    "Fronteira em +{offset} MHz — {} — registrado (TDR), parando.",
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
    let (brokkrs, v2_log) = select_brokkrs_v2(&prog.points, &off_cap, &know, ACTIVE_PROFILE);
    prog.log.extend(v2_log);
    prog.brokkrs = brokkrs;
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
    // Persist the completed result so the service can reconstruct forged
    // profiles/points after a restart. Only when there is a usable profile, so a
    // failed/empty sweep never overwrites a previously good snapshot.
    if prog.godforge.is_some() {
        save_forge_state(&gpu_key, &prog);
    }
    set(&progress, prog);
    info!("Power sweep finished");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_lower_bound_matches_spec_values() {
        // No trials → no confidence (the gate can never pass on an unseen point).
        assert_eq!(wilson_lower_bound(0, 0, 1.96), 0.0);
        // The V2 premise: a single clean trial is weak; many clean trials are strong.
        let one = wilson_lower_bound(1, 1, 1.96);
        let fifty = wilson_lower_bound(50, 50, 1.96);
        assert!((one - 0.21).abs() < 0.01, "1/1 ≈ 0.21, got {one}");
        assert!((fifty - 0.93).abs() < 0.01, "50/50 ≈ 0.93, got {fifty}");
        // A failure must drag confidence below a perfect record of the same size.
        assert!(wilson_lower_bound(9, 10, 1.96) < wilson_lower_bound(10, 10, 1.96));
    }

    #[cfg(windows)]
    fn pt(offset_mhz: i32, perf_per_watt: f64, capped: f32) -> PowerSweepPoint {
        PowerSweepPoint {
            voltage_mv: 900,
            clock_mhz: 1830,
            offset_mhz,
            power_w: 180.0,
            max_power_w: 185.0,
            power_std_w: 1.0,
            power_capped_frac: capped,
            stable: true,
            perf_per_watt,
            measured_voltage_mv: Some(900),
            vf_table_voltage_mv: None,
        }
    }

    /// Build a knowledge base where each entry has an exact (trials, stable, score).
    #[cfg(windows)]
    fn knowledge_with(entries: &[(i32, u32, u32, f64)]) -> GpuKnowledge {
        let mut k = GpuKnowledge::default();
        for &(offset, trials, stable, score) in entries {
            let e = k.points.entry(offset).or_default();
            e.trials = trials;
            e.stable_trials = stable;
            e.failures = trials - stable;
            // score() = clock_mhz_sum / power_w_sum → choose sums that yield `score`.
            e.power_w_sum = 100.0 * stable as f64;
            e.clock_mhz_sum = (score * e.power_w_sum) as u64;
        }
        k
    }

    #[cfg(windows)]
    #[test]
    fn gate_picks_best_score_among_confident_points() {
        // +195 is the most efficient point that is ALSO well-tested; +210 is more
        // efficient on paper but tested once → its low confidence must keep it out.
        let off_cap = vec![pt(180, 10.24, 0.0), pt(195, 11.0, 0.0), pt(210, 12.0, 0.0)];
        let know = knowledge_with(&[(180, 50, 50, 10.24), (195, 60, 60, 11.0), (210, 1, 1, 12.0)]);
        let (pick, log) = select_brokkrs_v2(&off_cap, &off_cap, &know, SweepProfile::Balanced);
        assert_eq!(pick.unwrap().offset_mhz, 195);
        assert!(log.iter().any(|l| l.contains("decision=accepted")));
    }

    #[cfg(windows)]
    #[test]
    fn gate_falls_back_to_v1_when_confidence_is_immature() {
        // Every point has a single trial (today's real state) → nothing clears .85
        // → V1 fallback chooses the best off-cap perf/watt and logs that it did.
        let off_cap = vec![pt(150, 9.0, 0.0), pt(180, 10.24, 0.0)];
        let know = knowledge_with(&[(150, 1, 1, 9.0), (180, 1, 1, 10.24)]);
        let (pick, log) = select_brokkrs_v2(&off_cap, &off_cap, &know, SweepProfile::Balanced);
        assert_eq!(pick.unwrap().offset_mhz, 180);
        assert!(log.iter().any(|l| l.contains("fallback=V1")));
    }

    #[cfg(windows)]
    fn fp(clock_mhz: u32, power_w: f32) -> PowerSweepPoint {
        PowerSweepPoint {
            voltage_mv: 900,
            clock_mhz,
            offset_mhz: 0,
            power_w,
            max_power_w: power_w + 5.0,
            power_std_w: 1.0,
            power_capped_frac: 0.0,
            stable: true,
            perf_per_watt: clock_mhz as f64 / power_w as f64,
            measured_voltage_mv: Some(900),
            vf_table_voltage_mv: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn forge_synthesis_separates_the_three_profiles() {
        // Multi-clock frontier: clock falls, power falls; all well-tested.
        let frontier = vec![
            (fp(1830, 200.0), 0.95),
            (fp(1815, 181.0), 0.95), // small clock loss, big power win → best R
            (fp(1770, 164.0), 0.95),
            (fp(1740, 158.0), 0.95), // best MHz/W
        ];
        let p = synthesize_forge_profiles(&frontier, 0.85);
        assert_eq!(p.godforge.unwrap().clock_mhz, 1830, "Godforge = highest clock");
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 1815, "Brokkr's = best benefit/cost R");
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 1740, "Deep Calm = best MHz/W");
    }

    #[cfg(windows)]
    #[test]
    fn forge_godforge_respects_confidence_gate() {
        // Top clock barely tested → Godforge drops to the highest TRUSTED clock.
        let frontier = vec![
            (fp(1830, 200.0), 0.20),
            (fp(1815, 181.0), 0.95),
            (fp(1770, 164.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, 0.85);
        assert_eq!(p.godforge.unwrap().clock_mhz, 1815);
    }

    #[cfg(windows)]
    #[test]
    fn forge_falls_back_when_nothing_is_trusted() {
        // Immature data → nothing clears the gate → best-effort, still returns profiles.
        let frontier = vec![(fp(1830, 200.0), 0.21), (fp(1770, 164.0), 0.21)];
        let p = synthesize_forge_profiles(&frontier, 0.85);
        assert!(p.godforge.is_some());
        assert!(p.log.iter().any(|l| l.contains("best-effort")));
    }

    #[cfg(windows)]
    #[test]
    fn power_point_legacy_json_loads_without_new_fields() {
        // A pre-split point (no measured_/vf_table_ fields) must still deserialize,
        // with the new optional fields defaulting to None — no panic on missing fields.
        let legacy = r#"{"voltage_mv":843,"clock_mhz":1785,"offset_mhz":180,
            "power_w":180.0,"max_power_w":185.0,"power_std_w":1.0,
            "power_capped_frac":0.0,"stable":true,"perf_per_watt":9.9}"#;
        let p: PowerSweepPoint = serde_json::from_str(legacy).expect("legacy point loads");
        assert_eq!(p.voltage_mv, 843);
        assert_eq!(p.measured_voltage_mv, None);
        assert_eq!(p.vf_table_voltage_mv, None);
    }

    #[cfg(windows)]
    #[test]
    fn power_point_roundtrips_new_voltage_fields() {
        let mut p = fp(1785, 180.0);
        p.measured_voltage_mv = Some(843);
        p.vf_table_voltage_mv = Some(850);
        let json = serde_json::to_string(&p).expect("encode");
        let back: PowerSweepPoint = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.measured_voltage_mv, Some(843));
        assert_eq!(back.vf_table_voltage_mv, Some(850));
    }

    #[cfg(windows)]
    #[test]
    fn forge_state_roundtrips_and_forces_not_running() {
        let mut prog = idle();
        prog.phase = "done".into();
        prog.running = true; // a running snapshot must restore as NOT running
        prog.stock_clock_mhz = 1786;
        prog.points = vec![fp(1830, 200.0), fp(1815, 181.0)];
        prog.godforge = Some(fp(1830, 200.0));
        prog.brokkrs = Some(fp(1815, 181.0));

        let json = encode_forge_state("RTX-TEST", &prog).expect("encode");
        match decode_forge_state(&json, "RTX-TEST") {
            ForgeStateLoad::Loaded(p) => {
                assert!(!p.running, "restored progress must never be running");
                assert_eq!(p.phase, "done");
                assert_eq!(p.points.len(), 2);
                assert_eq!(p.godforge.unwrap().clock_mhz, 1830);
                assert_eq!(p.brokkrs.unwrap().clock_mhz, 1815);
                assert_eq!(p.stock_clock_mhz, 1786);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn forge_state_rejects_gpu_mismatch() {
        let json = encode_forge_state("RTX-3060Ti", &idle()).unwrap();
        match decode_forge_state(&json, "RTX-4090") {
            ForgeStateLoad::GpuMismatch { stored } => assert_eq!(stored, "RTX-3060Ti"),
            other => panic!("expected GpuMismatch, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn forge_state_rejects_schema_mismatch() {
        let file = ForgeStateFile {
            schema_version: FORGE_STATE_SCHEMA + 1,
            gpu_key: "RTX-TEST".into(),
            progress: idle(),
        };
        let json = serde_json::to_string(&file).unwrap();
        match decode_forge_state(&json, "RTX-TEST") {
            ForgeStateLoad::SchemaMismatch { found } => assert_eq!(found, FORGE_STATE_SCHEMA + 1),
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn forge_state_rejects_corrupt_payload() {
        assert!(matches!(
            decode_forge_state("{ not valid json", "RTX-TEST"),
            ForgeStateLoad::Corrupt
        ));
    }

    #[cfg(windows)]
    #[test]
    fn forge_state_log_trimmed_to_tail() {
        let mut prog = idle();
        prog.log = (0..FORGE_STATE_LOG_TAIL + 25)
            .map(|i| format!("line {i}"))
            .collect();
        let json = encode_forge_state("RTX-TEST", &prog).unwrap();
        match decode_forge_state(&json, "RTX-TEST") {
            ForgeStateLoad::Loaded(p) => {
                assert_eq!(p.log.len(), FORGE_STATE_LOG_TAIL);
                // The tail is kept (oldest lines dropped).
                assert_eq!(
                    p.log.last().unwrap(),
                    &format!("line {}", FORGE_STATE_LOG_TAIL + 24)
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }
}
