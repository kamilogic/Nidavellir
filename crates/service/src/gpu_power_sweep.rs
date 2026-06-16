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
use nidavellir_core::ipc::{DwellQuality, PowerSweepPoint, PowerSweepProgress};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
use tracing::{info, warn};

// Long enough for power to RAMP UP and stabilize (real loads like Heaven take
// seconds to reach their sustained draw — it's a ramp, not a spike). We discard
// the ramp and take the WORST CASE (max), not the mean.
const DWELL_MS: u64 = 15000;
const RAMP_DISCARD_MS: u128 = 6000;
/// Plausible GPU core-voltage range (mV). Samples outside are dropped from the
/// measured-voltage stats as sensor glitches (a 0 mV / out-of-range read is noise).
const VOLT_SANE_MIN_MV: u32 = 500;
const VOLT_SANE_MAX_MV: u32 = 1250;
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

/// Read-only: load the persisted forge result for THIS GPU (the completed
/// `PowerSweepProgress` from `forge_state.json`), or `None` if absent/mismatched.
/// Path-independent (no `PowerSweepHandle` / `AppState` needed), so both the IPC
/// verifier and the `verify-applied` console subcommand can locate the applied
/// point's dwell stats. Never mutates anything.
#[cfg(windows)]
pub fn load_restored_progress() -> Option<PowerSweepProgress> {
    let gpu_key = nidavellir_gpu_nvapi::read_curve()
        .map(|c| c.name)
        .unwrap_or_else(|_| "unknown-gpu".into());
    load_forge_state(&gpu_key)
}

#[cfg(not(windows))]
pub fn load_restored_progress() -> Option<PowerSweepProgress> {
    None
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
    /// LEGACY: unfiltered AtomicU32 fetch_max; unchanged so the apply key is stable.
    volt_mv: u32,
    // ── Richer dwell stats (additive; computed from the retained raw samples) ──
    /// Post-ramp clock/power sample count and the steady-state window duration.
    sample_count: u32,
    duration_ms: u64,
    /// Sustained-clock distribution (post-ramp): lowest and 5th percentile.
    min_clock_mhz: u32,
    p5_clock_mhz: u32,
    /// Ramp-filtered + sanity-checked measured-voltage stats (telemetry only).
    volt_min_mv: Option<u32>,
    volt_avg_mv: Option<u32>,
    volt_max_mv: Option<u32>,
    volt_sample_count: u32,
    /// Steady-state temperature start/end/mean (°C), if NVML reported it.
    start_temp_c: Option<f32>,
    end_temp_c: Option<f32>,
    avg_temp_c: Option<f32>,
}

impl Measured {
    /// A no-data result (device init failed / no samples) carrying only the legacy
    /// voltage max. All richer stats are absent.
    fn degenerate(result: StabilityResult, volt_mv: u32) -> Self {
        Measured {
            result,
            clock_mhz: 0,
            power_w: 0.0,
            max_power_w: 0.0,
            power_std_w: 0.0,
            capped_frac: 0.0,
            volt_mv,
            sample_count: 0,
            duration_ms: 0,
            min_clock_mhz: 0,
            p5_clock_mhz: 0,
            volt_min_mv: None,
            volt_avg_mv: None,
            volt_max_mv: None,
            volt_sample_count: 0,
            start_temp_c: None,
            end_temp_c: None,
            avg_temp_c: None,
        }
    }
}

/// 5th-percentile (lower) clock of a sample set — the "bad-case" sustained clock.
/// `None` if empty. Pure + testable.
fn p5_clock_mhz(clocks: &[u32]) -> Option<u32> {
    if clocks.is_empty() {
        return None;
    }
    let mut s = clocks.to_vec();
    s.sort_unstable();
    let idx = (((s.len() - 1) as f64) * 0.05).floor() as usize;
    Some(s[idx])
}

/// Aggregate already-validated voltage samples → `(min, avg, max, count)`.
/// Empty → `None`. Pure + testable.
fn voltage_stats(samples: &[u32]) -> Option<(u32, u32, u32, u32)> {
    if samples.is_empty() {
        return None;
    }
    let count = samples.len() as u32;
    let min = *samples.iter().min().unwrap();
    let max = *samples.iter().max().unwrap();
    let avg = (samples.iter().map(|&v| v as u64).sum::<u64>() / count as u64) as u32;
    Some((min, avg, max, count))
}

/// Telemetry confidence from valid clock/power sample count.
fn clock_power_quality(sample_count: u32) -> DwellQuality {
    match sample_count {
        0 => DwellQuality::Unavailable,
        1..=29 => DwellQuality::Low,
        30..=99 => DwellQuality::Medium,
        _ => DwellQuality::High,
    }
}

/// Telemetry confidence from valid voltage sample count (sparser cadence → lower bars).
fn voltage_quality(sample_count: u32) -> DwellQuality {
    match sample_count {
        0 => DwellQuality::Unavailable,
        1..=9 => DwellQuality::Low,
        10..=49 => DwellQuality::Medium,
        _ => DwellQuality::High,
    }
}

/// The worst (most conservative) of two qualities.
fn worst_quality(a: DwellQuality, b: DwellQuality) -> DwellQuality {
    fn rank(q: DwellQuality) -> u8 {
        match q {
            DwellQuality::Unavailable => 0,
            DwellQuality::Low => 1,
            DwellQuality::Medium => 2,
            DwellQuality::High => 3,
        }
    }
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
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
        Err(_) => return Measured::degenerate(StabilityResult::Crash, 0),
    };
    let stop = Arc::new(AtomicBool::new(false));
    // Collect raw samples in the sampler thread for precise stats (mean/max/std + the
    // richer min/p5/temperature stats). Tuple: (clock_mhz, power_w, capped, temp_c).
    let samples: Arc<Mutex<Vec<(u32, f32, bool, Option<f32>)>>> = Arc::new(Mutex::new(Vec::new()));
    let volt = Arc::new(AtomicU32::new(0));
    // Ramp-filtered + sanity-checked voltage samples → measured-voltage telemetry
    // (avg/min/max/count). The legacy `volt` AtomicU32 max is kept UNCHANGED so the
    // apply key (which snaps `volt_mv`) is unaffected.
    let volts: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let (s2, smp, vlt, vsmp) = (stop.clone(), samples.clone(), volt.clone(), volts.clone());
    let t0 = std::time::Instant::now();
    let sampler = std::thread::spawn(move || {
        let mut tick: u32 = 0;
        while !s2.load(Ordering::SeqCst) {
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let (Some(c), Some(p)) = (r.core_clock_mhz, r.power_w) {
                    // Discard the ramp-up — steady state only.
                    if t0.elapsed().as_millis() >= RAMP_DISCARD_MS {
                        if let Ok(mut v) = smp.lock() {
                            v.push((c, p, r.power_capped == Some(true), r.temperature_c));
                        }
                    }
                }
            }
            // Voltage via NVAPI is heavier (re-inits), so sample it sparsely.
            tick += 1;
            if tick % 16 == 0 {
                if let Some(mv) = nidavellir_gpu_nvapi::read_core_voltage_mv() {
                    vlt.fetch_max(mv, Ordering::SeqCst); // legacy max — unchanged
                    // Additive telemetry: ramp-filter + sanity-check, like clock/power.
                    if t0.elapsed().as_millis() >= RAMP_DISCARD_MS
                        && (VOLT_SANE_MIN_MV..=VOLT_SANE_MAX_MV).contains(&mv)
                    {
                        if let Ok(mut g) = vsmp.lock() {
                            g.push(mv);
                        }
                    }
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
    let duration_ms = t0.elapsed().as_millis() as u64;
    let v = samples.lock().map(|g| g.clone()).unwrap_or_default();
    let volt_samples = volts.lock().map(|g| g.clone()).unwrap_or_default();
    let (volt_min_mv, volt_avg_mv, volt_max_mv, volt_sample_count) = match voltage_stats(&volt_samples)
    {
        Some((mn, avg, mx, c)) => (Some(mn), Some(avg), Some(mx), c),
        None => (None, None, None, 0),
    };
    if v.is_empty() {
        return Measured {
            volt_min_mv,
            volt_avg_mv,
            volt_max_mv,
            volt_sample_count,
            duration_ms,
            ..Measured::degenerate(res, volt_mv)
        };
    }
    let n = v.len() as f32;
    let clock = (v.iter().map(|s| s.0 as u64).sum::<u64>() / v.len() as u64) as u32;
    let mean_p = v.iter().map(|s| s.1).sum::<f32>() / n;
    let max_p = v.iter().map(|s| s.1).fold(0.0f32, f32::max);
    let var = v.iter().map(|s| (s.1 - mean_p).powi(2)).sum::<f32>() / n;
    let std_p = var.sqrt();
    let capped = v.iter().filter(|s| s.2).count() as f32 / n;
    let clocks: Vec<u32> = v.iter().map(|s| s.0).collect();
    let min_clock = clocks.iter().copied().min().unwrap_or(0);
    let p5_clock = p5_clock_mhz(&clocks).unwrap_or(0);
    let temps: Vec<f32> = v.iter().filter_map(|s| s.3).collect();
    let (start_temp_c, end_temp_c, avg_temp_c) = if temps.is_empty() {
        (None, None, None)
    } else {
        let avg = temps.iter().sum::<f32>() / temps.len() as f32;
        (Some(temps[0]), Some(temps[temps.len() - 1]), Some(avg))
    };
    Measured {
        result: res,
        clock_mhz: clock,
        power_w: mean_p,
        max_power_w: max_p,
        power_std_w: std_p,
        capped_frac: capped,
        volt_mv,
        sample_count: v.len() as u32,
        duration_ms,
        min_clock_mhz: min_clock,
        p5_clock_mhz: p5_clock,
        volt_min_mv,
        volt_avg_mv,
        volt_max_mv,
        volt_sample_count,
        start_temp_c,
        end_temp_c,
        avg_temp_c,
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
/// Centralized product policy for profile synthesis (F1b) — thresholds live here, not
/// scattered through the algorithm. Clock floors keep Brokkr's near Godforge and keep
/// Deep Calm useful; the confidence threshold reuses the V2 Wilson gate.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // presets wired into the live sweep by F1b Phase 2
struct ForgePolicy {
    /// Brokkr's Best must keep >= this fraction of Godforge's (sustained) clock.
    brokkrs_min_clock_frac: f64,
    /// Deep Calm must keep >= this fraction of Godforge's (sustained) clock.
    deep_calm_min_clock_frac: f64,
    /// Minimum stability confidence (Wilson LB) a point must clear to be eligible.
    confidence_threshold: f64,
}

#[cfg(windows)]
#[allow(dead_code)] // conservative/aggressive wired by F1b Phase 2 (profile selector)
impl ForgePolicy {
    /// Default daily-use policy: Brokkr's >= 98% clock, Deep Calm >= 90% clock, gate .85.
    fn balanced() -> Self {
        Self { brokkrs_min_clock_frac: 0.98, deep_calm_min_clock_frac: 0.90, confidence_threshold: 0.85 }
    }
    fn conservative() -> Self {
        Self { brokkrs_min_clock_frac: 0.99, deep_calm_min_clock_frac: 0.92, confidence_threshold: 0.95 }
    }
    fn aggressive() -> Self {
        Self { brokkrs_min_clock_frac: 0.97, deep_calm_min_clock_frac: 0.85, confidence_threshold: 0.70 }
    }
}

/// Observed GPU constraint regime — descriptive, and drives the candidate-clock range.
/// `VoltageLimited` is reserved: it can't be told from a single stock sample (it emerges
/// during the sweep when raising voltage stops raising clock), so `classify_regime`
/// does not yet produce it.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // wired into the live sweep by F1b Phase 2
enum Regime {
    PowerLimited,
    VoltageLimited,
    ThermalLimited,
    Mixed,
    Unconstrained,
}

/// Classify the regime from a stock heavy-load telemetry sample. Pure + testable.
/// `cap_fraction` 0..1, `power_limit_w`/`temp_c` optional context. A conservative ~83 °C
/// thermal-throttle threshold is assumed when a temperature is present.
#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b Phase 2
fn classify_regime(cap_fraction: f32, power_w: f32, power_limit_w: f32, temp_c: Option<f32>) -> Regime {
    let near_power = cap_fraction > 0.5 || (power_limit_w > 0.0 && power_w >= 0.95 * power_limit_w);
    let near_thermal = temp_c.map_or(false, |t| t >= 83.0);
    match (near_power, near_thermal) {
        (true, true) => Regime::Mixed,
        (true, false) => Regime::PowerLimited,
        (false, true) => Regime::ThermalLimited,
        (false, false) => Regime::Unconstrained,
    }
}

/// Build the descending candidate target-clock list to probe. Power/thermal/mixed
/// regimes don't probe above the sustained clock (the cap/heat holds it); an
/// unconstrained GPU may explore a few steps ABOVE the stock boost ceiling (real OC).
/// The floor is `floor_frac × stock_sustained` (the Deep Calm floor). Pure + testable.
#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b Phase 2
fn candidate_clocks(
    stock_sustained_mhz: u32,
    stock_boost_max_mhz: u32,
    regime: Regime,
    step: u32,
    floor_frac: f64,
) -> Vec<u32> {
    let step = step.max(1);
    let top = match regime {
        Regime::Unconstrained => stock_boost_max_mhz.max(stock_sustained_mhz) + 3 * step,
        _ => stock_sustained_mhz,
    };
    let floor = ((stock_sustained_mhz as f64) * floor_frac).round() as u32;
    let mut out = Vec::new();
    let mut c = top;
    while c >= floor {
        out.push(c);
        if c < step {
            break;
        }
        c -= step;
    }
    out
}

#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b (multi-clock measurement)
struct ForgeProfiles {
    godforge: Option<PowerSweepPoint>,
    brokkrs: Option<PowerSweepPoint>,
    deep_calm: Option<PowerSweepPoint>,
    /// Frontier points excluded from differentiation because they were power-bound
    /// (`power_capped_frac >= POWER_BOUND_FRAC`) — valid raw brackets, but no clock-frontier value.
    power_bound_excluded: usize,
    /// True when fewer than `MIN_USEFUL_FRONTIER_POINTS` useful (non-power-bound) points remained:
    /// the returned profiles are a FLAGGED best-effort over the full frontier, NOT a differentiated
    /// VF frontier. The operator must not read them as a real clock frontier.
    power_bound_collapse: bool,
    log: Vec<String>,
}

/// A dwell is **power-bound** when it stayed pinned at the power cap for at least this fraction of
/// its samples: the achieved clock was set by the cap, not the voltage descent, so the point carries
/// no clock-frontier information. From the F1b algorithm audit (`decisions.md`): `POWER_BOUND_FRAC`.
#[cfg(windows)]
const POWER_BOUND_FRAC: f32 = 0.95;

/// Fewer than this many useful (non-power-bound) points means the frontier cannot be differentiated
/// on clock — synthesis falls back to a flagged best-effort instead of inventing differentiation.
#[cfg(windows)]
const MIN_USEFUL_FRONTIER_POINTS: usize = 2;

/// True iff `f` is a VALID power-cap fraction at or above the power-bound threshold. A missing /
/// invalid fraction (NaN / <0 / >1) is NOT treated as power-bound: an unknown cap state must never
/// silently mark a point as a power-bound plateau. Pure (reuses `valid_cap_frac`).
#[cfg(windows)]
fn is_power_bound_frac(f: f32) -> bool {
    valid_cap_frac(f).map_or(false, |v| v >= POWER_BOUND_FRAC)
}

/// True iff a (stable, verified) frontier point is power-bound — its dwell stayed at the power cap.
#[cfg(windows)]
fn is_power_bound_point(p: &PowerSweepPoint) -> bool {
    is_power_bound_frac(p.power_capped_frac)
}

/// The frontier points that carry real clock-frontier information (NOT power-bound). Pure.
#[cfg(windows)]
fn useful_frontier_points(frontier: &[(PowerSweepPoint, f64)]) -> Vec<(PowerSweepPoint, f64)> {
    frontier.iter().copied().filter(|(p, _)| !is_power_bound_point(p)).collect()
}

/// True iff the frontier is non-empty, has at least one power-bound point, AND fewer than
/// `MIN_USEFUL_FRONTIER_POINTS` useful (non-power-bound) points — i.e. (nearly) every sample was
/// power-capped, so no differentiated VF frontier can be built. Detects the jittery power-bound
/// plateau (e.g. 1798/1811/1819 MHz @ pcf≈1.0) that exact-distinct-clock checks miss. Pure + testable.
#[cfg(windows)]
fn frontier_power_bound_collapse(frontier: &[(PowerSweepPoint, f64)]) -> bool {
    !frontier.is_empty()
        && frontier.iter().any(|(p, _)| is_power_bound_point(p))
        && useful_frontier_points(frontier).len() < MIN_USEFUL_FRONTIER_POINTS
}

// ── F1c: two-phase power-bound knee-seeking (pure helpers) ───────────────────────────────────────
// Design audit (`decisions.md`, 2026-06-15): a Phase-A `PowerBoundCollapse` is an HONEST diagnostic
// for a SHALLOW descent, NOT proof that no VF frontier exists. The validated collapse run only walked
// the top ~13 mV (bins 1075/1068/1062) — ~130 mV ABOVE the card's real operating voltage — so the VF
// ceiling never bit (`apply_vf_ceiling_monotone` only caps bins at voltage ≥ ceiling_mv) and every
// dwell stayed pcf-saturated. A power-bound descent has three voltage regions: ABOVE the knee (ceiling
// inert, pcf≈1.0, clock pinned by the power cap — no frontier info); AT the knee (the lowest ceiling
// that still sustains the power-limited clock — candidate Godforge); BELOW the knee (ceiling controls
// clock/power, pcf drops — the useful Brokkr's / Deep Calm efficiency tail). These pure helpers let an
// OPT-IN Phase B aim a focused deeper descent at the knee. No hardware, no scheduler state.

/// Minimum power-bound points before `detect_plateau_clock` will report a plateau — a single
/// saturated dwell is not a plateau. Mirrors `MIN_USEFUL_FRONTIER_POINTS` on the saturated side.
#[cfg(windows)]
const MIN_PLATEAU_POINTS: usize = 2;

/// Robust representative of the power-limited plateau clock: the MEDIAN sustained clock among the
/// power-bound points. Median (not exact-distinct) is robust to the jittery saturated plateau
/// (e.g. 1798/1811/1819 MHz @ pcf≈1.0 — which exact-distinct detection mis-reads as 3 real clocks).
/// `None` unless at least `MIN_PLATEAU_POINTS` power-bound points with a positive clock exist: without
/// a real plateau there is nothing for a focused Phase-B descent to aim at. Pure + testable.
#[cfg(windows)]
fn detect_plateau_clock(frontier: &[(PowerSweepPoint, f64)]) -> Option<u32> {
    let mut clocks: Vec<u32> = frontier
        .iter()
        .filter(|(p, _)| is_power_bound_point(p))
        .map(|(p, _)| p.p5_clock_mhz.unwrap_or(p.clock_mhz))
        .filter(|&c| c > 0)
        .collect();
    if clocks.len() < MIN_PLATEAU_POINTS {
        return None;
    }
    clocks.sort_unstable();
    Some(clocks[clocks.len() / 2])
}

/// Pick the focused Phase-B target for a detected `plateau_clock`: the LOWEST candidate clock still
/// ≥ the plateau, so the clock cap never binds BELOW the plateau — we want the VOLTAGE ceiling, not the
/// target, to be the binding constraint as the descent crosses the knee — without wasting effort on
/// clearly-unreachable higher targets. If no candidate reaches the plateau, fall back to the nearest
/// candidate. Pure + testable; no hardware.
#[cfg(windows)]
fn select_phase_b_target(candidates: &[u32], plateau_clock: u32) -> Option<u32> {
    candidates
        .iter()
        .copied()
        .filter(|&c| c >= plateau_clock)
        .min()
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .min_by_key(|&c| (c as i64 - plateau_clock as i64).abs())
        })
}

/// One step of a descending-voltage trajectory, classified purely from the power-cap fraction.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KneeTransition {
    /// Still power-bound (`pcf >= POWER_BOUND_FRAC`): the ceiling has not bitten — keep descending.
    AboveKnee,
    /// Just crossed the knee: the previous point was power-bound (or absent) and this one LEFT
    /// saturation (`pcf < POWER_BOUND_FRAC`). The first below-knee point.
    KneeCrossed,
    /// Both the previous and current points are off the plateau: the below-knee efficiency tail.
    BelowKneeTail,
}

/// Classify one descending step from the previous/current power-cap fraction. A missing/invalid
/// previous pcf is treated as "above the knee" (a descent starts saturated), so the FIRST off-cap
/// point reads as `KneeCrossed`. Reuses the shared `is_power_bound_frac` threshold so the knee and the
/// synthesis power-bound exclusion always agree. Pure + testable.
#[cfg(windows)]
fn classify_knee_transition(prev_pcf: Option<f32>, cur_pcf: f32) -> KneeTransition {
    let cur_bound = is_power_bound_frac(cur_pcf);
    let prev_bound = prev_pcf.map_or(true, is_power_bound_frac);
    if cur_bound {
        KneeTransition::AboveKnee
    } else if prev_bound {
        KneeTransition::KneeCrossed
    } else {
        KneeTransition::BelowKneeTail
    }
}

/// Find the knee in a descending-voltage trajectory: the index of the FIRST point that LEAVES power-
/// cap saturation (`KneeCrossed`). `None` means the descent stayed power-bound throughout — no knee was
/// reached (too shallow / still above the operating voltage), so synthesis must NOT differentiate and
/// the honest `PowerBoundCollapse` stands. Pure + testable; no hardware.
#[cfg(windows)]
fn detect_power_bound_knee(trajectory: &[(PowerSweepPoint, f64)]) -> Option<usize> {
    let mut prev_pcf: Option<f32> = None;
    for (i, (p, _)) in trajectory.iter().enumerate() {
        if classify_knee_transition(prev_pcf, p.power_capped_frac) == KneeTransition::KneeCrossed {
            return Some(i);
        }
        prev_pcf = Some(p.power_capped_frac);
    }
    None
}

/// The deepest (lowest-mV) VF bin Phase A retained a stable frontier point at for `target` — the
/// bottom of what Phase A already covered for it. `None` when Phase A kept no point for it (target not
/// probed, or dropped without a stable bin). In the power-bound-collapse case that gates Phase B every
/// Phase-A probe was stable, so this is the deepest bin Phase A actually probed for that target. Pure.
#[cfg(windows)]
fn phase_a_deepest_bin(frontier: &[(PowerSweepPoint, f64)], target: u32) -> Option<u32> {
    frontier
        .iter()
        .filter(|(p, _)| p.target_clock_mhz == Some(target))
        .filter_map(|(p, _)| p.vf_table_voltage_mv)
        .min()
}

/// The Phase-B descent start when CONTINUING below Phase A: the highest real VF bin STRICTLY below
/// `phase_a_floor_mv`, so Phase B descends only new, deeper bins instead of re-probing the inert top
/// bins Phase A already covered (the budget-efficiency fix on fine-grained VF curves). `None` when no
/// lower real bin exists (Phase A already reached the hardware floor) — nothing deeper for Phase B to
/// do. Only real curve bins are ever returned. Pure + testable; no hardware.
#[cfg(windows)]
fn phase_b_start_below(descent: &FrontierDescent, phase_a_floor_mv: u32) -> Option<u32> {
    descent.bins_desc.iter().copied().filter(|&b| b < phase_a_floor_mv).max()
}

/// Synthesize the three forge profiles from a (multi-clock) power frontier — each entry
/// a measured operating point plus its accumulated stability confidence (Wilson LB):
/// - **Godforge**  = highest SUSTAINED clock (performance); ties → lowest power.
/// - **Brokkr's**  = best benefit/cost `R = %power_saved ÷ %clock_lost` vs Godforge,
///   among points that keep ≥ `policy.brokkrs_min_clock_frac` of Godforge's clock
///   (so Brokkr's stays near Godforge and never collapses into Deep Calm). Max R wins.
/// - **Deep Calm** = best MHz/W among points that keep ≥ `policy.deep_calm_min_clock_frac`
///   of Godforge's clock (so it stays useful, never a near-idle clock).
///
/// Sustainability uses `p5_clock_mhz` when present (dip-aware), else `clock_mhz`
/// (legacy fallback). Selection uses clock / power / p5 / confidence ONLY — NEVER
/// measured voltage (`vf_table_voltage_mv` is the deterministic apply axis, not used
/// for selection here). Only points with confidence ≥ `policy.confidence_threshold`
/// are eligible; if none qualify the gate is dropped (best-effort) and logged, so
/// synthesis never returns nothing. Pure + unit-tested; the multi-clock frontier that
/// feeds it is produced by F1b Phase 2.
#[cfg(windows)]
#[allow(dead_code)] // wired into the live sweep by F1b Phase 2 (multi-clock measurement)
fn synthesize_forge_profiles(frontier: &[(PowerSweepPoint, f64)], policy: &ForgePolicy) -> ForgeProfiles {
    use std::cmp::Ordering as Ord;
    let mut log = Vec::new();

    // ── Power-bound classification (F1b audit) ──────────────────────────────────────────────────
    // A power-bound point (`power_capped_frac >= POWER_BOUND_FRAC`) is a VALID raw voltage bracket
    // but carries NO clock-frontier information — the cap, not the descent, set its clock. Such
    // points are EXCLUDED from differentiated profile selection. When fewer than two useful
    // (non-power-bound) points remain, the frontier is power-bound-collapsed: we keep the full
    // frontier only as a FLAGGED best-effort and never present a jittery saturated plateau as a
    // differentiated VF frontier. With NO power-bound points this is a no-op: `working == frontier`,
    // so the legacy (non-power-bound) path is byte-for-byte unchanged.
    let useful = useful_frontier_points(frontier);
    let power_bound_excluded = frontier.len() - useful.len();
    let power_bound_collapse = frontier_power_bound_collapse(frontier);
    let working: Vec<(PowerSweepPoint, f64)> = if power_bound_excluded == 0 {
        frontier.to_vec()
    } else if power_bound_collapse {
        log.push(format!(
            "FORGE: power-bound collapse — cannot build a differentiated VF frontier under this \
             workload/regime ({power_bound_excluded}/{} point(s) power-capped >= {:.2}, {} useful) \
             — best-effort only, not differentiated",
            frontier.len(), POWER_BOUND_FRAC, useful.len()
        ));
        frontier.to_vec()
    } else {
        log.push(format!(
            "FORGE: excluded {power_bound_excluded} power-bound point(s) (power_capped_frac >= {:.2}) \
             from differentiation — synthesizing from {} useful point(s)",
            POWER_BOUND_FRAC, useful.len()
        ));
        useful
    };

    // Confidence gate (reuses V2): trust only well-tested points; else best-effort.
    let pool: Vec<(PowerSweepPoint, f64)> = {
        let trusted: Vec<(PowerSweepPoint, f64)> = working
            .iter()
            .copied()
            .filter(|(_, c)| *c >= policy.confidence_threshold)
            .collect();
        if trusted.is_empty() {
            let best = working.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
            log.push(format!(
                "FORGE: no point met confidence ≥ {:.2} (best {best:.2}) — best-effort synthesis",
                policy.confidence_threshold
            ));
            working.clone()
        } else {
            trusted
        }
    };
    if pool.is_empty() {
        return ForgeProfiles {
            godforge: None, brokkrs: None, deep_calm: None,
            power_bound_excluded, power_bound_collapse, log,
        };
    }

    // Sustained clock = p5 when available (dip-aware), else average (legacy fallback).
    let sustained = |p: &PowerSweepPoint| p.p5_clock_mhz.unwrap_or(p.clock_mhz);

    // Godforge = highest sustainable clock (ties → the lowest power that holds it).
    let godforge = pool
        .iter()
        .copied()
        .max_by(|a, b| {
            sustained(&a.0)
                .cmp(&sustained(&b.0))
                .then(b.0.power_w.partial_cmp(&a.0.power_w).unwrap_or(Ord::Equal))
        })
        .unwrap();
    let gc = sustained(&godforge.0) as f64;
    let gp = godforge.0.power_w as f64;

    // Collapse detection: a single distinct sustainable clock → can't differentiate on
    // clock (the failure mode of the old single-clock sweep). Still return valid profiles.
    let distinct_clocks = {
        let mut cs: Vec<u32> = pool.iter().map(|(p, _)| sustained(p)).collect();
        cs.sort_unstable();
        cs.dedup();
        cs.len()
    };
    if distinct_clocks <= 1 {
        log.push(format!(
            "FORGE: frontier has a single sustainable clock ({} MHz) — profiles cannot \
             differentiate on clock; run a multi-clock sweep (F1b)",
            gc as u32
        ));
    }

    // Deep Calm = best MHz/W within the Deep Calm clock floor (stays useful).
    let dc_floor = gc * policy.deep_calm_min_clock_frac;
    let deep_calm = pool
        .iter()
        .copied()
        .filter(|(p, _)| sustained(p) as f64 >= dc_floor)
        .max_by(|a, b| a.0.perf_per_watt.partial_cmp(&b.0.perf_per_watt).unwrap_or(Ord::Equal))
        .unwrap_or(godforge);

    // Brokkr's = best R within the Brokkr's clock floor; must be a real trade (clock
    // below Godforge AND less power). Falls back to Godforge if no such point exists.
    let br_floor = gc * policy.brokkrs_min_clock_frac;
    let r_of = |p: &PowerSweepPoint| -> f64 {
        let clk_lost = (gc - sustained(p) as f64) / gc;
        let pwr_saved = (gp - p.power_w as f64) / gp;
        if clk_lost > 0.0 { pwr_saved / clk_lost } else { 0.0 }
    };
    let brokkrs = pool
        .iter()
        .copied()
        .filter(|(p, _)| {
            let s = sustained(p) as f64;
            s >= br_floor && s < gc && (p.power_w as f64) < gp
        })
        .max_by(|a, b| r_of(&a.0).partial_cmp(&r_of(&b.0)).unwrap_or(Ord::Equal))
        .unwrap_or(godforge);

    log.push(format!(
        "FORGE: Godforge {}MHz/{:.0}W · Brokkr's {}MHz/{:.0}W (R={:.2}, floor {:.0}%) · \
         Deep Calm {}MHz/{:.0}W ({:.2} MHz/W, floor {:.0}%)",
        sustained(&godforge.0), godforge.0.power_w,
        sustained(&brokkrs.0), brokkrs.0.power_w, r_of(&brokkrs.0), policy.brokkrs_min_clock_frac * 100.0,
        sustained(&deep_calm.0), deep_calm.0.power_w, deep_calm.0.perf_per_watt, policy.deep_calm_min_clock_frac * 100.0
    ));

    ForgeProfiles {
        godforge: Some(godforge.0),
        brokkrs: Some(brokkrs.0),
        deep_calm: Some(deep_calm.0),
        power_bound_excluded,
        power_bound_collapse,
        log,
    }
}

// ── F1b Phase 2A: simulated multi-clock outer-loop scaffolding ──────────────────
// A generic frontier builder that drives the multi-clock loop through an INJECTED
// probe closure. In Phase 2A the closure is always simulated (tests); Phase 2B will
// pass a real closure that applies the ceiling + runs `load_and_measure` under
// supervised approval. The loop itself never touches hardware — no VF write, no
// `apply_vf_ceiling`, no `load_and_measure`, no Safe Loop interaction.

/// Outcome of a single probe (one target clock at one candidate voltage bin).
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // richer severities arrive with Phase 2B/3
enum ProbeOutcome {
    Stable,
    Unstable,
}

/// What a probe returns — models exactly what a real dwell would yield, so the loop
/// logic is identical whether the closure is simulated (2A) or real (2B).
#[cfg(windows)]
#[derive(Debug, Clone)]
#[allow(dead_code)] // wired to the real measurement path in Phase 2B
struct ProbeSample {
    outcome: ProbeOutcome,
    /// Simulated Patch-A curve verification (Phase 2B: real offset-readback). When
    /// false, the ceiling did not take → stop descending this clock.
    curve_verified: bool,
    avg_clock_mhz: u32,
    p5_clock_mhz: Option<u32>,
    power_w: f32,
    max_power_w: f32,
    power_capped_frac: f32,
    measured_voltage_mv: Option<u32>,
    /// The deterministic VF-table bin ACTUALLY applied (snapped). Set by the real probe
    /// (Phase 2B.2-b); `None` from the pure mapper. When present it becomes the frontier
    /// point's `vf_table_voltage_mv` apply key. Internal only — NOT IPC.
    vf_bin_mv: Option<u32>,
    telemetry_quality: DwellQuality,
    voltage_quality: DwellQuality,
    /// Accumulated stability confidence (Wilson LB) for this point — feeds the gate.
    confidence: f64,
    // ── Scheduler drain/hard-failure signals (warm-start bracket carry-forward). All default
    //    false. They do NOT change the frontier or the probe sequence; they let the descent
    //    classify WHY it stopped and gate the B2 warm-start fallback (which must NOT fire on a
    //    drain or a crash). Set by the real probe / closure; left false by the pure mappers. ──
    /// `--max-probes` budget short-circuit fired (no hardware ran) — a drain, not a finding.
    budget_drained: bool,
    /// A prior crash set the run abort flag and this probe short-circuited (no hardware ran).
    aborted: bool,
    /// THIS probe's dwell crashed (TDR) — a hard failure; the run aborts.
    crashed: bool,
}

/// Voltage-bin descent config for a target clock. The descent walks `bins_desc` — the GPU's REAL
/// VF-table voltage bins in [`lowest_safe_mv`..=`safe_start_mv`], strictly descending — so it only
/// ever probes voltages that exist in the curve. `lowest_safe_mv` (= `bins_desc.last()`) is the
/// HARDWARE-DERIVED floor: the lowest real graphics-core VF bin. `safe_start_mv` (= the highest
/// included bin, ≤ any `--safe-start-cap`) is the descent start. `voltage_step_mv` is the nominal
/// spacing used only for the warm-start re-anchor margin, never as the descent grid.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct FrontierDescent {
    /// Real VF-table voltage bins to probe, strictly descending (cap → hardware floor). Every entry
    /// is a voltage present in the live curve; the descent never invents a non-bin voltage.
    bins_desc: Vec<u32>,
    safe_start_mv: u32,
    voltage_step_mv: u32,
    lowest_safe_mv: u32,
}

/// Result of building a (simulated) multi-clock frontier + its synthesized profiles.
#[cfg(windows)]
#[allow(dead_code)]
struct FrontierBuildResult {
    frontier: Vec<PowerSweepPoint>,
    profiles: ForgeProfiles,
    log: Vec<String>,
}

/// Convert a stable probe at `vbin` into a frontier `PowerSweepPoint`. The deterministic
/// VF-table bin is recorded as the apply axis; measured voltage stays telemetry.
#[cfg(windows)]
#[allow(dead_code)]
fn probe_to_point(target_mhz: u32, vbin: u32, s: &ProbeSample) -> PowerSweepPoint {
    PowerSweepPoint {
        clock_mhz: s.avg_clock_mhz,
        power_w: s.power_w,
        max_power_w: s.max_power_w,
        power_capped_frac: s.power_capped_frac,
        stable: true,
        perf_per_watt: if s.power_w > 0.0 { s.avg_clock_mhz as f64 / s.power_w as f64 } else { 0.0 },
        // Prefer the actually-applied snapped bin (real probe); fall back to the descent vbin.
        vf_table_voltage_mv: s.vf_bin_mv.or(Some(vbin)),
        measured_voltage_mv: s.measured_voltage_mv,
        avg_measured_voltage_mv: s.measured_voltage_mv,
        max_measured_voltage_mv: s.measured_voltage_mv,
        p5_clock_mhz: s.p5_clock_mhz,
        voltage_quality: Some(s.voltage_quality),
        telemetry_quality: Some(s.telemetry_quality),
        // The clock we asked for (vs `clock_mhz` = measured achieved). F1b Phase 2B.1.
        target_clock_mhz: Some(target_mhz),
        ..Default::default()
    }
}

/// Pure conversion of a real dwell `Measured` into a `ProbeSample` (Phase 2B.1). This is
/// the seam the real probe closure (Phase 2B.2) will use to feed `build_frontier` — it
/// performs NO hardware I/O, only a conservative interpretation of already-collected dwell
/// data. `curve_verified` comes from the read-only offset-readback gate; `confidence` from
/// accumulated Forge Knowledge (both supplied by the future caller).
///
/// Conservative rules:
/// - A `Stable` verdict becomes `ProbeOutcome::Stable` ONLY when the telemetry is trustworthy
///   enough to believe it: clock/power quality ≥ Medium AND a sustained-clock p5 is present.
/// - Any other verdict (`SilentError`, `Crash` — which also covers a TDR / device-lost dwell
///   that returns `Measured::degenerate(Crash, …)`) or weak telemetry → `ProbeOutcome::Unstable`.
/// - `p5_clock_mhz` is preserved as the sustained-clock signal (`0` / no samples → `None`).
/// - Measured voltage uses the ramp-filtered avg and stays `None` when missing — never a fake 0.
#[cfg(windows)]
#[allow(dead_code)] // wired into the real probe closure in Phase 2B.2
fn measured_to_probe(m: &Measured, curve_verified: bool, confidence: f64) -> ProbeSample {
    let telemetry_quality = clock_power_quality(m.sample_count);
    let voltage_quality = voltage_quality(m.volt_sample_count);
    let p5_clock_mhz = (m.p5_clock_mhz > 0).then_some(m.p5_clock_mhz);
    // Telemetry-only; filtered avg. Missing voltage lowers `voltage_quality` (above) but is
    // reported as `None`, NOT 0.
    let measured_voltage_mv = m.volt_avg_mv;

    let telemetry_trustworthy =
        matches!(telemetry_quality, DwellQuality::Medium | DwellQuality::High)
            && p5_clock_mhz.is_some();
    let outcome = match m.result {
        StabilityResult::Stable if telemetry_trustworthy => ProbeOutcome::Stable,
        _ => ProbeOutcome::Unstable,
    };

    ProbeSample {
        outcome,
        curve_verified,
        avg_clock_mhz: m.clock_mhz,
        p5_clock_mhz,
        power_w: m.power_w,
        max_power_w: m.max_power_w,
        power_capped_frac: m.capped_frac,
        measured_voltage_mv,
        // The snapped applied bin is known only to the real probe (it does the apply),
        // not to this pure mapper — the probe sets it after calling `measured_to_probe`.
        vf_bin_mv: None,
        telemetry_quality,
        voltage_quality,
        confidence,
        budget_drained: false,
        aborted: false,
        crashed: false,
    }
}

// ── F1b: generic warm-start voltage-bracket carry-forward (scheduler primitive) ──────────
// Reusable for ANY ordered hardest→easiest core-clock voltage descent: a verified + dwell-stable
// bracket from a harder target seeds the next easier target's descent start, so the dominated
// high-voltage bins are not re-probed. The dwell/stability axis is treated as monotone in clock
// (a lower clock needs no more voltage); the verifier/ceiling axis is NOT (it is per-target — see
// the B2 fallback in `build_frontier`). Pure + testable; no hardware, no product (forge) naming.

/// Why a target's descent stopped. Drives carry-forward eligibility and per-target logging.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BracketStop {
    /// Reached the floor with the last probe still verified + stable.
    CleanFloor,
    /// Hit the per-target depth bound (`--max-probes-per-target`) with the last probe still verified
    /// + stable — a CLEAN, intentional stop (descended N bins by choice, no failure). Distinct from
    /// `BudgetExhausted` (global drain, non-evidence): a `PerTargetCap` bracket IS carry-forward
    /// eligible when it recorded a `lowest_verified_mv`.
    PerTargetCap,
    /// Bind-seeking (`--bind-seeking`) found the first verified + dwell-stable point where the card
    /// has LEFT the power-limited regime (`power_capped_frac <= BIND_CAP_FRAC`) and stopped the
    /// target here — a CLEAN, intentional stop, typically earlier than the per-target cap. Carry-
    /// forward eligible (it records a `lowest_verified_mv`); `is_hard_failed()` is false. Distinct
    /// from `PerTargetCap` (depth bound), `CleanFloor` (hardware floor), and `BudgetExhausted`
    /// (global drain). NOTE (F1b audit): the old clock-near-target stop arm was RETIRED — on a
    /// power-bound card it false-binds (the cap, not the descent, sets the clock), so leaving the
    /// power regime is now the only early-stop signal.
    LeftPowerRegime,
    /// Dwell instability (curve verified, but the dwell was not Stable) below the verified band.
    SoftUnstable,
    /// Ceiling did not verify/apply (LiveMismatch / overshoot_veto / monotone writer fail-closed).
    SoftUnverified,
    /// This target's dwell crashed (TDR) — the whole run aborts.
    HardFailure,
    /// `--max-probes` budget spent — a drain, not a finding.
    BudgetExhausted,
    /// A prior crash set the abort flag — this target drained without running hardware.
    Aborted,
}

/// Per-target verified-voltage bracket produced by one descent. Generic scheduler state — carries
/// NO product semantics. `lowest_verified_mv` is the ONLY value eligible to seed the next easier
/// target, and only when it was set from a verified + dwell-Stable bin (B1, enforced by the
/// descent loop which records it solely on `ProbeOutcome::Stable`).
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // several fields are diagnostic / log-only
struct TargetBracket {
    target_mhz: u32,
    highest_start_mv: u32,
    lowest_verified_mv: Option<u32>,
    first_failed_below_verified_mv: Option<u32>,
    stop_reason: BracketStop,
    bracket_source_target: Option<u32>,
    bracket_reuse_start_mv: Option<u32>,
    bracket_reuse_margin_mv: u32,
    warm_started: bool,
    fell_back_to_cap: bool,
    probes_used: u32,
}

#[cfg(windows)]
impl TargetBracket {
    /// B1: a target ending in a crash/abort must never seed the next target, even if it recorded
    /// a verified floor before the crash.
    fn is_hard_failed(&self) -> bool {
        matches!(self.stop_reason, BracketStop::HardFailure | BracketStop::Aborted)
    }
}

/// Config for warm-start bracket carry-forward. `enabled = false` reproduces the legacy
/// every-target-starts-at-the-cap behavior byte-for-byte. Derived from the run's `FrontierDescent`.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BracketCarryConfig {
    enabled: bool,
    safe_start_cap_mv: u32,
    floor_mv: u32,
    step_mv: u32,
    margin_steps: u32,
}

#[cfg(windows)]
impl BracketCarryConfig {
    fn from_descent(d: &FrontierDescent, enabled: bool, margin_steps: u32) -> Self {
        Self {
            enabled,
            safe_start_cap_mv: d.safe_start_mv,
            floor_mv: d.lowest_safe_mv,
            step_mv: d.voltage_step_mv,
            margin_steps,
        }
    }
    /// Legacy behavior: every target starts at the cap (no carry-forward). Test-only helper.
    #[cfg(test)]
    fn disabled(d: &FrontierDescent) -> Self {
        Self::from_descent(d, false, 0)
    }
    fn margin_mv(&self) -> u32 {
        self.margin_steps.saturating_mul(self.step_mv)
    }
}

/// Why `warm_start_mv` chose its start voltage (logging only).
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum WarmStartReason {
    Disabled,
    FirstTarget,
    NoBracket,
    HardFailure,
    Carried,
}

/// Result of the warm-start start-voltage decision for one target.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `reason` is diagnostic
struct WarmStartDecision {
    start_mv: u32,
    warm_started: bool,
    source_target: Option<u32>,
    reason: WarmStartReason,
}

/// Pure warm-start rule. Disabled / first target / no usable bracket / hard-failed previous →
/// start at the cap. Otherwise start at `clamp(prev.lowest_verified_mv + margin,
/// prev.lowest_verified_mv, cap)` — never below the previous verified floor (B1), never below the
/// run floor (the floor ≤ that verified bin), never above the cap. When `lv + margin` reaches the
/// cap there is nothing to skip, so it reports `warm_started = false` and starts at the cap.
#[cfg(windows)]
fn warm_start_mv(prev: Option<&TargetBracket>, cfg: &BracketCarryConfig) -> WarmStartDecision {
    let cap = cfg.safe_start_cap_mv;
    let at_cap = |reason| WarmStartDecision {
        start_mv: cap,
        warm_started: false,
        source_target: None,
        reason,
    };
    if !cfg.enabled {
        return at_cap(WarmStartReason::Disabled);
    }
    let Some(prev) = prev else {
        return at_cap(WarmStartReason::FirstTarget);
    };
    if prev.is_hard_failed() {
        return at_cap(WarmStartReason::HardFailure);
    }
    let Some(lv) = prev.lowest_verified_mv else {
        return at_cap(WarmStartReason::NoBracket);
    };
    // Lower bound is the previous verified floor (which is itself ≥ the run floor); upper bound is
    // the cap. `start ≥ lv` is the B1 guarantee.
    let lower = lv.max(cfg.floor_mv);
    let start = lv.saturating_add(cfg.margin_mv()).clamp(lower, cap);
    let warm_started = start < cap;
    WarmStartDecision {
        start_mv: start,
        warm_started,
        source_target: Some(prev.target_mhz),
        reason: if warm_started { WarmStartReason::Carried } else { WarmStartReason::NoBracket },
    }
}

// ── F1b bind-seeking (regime-only): stop a target at the first ELIGIBLE point that has LEFT the
//    power-limited regime ──────────────────────────────────────────────────────────────────────
// A target "binds" when the card is no longer pinned at the power cap (`power_capped_frac <=
// BIND_CAP_FRAC`): the ceiling/voltage — not power — is now the binding constraint, so this is a
// real, distinguishable VF point. Bind-seeking keeps descending while still power-pinned and stops
// at the first point that leaves the regime. The start bin is NEVER bind-eligible (a target must
// descend ≥1 real VF bin first). The old clock-near-target arm was RETIRED after the first hardware
// run (F1b audit): on a power-bound card the cap, not the descent, sets the achieved clock, so an
// avg-clock-near-target reading FALSE-binds. Power-bound samples are now CLASSIFIED, not bound (see
// `is_power_bound_point` / `synthesize_forge_profiles`). Regime signal ONLY — no clock or power-drop
// stop rule.

/// Power-cap fraction at or below which a probe counts as regime-binding (the card is no longer
/// power-pinned, so the ceiling/voltage — not power — is the binding constraint). Mirrors the
/// `near_power` threshold used by `classify_regime`.
#[cfg(windows)]
const BIND_CAP_FRAC: f32 = 0.5;

/// Bind-seeking thresholds (regime-only). Held in a struct so the classifier is a pure function the
/// tests can drive with explicit values; the live run always uses `v2()`.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct BindThresholds {
    /// Max power-cap fraction for regime binding (`power_capped_frac <= cap_frac` ⇒ left the regime).
    cap_frac: f32,
}

#[cfg(windows)]
impl BindThresholds {
    /// Live thresholds. The clock arm was retired (F1b audit); only the regime threshold remains.
    /// Kept named `v2()` for call-site stability (`classify_binding` + tests).
    fn v2() -> Self {
        Self { cap_frac: BIND_CAP_FRAC }
    }
}

/// Which rule bound a probe (telemetry). `None` = did not bind (or not eligible / not verified+stable).
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindReason {
    /// Did not bind.
    None,
    /// Card left the power-limited regime (`power_capped_frac <= cap_frac`) — the only stop arm.
    Regime,
}

/// Outcome of the v2 binding classifier — the decision plus the metrics it used, so the live run can
/// log WHY a probe did or did not bind. Pure data; carries no scheduler state.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct BindDecision {
    /// Bind eligibility: false on a descent's first/start bin (a target must descend ≥1 real VF bin
    /// before it may bind). A non-eligible probe never binds, whatever its metrics.
    eligible: bool,
    /// Final decision: stop this target here as a clean `LeftPowerRegime`.
    bound: bool,
    /// Which rule fired (or `None`).
    reason: BindReason,
    /// Average/achieved clock — reporting/telemetry only (the clock stop arm was retired).
    avg_clock_mhz: u32,
    /// Dip-aware sustained clock — reporting/synthesis only, never a stop signal.
    p5_clock_mhz: Option<u32>,
    /// Power-cap fraction IF valid (finite, in [0,1]); `None` when missing/invalid → no regime binding.
    power_capped_frac: Option<f32>,
}

/// A power-cap fraction is usable only when finite and in [0,1]; anything else (NaN, <0, >1) is a
/// missing/invalid metric and must fail closed (no regime binding).
#[cfg(windows)]
fn valid_cap_frac(f: f32) -> Option<f32> {
    (f.is_finite() && (0.0..=1.0).contains(&f)).then_some(f)
}

/// Bind eligibility (v2): a target must descend at least one real VF bin before it can bind, so the
/// first/start bin of a descent is NEVER bind-eligible. `probes_before` is this descent's probe count
/// BEFORE the current probe; `cur_bin_mv`/`start_bin_mv` guard the same first-bin case by voltage.
/// Pure; the earliest eligible probe is the 2nd probed bin.
#[cfg(windows)]
fn bind_eligible(probes_before: u32, cur_bin_mv: u32, start_bin_mv: u32) -> bool {
    probes_before >= 1 && cur_bin_mv != start_bin_mv
}

/// Pure binding classifier (regime-only, post-audit). A target binds — stops its descent CLEANLY as
/// `LeftPowerRegime` — only when ALL of:
///   1. ELIGIBILITY — not the start bin (`eligible`); a target must descend ≥1 real VF bin first.
///   2. SAMPLE — a verified + dwell-stable, non-drain/crash/abort probe.
///   3. REGIME — a VALID `power_capped_frac <= cap_frac` (the card left the power-limited regime);
///      a missing/invalid fraction fails closed (no bind).
/// The old clock-near-target arm was RETIRED (F1b audit): on a power-bound card it false-binds
/// because the cap, not the voltage descent, set the achieved clock. `avg_clock_mhz` / `p5_clock_mhz`
/// remain in the decision for telemetry only. Returns the full decision (with the metrics used) for
/// logging. Pure + testable; no hardware, no scheduler state.
#[cfg(windows)]
fn classify_binding(
    _target_mhz: u32,
    s: &ProbeSample,
    t: &BindThresholds,
    eligible: bool,
) -> BindDecision {
    let avg_clock_mhz = s.avg_clock_mhz;
    let p5_clock_mhz = s.p5_clock_mhz;
    let power_capped_frac = valid_cap_frac(s.power_capped_frac);

    // Only a verified + dwell-stable probe can bind. The drain/crash/abort guards are belt-and-
    // suspenders (such samples are already Unstable/unverified) but make the precondition explicit.
    let sample_bindable = s.curve_verified
        && s.outcome == ProbeOutcome::Stable
        && !s.crashed
        && !s.aborted
        && !s.budget_drained;

    // Regime rule (the ONLY stop arm): a VALID cap fraction at or below the threshold means the card
    // has left the power-limited regime. Missing/invalid → fail closed (no regime bind).
    let regime_binding = matches!(power_capped_frac, Some(f) if f <= t.cap_frac);

    let (bound, reason) = if eligible && sample_bindable && regime_binding {
        (true, BindReason::Regime)
    } else {
        (false, BindReason::None)
    };

    BindDecision { eligible, bound, reason, avg_clock_mhz, p5_clock_mhz, power_capped_frac }
}

/// Dry-run reporting lines for bind-seeking (pure, so the dry-run output is unit-testable without
/// hardware). OFF → a single "off" line that does NOT imply binding logic is active. ON → the mode
/// line plus the v1 thresholds and the live-metrics caveat. The live dry-run prints these verbatim.
#[cfg(windows)]
fn bind_seeking_plan_lines(bind_seeking: bool) -> Vec<String> {
    let mut out = vec![format!(
        "bind-seeking       : {}",
        if bind_seeking {
            "ENABLED — stop a target at the first ELIGIBLE verified+stable point that LEFT the power-limited regime, past the start bin (regime-only; clock arm retired)"
        } else {
            "off — descend by depth only (per-target cap / floor / failure)"
        }
    )];
    if bind_seeking {
        out.push(format!(
            "binding threshold  : power_capped_frac <= {:.2} (card left the power-limited regime — only stop arm)",
            BIND_CAP_FRAC
        ));
        out.push(
            "binding eligibility: start bin is NOT bind-eligible — a target must descend ≥1 real VF \
             bin first (earliest bind = 2nd probed bin)"
                .to_string(),
        );
        out.push(
            "binding note       : actual bind stop depends on live verified+dwell metrics — \
             not promised before hardware runs"
                .to_string(),
        );
    }
    out
}

/// Dry-run reporting lines for F1c power-bound knee-seeking (pure → unit-testable without hardware).
/// OFF → a single line that does NOT imply a Phase B runs. ON → the mode line, the plateau/knee
/// thresholds, the bounded deep budget, and the same-safety-envelope caveat (Phase B is supervised
/// hardware exactly like Phase A: it only descends to LOWER voltages, under the same verifier / Safe
/// Loop / reset / hardware-floor chain, and applies no profile). The live dry-run prints these verbatim.
#[cfg(windows)]
fn phase_b_plan_lines(knee_seeking: bool, budget: u32) -> Vec<String> {
    if !knee_seeking {
        return vec![
            "knee-seeking       : off — single-pass frontier (Phase A only; legacy behavior)"
                .to_string(),
        ];
    }
    vec![
        format!(
            "knee-seeking       : ENABLED (opt-in) — on a Phase-A power-bound collapse, run a focused \
             Phase-B deep descent (budget {budget} probe(s)) to cross the knee"
        ),
        format!(
            "knee thresholds    : plateau = median power-bound clock; knee = pcf crosses below {:.2}; \
             clean deep stop at pcf <= {:.2}",
            POWER_BOUND_FRAC, BIND_CAP_FRAC
        ),
        "knee budget        : global --max-probes stays the MASTER cap; --phase-b-probes only bounds \
         the focused descent depth"
            .to_string(),
        "knee start         : Phase B CONTINUES below the focus target's deepest Phase-A bin (skips \
         already-probed top bins; the budget is spent on new, deeper bins)"
            .to_string(),
        "knee note          : Phase B is supervised hardware like Phase A — descends LOWER voltages \
         only; same verifier / Safe Loop / reset / hardware-floor envelope; no profile applied"
            .to_string(),
    ]
}

/// Run ONE target's voltage descent from `start_mv` and capture its bracket. Mirrors the legacy
/// inner loop exactly (same probe sequence, same deepest-stable selection); additionally records
/// the verified floor, first failure below it, and the stop reason. The drain/crash signals on
/// `ProbeSample` are checked BEFORE the verify/outcome arms so a budget/abort/crash stop is never
/// mistaken for a plain verify failure (only the latter is B2-fallback-eligible). When
/// `bind_seeking` is set, a verified + dwell-stable probe that is the first to LEAVE the power-limited
/// regime stops the descent CLEANLY here (`BracketStop::LeftPowerRegime`), earlier than the per-target
/// cap — checked only after the failure/drain arms, so a failure always takes precedence. `bind_seeking=false`
/// reproduces the legacy descent byte-for-byte. The `probe` closure is the only seam to hardware;
/// when `bind_seeking` is on, each verified+stable probe additionally emits a `tracing` bind-seeking
/// telemetry line (eligibility + rule + metrics) — return values stay deterministic and testable.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn descend_target(
    target: u32,
    start_mv: u32,
    descent: &FrontierDescent,
    max_per_target: Option<u32>,
    bind_seeking: bool,
    probe: &impl Fn(u32, u32) -> ProbeSample,
) -> (TargetBracket, Option<(PowerSweepPoint, f64)>) {
    let mut bracket = TargetBracket {
        target_mhz: target,
        highest_start_mv: start_mv,
        lowest_verified_mv: None,
        first_failed_below_verified_mv: None,
        stop_reason: BracketStop::CleanFloor,
        bracket_source_target: None,
        bracket_reuse_start_mv: None,
        bracket_reuse_margin_mv: 0,
        warm_started: false,
        fell_back_to_cap: false,
        probes_used: 0,
    };
    let mut deepest: Option<(PowerSweepPoint, f64)> = None;
    // Bin-based descent. Snap the requested start voltage UP to the lowest real bin ≥ `start_mv`
    // (the CONSERVATIVE re-anchor — same `nearest_vf_bin_at_or_above` direction the hardware apply
    // uses), then walk strictly down through every lower real bin to the hardware floor
    // (`bins_desc.last()`). Because `start_mv ≥ prev.lowest_verified_mv` (itself a real bin) and the
    // start bin is ≥ `start_mv`, the descent never starts below the previous verified floor (B1).
    // When the margin target lands between two bins it re-anchors at the higher (safer) one; an
    // exact-bin `start_mv` (e.g. the cap, or the legacy step grid) resolves to itself. No invented
    // voltages — only real curve bins are ever probed.
    let Some(start_bin) = descent
        .bins_desc
        .iter()
        .copied()
        .filter(|&b| b >= start_mv)
        .min()
        .or_else(|| descent.bins_desc.first().copied())
    else {
        return (bracket, deepest); // empty bin domain — caller fails closed before any hardware
    };
    for &v in descent.bins_desc.iter().filter(|&&b| b <= start_bin) {
        // Per-target depth cap: stop CLEANLY once N probes have run for this target, BEFORE probing
        // the next (deeper) bin. This arm is only reached when the prior N probes were all
        // verified + stable — any failure / drain / crash breaks earlier with its own stop reason
        // and takes precedence — so `PerTargetCap` is genuinely clean (carry-forward eligible when a
        // verified floor was recorded). No effect when `None` (legacy full descent).
        if let Some(n) = max_per_target {
            if bracket.probes_used >= n {
                bracket.stop_reason = BracketStop::PerTargetCap;
                break;
            }
        }
        let s = probe(target, v);
        bracket.probes_used += 1;
        // Drain / hard-failure first: these stop the descent and are NOT verify failures.
        if s.crashed {
            bracket.stop_reason = BracketStop::HardFailure;
            break;
        }
        if s.aborted {
            bracket.stop_reason = BracketStop::Aborted;
            break;
        }
        if s.budget_drained {
            bracket.stop_reason = BracketStop::BudgetExhausted;
            break;
        }
        if !s.curve_verified {
            bracket.stop_reason = BracketStop::SoftUnverified;
            bracket.first_failed_below_verified_mv = Some(v);
            break;
        }
        match s.outcome {
            ProbeOutcome::Stable => {
                // B1: a verified floor is recorded ONLY from a verified + dwell-Stable bin.
                deepest = Some((probe_to_point(target, v, &s), s.confidence));
                bracket.lowest_verified_mv = Some(v);
                // Bind-seeking (opt-in), regime-only: a target must descend ≥1 real VF bin before it
                // may bind (the start bin is never eligible). If this verified + dwell-stable probe is
                // the first ELIGIBLE point that has LEFT the power-limited regime (power_capped_frac <=
                // BIND_CAP_FRAC), stop the target CLEANLY here — earlier than the per-target cap / floor
                // — keeping THIS point. The old clock-near-target arm was retired (it false-binds on a
                // power-bound card). Reached only after the crash/abort/budget/unverified/unstable arms
                // above, so a failure always takes precedence over binding. No-op when off.
                if bind_seeking {
                    // `probes_used` was incremented above, so `- 1` is this probe's predecessors.
                    let eligible = bind_eligible(bracket.probes_used - 1, v, start_bin);
                    let decision = classify_binding(target, &s, &BindThresholds::v2(), eligible);
                    // Telemetry (live-run only): surface eligibility, the rule, and the metrics used.
                    info!(
                        "build-frontier bind-seeking: target={target} bin_mv={v} eligible={} \
                         bound={} reason={:?} avg_clock_mhz={} p5_clock_mhz={:?} power_capped_frac={}",
                        decision.eligible, decision.bound, decision.reason, decision.avg_clock_mhz,
                        decision.p5_clock_mhz,
                        decision
                            .power_capped_frac
                            .map(|f| format!("{f:.3}"))
                            .unwrap_or_else(|| "n/a".to_string()),
                    );
                    if decision.bound {
                        bracket.stop_reason = BracketStop::LeftPowerRegime;
                        break;
                    }
                }
                // Otherwise continue to the next lower real bin (loop ends at the hardware floor).
            }
            ProbeOutcome::Unstable => {
                bracket.stop_reason = BracketStop::SoftUnstable;
                bracket.first_failed_below_verified_mv = Some(v);
                break;
            }
        }
    }
    (bracket, deepest)
}

/// Build a multi-clock frontier by descending each candidate clock's voltage bins via the injected
/// `probe` closure, then synthesize the three profiles with `policy`.
///
/// Per target: pick the descent start via `warm_start_mv` (legacy `safe_start_cap` when carry is
/// disabled or no bracket can be reused), descend with `descend_target`, then carry the resulting
/// verified bracket forward. **B2** — because the verifier/ceiling axis is NOT monotone in clock,
/// a warm-started first probe that fails to apply/verify (and produced no verified bin) falls back
/// ONCE to the cap and descends normally; it is never dropped, and the fallback never fires on a
/// drain (budget/abort) or a crash. Outer loop allows a partial frontier, then synthesizes. Pure —
/// the closure is the only seam to (future) hardware. Never runs stress / writes the VF curve.
///
/// `max_per_target` (`--max-probes-per-target`) bounds each target's descent depth so one target
/// cannot drain the whole global budget (the F1b multi-clock coverage fix). It is passed to BOTH
/// the normal and the B2-fallback descents. `bind_seeking` (`--bind-seeking`, opt-in) lets a target
/// stop at the first verified+stable BINDING point (earlier than the cap); `false` preserves the
/// legacy depth-bounded descent. It too is passed to both the normal and B2-fallback descents. NOTE: with warm-start ON, a B2 fallback re-descends
/// from the cap, so a single target may run up to two capped descent attempts (≤ 2·N probes); this
/// is acceptable because warm-start is opt-in and the global `--max-probes` cap still bounds the run.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn build_frontier(
    candidate_clocks: &[u32],
    descent: &FrontierDescent,
    policy: &ForgePolicy,
    carry: &BracketCarryConfig,
    max_per_target: Option<u32>,
    bind_seeking: bool,
    probe: impl Fn(u32, u32) -> ProbeSample,
) -> FrontierBuildResult {
    let (paired, log) =
        run_target_descents(candidate_clocks, descent, carry, max_per_target, bind_seeking, &probe);
    let profiles = synthesize_forge_profiles(&paired, policy);
    FrontierBuildResult {
        frontier: paired.into_iter().map(|(p, _)| p).collect(),
        profiles,
        log,
    }
}

/// Phase-A core: run every candidate target's bounded warm-start descent and collect the
/// `(point, confidence)` pairs plus the per-target decision log. Extracted VERBATIM from the original
/// `build_frontier` body so the single-pass path is byte-for-byte unchanged; the F1c two-phase
/// orchestrator reuses it (and the dropped confidence) for Phase A. Pure — the closure is the only
/// seam to (future) hardware.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn run_target_descents(
    candidate_clocks: &[u32],
    descent: &FrontierDescent,
    carry: &BracketCarryConfig,
    max_per_target: Option<u32>,
    bind_seeking: bool,
    probe: &impl Fn(u32, u32) -> ProbeSample,
) -> (Vec<(PowerSweepPoint, f64)>, Vec<String>) {
    let mut paired: Vec<(PowerSweepPoint, f64)> = Vec::new();
    let mut log = Vec::new();
    let mut prev: Option<TargetBracket> = None;

    for &target in candidate_clocks {
        let decision = warm_start_mv(prev.as_ref(), carry);
        let (mut bracket, mut point) =
            descend_target(target, decision.start_mv, descent, max_per_target, bind_seeking, probe);
        bracket.warm_started = decision.warm_started;
        bracket.bracket_source_target = decision.source_target;
        bracket.bracket_reuse_start_mv = decision.warm_started.then_some(decision.start_mv);
        bracket.bracket_reuse_margin_mv = if decision.warm_started { carry.margin_mv() } else { 0 };

        // B2: a warm-started first probe that failed to apply/verify (no verified bin, stop is
        // SoftUnverified — NOT a drain, crash, or dwell instability) must fall back ONCE to the cap
        // and descend normally. The verify axis can need MORE voltage than a harder target did.
        if decision.warm_started
            && bracket.lowest_verified_mv.is_none()
            && bracket.stop_reason == BracketStop::SoftUnverified
        {
            log.push(format!(
                "bracket_carry target={target} warm_start_verify_failed at={} mV → fallback safe_start_cap={} mV",
                decision.start_mv, carry.safe_start_cap_mv
            ));
            let warm_probes = bracket.probes_used;
            let (mut fb, fb_point) =
                descend_target(target, carry.safe_start_cap_mv, descent, max_per_target, bind_seeking, probe);
            fb.bracket_source_target = decision.source_target;
            fb.fell_back_to_cap = true;
            fb.probes_used = fb.probes_used.saturating_add(warm_probes);
            bracket = fb;
            point = fb_point;
        }

        // Legacy-compatible per-descent reason line (unchanged phrasing) for the stop case.
        match bracket.stop_reason {
            BracketStop::SoftUnverified => log.push(format!(
                "{target} MHz @ {} mV: curve not verified — stop descent",
                bracket.first_failed_below_verified_mv.unwrap_or(bracket.highest_start_mv)
            )),
            BracketStop::SoftUnstable => log.push(format!(
                "{target} MHz @ {} mV: unstable — keep deepest stable",
                bracket.first_failed_below_verified_mv.unwrap_or(bracket.highest_start_mv)
            )),
            _ => {}
        }

        // One compact, deterministic decision line per target.
        log.push(format!(
            "bracket_carry enabled={} target={} source_target={:?} start_mv={} safe_start_cap={} \
             margin_mv={} lowest_verified_mv={:?} first_failed_mv={:?} stop_reason={:?} \
             warm_started={} fell_back_to_cap={} probes_used={}",
            carry.enabled, target, bracket.bracket_source_target, bracket.highest_start_mv,
            carry.safe_start_cap_mv, bracket.bracket_reuse_margin_mv, bracket.lowest_verified_mv,
            bracket.first_failed_below_verified_mv, bracket.stop_reason, bracket.warm_started,
            bracket.fell_back_to_cap, bracket.probes_used
        ));

        match point {
            Some(p) => paired.push(p),
            None => log.push(format!("{target} MHz: no stable point in safe range — dropped")),
        }
        prev = Some(bracket);
    }

    (paired, log)
}

/// One Phase-B deep knee-seeking descent: every verified + dwell-stable bin probed for the focused
/// target, in descending-voltage order, plus where/why it stopped. Pure scheduler data — no hardware,
/// no product naming. Unlike `descend_target` (which keeps only the deepest stable point), Phase B
/// keeps the FULL trajectory so the knee (the pcf transition) can be detected.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
struct PhaseBTrajectory {
    points: Vec<(PowerSweepPoint, f64)>,
    stop_reason: BracketStop,
    probes_used: u32,
    log: Vec<String>,
}

/// Run ONE focused target's DEEP voltage descent, recording every verified + dwell-stable bin so the
/// knee can be detected from the pcf trajectory. Mirrors `descend_target`'s stop precedence exactly
/// (crash → abort → budget drain → verifier failure → dwell instability), and descends THROUGH the
/// knee (a pcf drop below `POWER_BOUND_FRAC` is recorded, not a stop) so the below-knee efficiency
/// tail is captured; it stops CLEANLY only once the card is clearly off the cap (left the power-limited
/// regime, `pcf <= BIND_CAP_FRAC`), at the per-target `budget`, or at the hardware floor. The global
/// `--max-probes` (enforced by the probe closure via `budget_drained`) remains the master cap. Pure;
/// the closure is the only seam to hardware. Never writes the VF curve / runs stress itself.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn descend_phase_b(
    target: u32,
    start_mv: u32,
    descent: &FrontierDescent,
    budget: u32,
    probe: &impl Fn(u32, u32) -> ProbeSample,
) -> PhaseBTrajectory {
    let mut points: Vec<(PowerSweepPoint, f64)> = Vec::new();
    let mut probes_used = 0u32;
    let mut log = Vec::new();

    // Snap the start UP to the lowest real bin ≥ `start_mv` (same conservative re-anchor the apply
    // uses), then walk strictly down through every lower real bin. Only real curve bins are probed.
    let Some(start_bin) = descent
        .bins_desc
        .iter()
        .copied()
        .filter(|&b| b >= start_mv)
        .min()
        .or_else(|| descent.bins_desc.first().copied())
    else {
        log.push("phase-b descent: empty bin domain — fail closed (no hardware)".to_string());
        return PhaseBTrajectory {
            points,
            stop_reason: BracketStop::SoftUnverified,
            probes_used,
            log,
        };
    };

    let mut stop_reason = BracketStop::CleanFloor;
    for &v in descent.bins_desc.iter().filter(|&&b| b <= start_bin) {
        if probes_used >= budget {
            stop_reason = BracketStop::PerTargetCap;
            break;
        }
        let s = probe(target, v);
        probes_used += 1;
        // Drain / hard-failure first — same precedence as `descend_target`; never a verify failure.
        if s.crashed {
            stop_reason = BracketStop::HardFailure;
            break;
        }
        if s.aborted {
            stop_reason = BracketStop::Aborted;
            break;
        }
        if s.budget_drained {
            stop_reason = BracketStop::BudgetExhausted;
            break;
        }
        if !s.curve_verified {
            stop_reason = BracketStop::SoftUnverified;
            break;
        }
        match s.outcome {
            ProbeOutcome::Stable => {
                points.push((probe_to_point(target, v, &s), s.confidence));
                // Descend THROUGH the knee (pcf crossing POWER_BOUND_FRAC is recorded, not a stop) to
                // build the below-knee tail; stop CLEANLY only once clearly off the cap.
                if matches!(valid_cap_frac(s.power_capped_frac), Some(f) if f <= BIND_CAP_FRAC) {
                    stop_reason = BracketStop::LeftPowerRegime;
                    break;
                }
            }
            ProbeOutcome::Unstable => {
                stop_reason = BracketStop::SoftUnstable;
                break;
            }
        }
    }
    log.push(format!(
        "phase-b descent: target={target} start_bin={start_bin} mV probes_used={probes_used} \
         stable_points={} stop={stop_reason:?}",
        points.len()
    ));
    PhaseBTrajectory { points, stop_reason, probes_used, log }
}

/// Result of a two-phase (F1c) frontier build. `result` is the same `FrontierBuildResult` the live run
/// consumes (so reporting is unchanged); the rest is Phase-B telemetry for tests/logging.
#[cfg(windows)]
#[allow(dead_code)] // fields are diagnostic / test-facing
struct TwoPhaseFrontier {
    result: FrontierBuildResult,
    phase_b_ran: bool,
    plateau_clock: Option<u32>,
    focus_target: Option<u32>,
    knee_index: Option<usize>,
    phase_b_points: usize,
    phase_b_probes_used: u32,
}

/// Two-phase power-bound knee-seeking frontier build (F1c, opt-in). **Phase A** is the existing
/// single-pass `build_frontier` descent (broad, shallow). When `phase_b_budget` is `None`, OR Phase A
/// already produced a differentiated frontier, the result is byte-for-byte the `build_frontier`
/// output. Otherwise — Phase A collapsed power-bound AND a budget is given — **Phase B** detects the
/// plateau, selects ONE focused target near/above it, and descends that target DEEPER (recording the
/// full trajectory) to cross the knee. Phase A + Phase B points are merged and re-synthesized by the
/// SAME `synthesize_forge_profiles`: if Phase B crossed the knee, the now-present below-knee useful
/// points let it differentiate (Godforge = highest sustained off-cap clock = the knee region); if
/// Phase B never left saturation, the merge stays collapsed and the honest `PowerBoundCollapse` stands.
/// Pure — the closure is the only seam to hardware; never writes the VF curve / runs stress itself.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn build_frontier_two_phase(
    candidate_clocks: &[u32],
    descent: &FrontierDescent,
    policy: &ForgePolicy,
    carry: &BracketCarryConfig,
    max_per_target: Option<u32>,
    bind_seeking: bool,
    phase_b_budget: Option<u32>,
    probe: impl Fn(u32, u32) -> ProbeSample,
) -> TwoPhaseFrontier {
    let (paired_a, mut log) =
        run_target_descents(candidate_clocks, descent, carry, max_per_target, bind_seeking, &probe);
    let profiles_a = synthesize_forge_profiles(&paired_a, policy);

    // Build a Phase-A-only result identical to `build_frontier`'s output (no Phase B ran).
    let phase_a_only =
        |paired: Vec<(PowerSweepPoint, f64)>, profiles: ForgeProfiles, log: Vec<String>| TwoPhaseFrontier {
            result: FrontierBuildResult {
                frontier: paired.into_iter().map(|(p, _)| p).collect(),
                profiles,
                log,
            },
            phase_b_ran: false,
            plateau_clock: None,
            focus_target: None,
            knee_index: None,
            phase_b_points: 0,
            phase_b_probes_used: 0,
        };

    // OFF → byte-for-byte single-pass (build_frontier) behavior.
    let Some(budget) = phase_b_budget else {
        return phase_a_only(paired_a, profiles_a, log);
    };
    // Phase A already differentiated → nothing for Phase B to do.
    if !profiles_a.power_bound_collapse {
        log.push(
            "PHASE-B: skipped — Phase A produced a differentiated frontier (no power-bound collapse)"
                .to_string(),
        );
        return phase_a_only(paired_a, profiles_a, log);
    }
    // Collapse → knee-seek. Detect the plateau, pick a focused target, descend it deep.
    let Some(plateau_clock) = detect_plateau_clock(&paired_a) else {
        log.push(
            "PHASE-B: skipped — collapse but no robust power-bound plateau (need >= 2 power-bound points)"
                .to_string(),
        );
        return phase_a_only(paired_a, profiles_a, log);
    };
    let Some(focus_target) = select_phase_b_target(candidate_clocks, plateau_clock) else {
        log.push(format!("PHASE-B: skipped — no candidate target for plateau ~{plateau_clock} MHz"));
        return phase_a_only(paired_a, profiles_a, log);
    };
    log.push(format!(
        "PHASE-B: power-bound collapse → deep knee-seeking descent target={focus_target} MHz \
         (plateau ~{plateau_clock} MHz, budget {budget} probe(s))"
    ));

    // Budget efficiency (F1c follow-up): CONTINUE the focused descent BELOW the deepest bin Phase A
    // already explored for this target, rather than re-probing the inert top bins. On fine-grained VF
    // curves this spends every Phase-B probe on new, deeper bins. No retained Phase-A point for the
    // target (not probed / dropped) → fall back to the cap; Phase A already at the floor → nothing
    // deeper, so Phase B returns cleanly.
    let phase_b_start_mv = match phase_a_deepest_bin(&paired_a, focus_target) {
        Some(floor) => match phase_b_start_below(descent, floor) {
            Some(below) => {
                log.push(format!(
                    "PHASE-B: focus {focus_target} MHz explored to {floor} mV in Phase A → start Phase B \
                     below, at {below} mV (skip already-probed bins)"
                ));
                below
            }
            None => {
                log.push(format!(
                    "PHASE-B: skipped — focus {focus_target} MHz already reached the hardware floor \
                     ({floor} mV) in Phase A; no deeper real bin"
                ));
                return phase_a_only(paired_a, profiles_a, log);
            }
        },
        None => {
            log.push(format!(
                "PHASE-B: focus {focus_target} MHz has no retained Phase-A point → start at \
                 safe_start_cap {} mV (fallback)",
                carry.safe_start_cap_mv
            ));
            carry.safe_start_cap_mv
        }
    };

    let traj = descend_phase_b(focus_target, phase_b_start_mv, descent, budget, &probe);
    let knee_index = detect_power_bound_knee(&traj.points);
    for l in &traj.log {
        log.push(l.clone());
    }
    log.push(format!(
        "PHASE-B: target={focus_target} probes_used={} stop={:?} stable_points={} knee={}",
        traj.probes_used,
        traj.stop_reason,
        traj.points.len(),
        knee_index
            .map(|i| format!("crossed@idx{i}"))
            .unwrap_or_else(|| "not-found (still saturated — collapse stands)".to_string()),
    ));

    // Merge Phase A + Phase B points and re-synthesize. The existing synthesis EXCLUDES power-bound
    // points and differentiates over the (now-present) below-knee useful tail; with no knee, the merge
    // stays collapsed and the honest refusal is preserved.
    let phase_b_points = traj.points.len();
    let mut merged = paired_a;
    merged.extend(traj.points);
    let profiles_ab = synthesize_forge_profiles(&merged, policy);

    TwoPhaseFrontier {
        result: FrontierBuildResult {
            frontier: merged.into_iter().map(|(p, _)| p).collect(),
            profiles: profiles_ab,
            log,
        },
        phase_b_ran: true,
        plateau_clock: Some(plateau_clock),
        focus_target: Some(focus_target),
        knee_index,
        phase_b_points,
        phase_b_probes_used: traj.probes_used,
    }
}

/// Estimated non-dwell overhead per probe (apply + read-only verify + fresh GpuCtx
/// create/drop), for the dry-run wall-time estimate only. Conservative.
#[cfg(windows)]
#[allow(dead_code)] // used by `plan_frontier` (wired into the dry-run in Phase 2B.2-b)
const PROBE_OVERHEAD_MS: u64 = 5_000;

/// Derive a BIN-BASED `FrontierDescent` from the GPU's real graphics-core VF voltage bins.
/// `core_bins_mv` are the actual stock-curve core voltages (the seeded cluster's bins); `cap_mv`
/// is the effective descent start (the cluster top, lowered by any `--safe-start-cap`). The
/// descent keeps only real bins in `[floor..=cap]`, strictly descending, where the HARDWARE-DERIVED
/// floor = the lowest real core bin (`bins_desc.last()`). The lower bound is discovered from the
/// curve, never a hardcoded voltage; `step_mv` is retained only as the warm-start re-anchor margin
/// unit + dry-run label. An empty result (no real bin ≤ cap) signals the caller to FAIL CLOSED —
/// the supervised run must not descend an invented grid. Pure + testable; no hardware here.
#[cfg(windows)]
#[allow(dead_code)] // wired into the dry-run / supervised run in Phase 2B.2-b
fn derive_descent(core_bins_mv: &[u32], cap_mv: u32, step_mv: u32) -> FrontierDescent {
    let mut bins: Vec<u32> = core_bins_mv.iter().copied().filter(|&v| v <= cap_mv).collect();
    bins.sort_unstable();
    bins.dedup();
    // Hardware-derived floor + start: the lowest / highest real bin within the cap. Degenerate
    // (no bin ≤ cap) → empty `bins_desc`, floor = start = cap, and the caller fails closed.
    let lowest_safe_mv = bins.first().copied().unwrap_or(cap_mv);
    let safe_start_mv = bins.last().copied().unwrap_or(cap_mv);
    bins.reverse(); // strictly descending: cap → hardware floor
    FrontierDescent {
        bins_desc: bins,
        safe_start_mv,
        voltage_step_mv: step_mv.max(1),
        lowest_safe_mv,
    }
}

/// A read-only dry-run plan: what a supervised `build-frontier` run WOULD do, with worst-case
/// (no-early-stop) dwell-count and wall-time estimates. Computed purely; never touches hardware.
#[cfg(windows)]
#[allow(dead_code)] // surfaced by the dry-run path in Phase 2B.2-b
#[derive(Debug, Clone, PartialEq)]
struct FrontierPlan {
    targets: Vec<u32>,
    safe_start_mv: u32,
    lowest_safe_mv: u32,
    voltage_step_mv: u32,
    /// The real VF-table voltage bins the descent will probe (cap → hardware floor, descending).
    /// Every entry exists in the live curve; surfaced in the dry-run so the operator reviews the
    /// exact sequence before `--confirm`.
    descent_bins: Vec<u32>,
    bins_per_descent: u32,
    /// Per-target depth bound in effect (`--max-probes-per-target`), or `None` for full descent.
    max_probes_per_target: Option<u32>,
    /// Bins each target actually descends this run = `min(bins_per_descent, max_probes_per_target)`.
    /// Equals `bins_per_descent` when no per-target cap is set.
    effective_bins_per_descent: u32,
    est_dwell_count: u32,
    est_wall_secs: u64,
    safety_notice: String,
}

/// Compute the dry-run plan from candidate `targets` + a `FrontierDescent`. `est_dwell_count`
/// is the WORST case (every target descends its effective bin budget with no early stop); real runs
/// stop earlier on first instability. `max_per_target` (`--max-probes-per-target`) caps the bins one
/// target descends, so the worst case becomes `targets × min(bins, cap)` instead of `targets ×
/// bins`. The GLOBAL `--max-probes` cap is applied by the caller (it bounds the run-wide total).
/// Pure + testable.
#[cfg(windows)]
#[allow(dead_code)] // surfaced by the dry-run path in Phase 2B.2-b
fn plan_frontier(
    targets: Vec<u32>,
    descent: &FrontierDescent,
    dwell_ms: u64,
    max_per_target: Option<u32>,
) -> FrontierPlan {
    let step = descent.voltage_step_mv.max(1);
    // Worst case = every target descends every REAL bin (no early stop). Bin-based: the count is
    // the number of actual VF bins in range, not a step-grid span.
    let bins_per_descent = descent.bins_desc.len() as u32;
    // Per-target cap only ever REDUCES the bins a target descends (never widens exposure).
    let effective_bins_per_descent =
        max_per_target.map_or(bins_per_descent, |n| bins_per_descent.min(n));
    let est_dwell_count = targets.len() as u32 * effective_bins_per_descent;
    let est_wall_secs = est_dwell_count as u64 * (dwell_ms + PROBE_OVERHEAD_MS) / 1000;
    FrontierPlan {
        targets,
        safe_start_mv: descent.safe_start_mv,
        lowest_safe_mv: descent.lowest_safe_mv,
        voltage_step_mv: step,
        descent_bins: descent.bins_desc.clone(),
        bins_per_descent,
        max_probes_per_target: max_per_target,
        effective_bins_per_descent,
        est_dwell_count,
        est_wall_secs,
        safety_notice: "SUPERVISED ONLY: applies transient VF ceilings and runs game-power \
            dwells that can TDR or hard-reboot. Operator must be present and able to reboot. \
            Never probes below the known crash floor. No profile is applied or persisted by \
            this run."
            .to_string(),
    }
}

// ── F1b Phase 2B.2-b.2: real probe closure + supervised `build-frontier` entry ──────
// First-version, CONSERVATIVE seeding/limits. These are operator-tunable and should be
// reviewed against the printed dry-run plan before any supervised `--confirm` run.
/// GetStatus tolerance for the per-probe curve verify (~one boost bin).
#[cfg(windows)]
const FRONTIER_VERIFY_TOL_MHZ: u32 = 15;
/// Per-probe stability confidence for the FIRST run (single-trial Wilson LB). Low → synthesis
/// falls back to best-effort/V1; it matures across runs (V3). No knowledge writes here.
#[cfg(windows)]
const FRONTIER_PROBE_CONFIDENCE: f64 = 0.21;
/// Nominal voltage spacing (mV). Used ONLY for the warm-start re-anchor margin and the dry-run
/// label — NOT as the descent grid. The descent is BIN-BASED: it walks the GPU's real VF-table
/// voltage bins (see [`derive_descent`]), so it never requests a voltage that is not a real,
/// writable bin. The lower bound is the HARDWARE-DERIVED floor (the lowest real graphics-core VF
/// bin from the seeded cluster), not a hardcoded voltage: discovery descends naturally until the
/// floor bin, a verifier/dwell failure, or a TDR/crash/abort, whichever comes first. The per-probe
/// Safe Loop + verifier + reset remain the safety net (the old artificial 875 mV floor did not).
#[cfg(windows)]
const FRONTIER_VOLT_STEP_MV: u32 = 25;
/// Upward safety margin (in voltage steps) added when an easier target reuses a harder target's
/// verified bracket: it starts at `lowest_verified_mv + N*step` so the first warm probe re-anchors
/// at a dominated-safe bin before descending further. One step (25 mV) by default.
#[cfg(windows)]
const FRONTIER_WARM_START_MARGIN_STEPS: u32 = 1;
/// Target-clock spacing (MHz) ~ two boost bins.
#[cfg(windows)]
const FRONTIER_CLOCK_STEP_MHZ: u32 = 30;
/// Deep Calm clock floor fraction for candidate clocks.
#[cfg(windows)]
const FRONTIER_FLOOR_FRAC: f64 = 0.90;
/// Default Phase-B deep-descent budget (probes on the focused knee-seeking target) when
/// `--power-bound-knee-seeking` is on and `--phase-b-probes` is unset. Bounded and conservative; the
/// global `--max-probes` remains the master cap. ~4× the shallow `--max-probes-per-target 3` that
/// confined the validated collapse run to the top ~13 mV — enough to descend past the operating-voltage
/// knee on a power-bound card, still small.
#[cfg(windows)]
const FRONTIER_PHASE_B_PROBES: u32 = 12;

// ── Phase 2B.2-b.3: graphics-core SANITY-DOMAIN guards ──────────────────────────────
// NOT tuning targets — only safety guards to reject non-core / memory-domain / implausible
// VF points so seeding can never derive a target like 7001 MHz or a safe_start like 1237 mV.
// A future GPU outside these bounds should FAIL CLOSED and prompt a code update, not run.
#[cfg(windows)]
const CORE_FREQ_MIN_MHZ: u32 = 500;
#[cfg(windows)]
const CORE_FREQ_SOFT_WARN_MHZ: u32 = 3200;
#[cfg(windows)]
const CORE_FREQ_HARD_MAX_MHZ: u32 = 3500;
#[cfg(windows)]
const CORE_VF_MIN_MV: u32 = 600;
#[cfg(windows)]
const CORE_VF_SOFT_MAX_MV: u32 = 1125;
#[cfg(windows)]
const CORE_VF_HARD_MAX_MV: u32 = 1150;
/// Max voltage gap (mV) between consecutive points within ONE contiguous core VF cluster;
/// a larger gap marks a domain / outlier boundary (Phase 2B.2-b.4).
#[cfg(windows)]
const CORE_CLUSTER_GAP_MV: u32 = 60;
/// Minimum points for a cluster to be trusted as the stock core VF curve (else fail closed).
#[cfg(windows)]
const MIN_CORE_CLUSTER_POINTS: usize = 8;

/// First-run limiter flags for the supervised `build-frontier` run (Phase 2B.2-c.0). All
/// optional; `None` preserves the full default plan. Validated by `validate_limits`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrontierLimits {
    pub max_targets: Option<usize>,
    pub max_probes: Option<u32>,
    /// Per-target depth bound (`--max-probes-per-target`): the max hardware probes ONE target's
    /// descent may spend before stopping cleanly (before probing the next bin). `None` preserves
    /// the legacy depth-first descent. It can only REDUCE / redistribute exposure — the global
    /// `max_probes` stays the hard cap. Prevents one power-limited target from draining the whole
    /// budget so F1b collects multi-clock coverage. Default `None`.
    pub max_probes_per_target: Option<u32>,
    pub safe_start_cap_mv: Option<u32>,
    /// Opt-in (`--warm-start-brackets`): warm-start voltage-bracket carry-forward. Default `false`
    /// → legacy behavior (every target starts at the cap). First adopter: build-frontier / F1b.
    pub warm_start_brackets: bool,
    /// Opt-in (`--bind-seeking`): F1b bind-seeking v1. When set, each target's descent stops at the
    /// first verified + dwell-stable BINDING point (sustained clock near target OR the card left the
    /// power-limited regime) instead of walking a fixed number of bins. Default `false` preserves
    /// the current per-target-cap / depth-first behavior exactly. The global `max_probes` and the
    /// per-target cap remain the bounding limits; binding can only stop a target EARLIER.
    pub bind_seeking: bool,
    /// Opt-in (`--power-bound-knee-seeking`): F1c two-phase knee-seeking. When set, after a Phase-A
    /// power-bound collapse the run does a focused Phase-B deep descent on one target near/above the
    /// observed plateau to cross the knee (the lowest ceiling that still sustains the power-limited
    /// clock) and capture the below-knee efficiency tail. Default `false` → single-pass behavior,
    /// identical to the legacy frontier build.
    pub power_bound_knee_seeking: bool,
    /// Phase-B deep-descent budget (`--phase-b-probes`): max probes the focused knee-seeking descent
    /// may spend. `None` with knee-seeking ON uses `FRONTIER_PHASE_B_PROBES`. The global `--max-probes`
    /// (enforced by the probe closure) stays the master cap; this only bounds the focused descent's
    /// depth — it can never raise the global ceiling. Default `None`.
    pub phase_b_probes: Option<u32>,
}

/// Semantic validation of the limiter flags — FAIL CLOSED on absurd values. Pure + testable.
#[cfg(windows)]
fn validate_limits(l: &FrontierLimits, lowest_safe_mv: u32) -> Result<(), String> {
    if l.max_targets == Some(0) {
        return Err("--max-targets must be >= 1".into());
    }
    if l.max_probes == Some(0) {
        return Err("--max-probes must be >= 1".into());
    }
    if l.max_probes_per_target == Some(0) {
        return Err("--max-probes-per-target must be >= 1".into());
    }
    if l.phase_b_probes == Some(0) {
        return Err("--phase-b-probes must be >= 1".into());
    }
    if let Some(cap) = l.safe_start_cap_mv {
        if cap <= lowest_safe_mv {
            return Err(format!(
                "--safe-start-cap {cap} mV must be > the crash floor {lowest_safe_mv} mV"
            ));
        }
    }
    Ok(())
}

/// Apply `--max-targets` (truncate to the top N) and `--safe-start-cap` (lower the descent start
/// to the cap when it is below the derived cluster top, never below the floor, never raising it).
/// Returns the effective `(targets, safe_start_mv)`. Pure + testable.
#[cfg(windows)]
fn apply_frontier_limits(
    mut targets: Vec<u32>,
    derived_safe_start_mv: u32,
    lowest_safe_mv: u32,
    limits: &FrontierLimits,
) -> (Vec<u32>, u32) {
    if let Some(n) = limits.max_targets {
        targets.truncate(n);
    }
    let mut safe_start = derived_safe_start_mv;
    if let Some(cap) = limits.safe_start_cap_mv {
        if cap < safe_start {
            safe_start = cap.max(lowest_safe_mv); // never below the floor; never raise above derived
        }
    }
    (targets, safe_start)
}

/// Build the voltage soft-max warning, distinguishing the derived stock curve top from the
/// effective (post `--safe-start-cap`) descent start. `None` when the curve top is within the
/// soft max. Pure + testable.
#[cfg(windows)]
fn soft_max_voltage_warning(
    curve_top_mv: u32,
    effective_safe_start_mv: u32,
    soft_max_mv: u32,
) -> Option<String> {
    if curve_top_mv <= soft_max_mv {
        return None;
    }
    Some(if effective_safe_start_mv < curve_top_mv {
        format!(
            "curve top {curve_top_mv} mV exceeds soft max {soft_max_mv} mV; descent capped to {effective_safe_start_mv} mV"
        )
    } else {
        format!("curve top {curve_top_mv} mV exceeds soft max {soft_max_mv} mV")
    })
}

/// True iff a VF point is a plausible graphics-core point (within the sanity domain). Pure.
#[cfg(windows)]
fn is_sane_core_point(voltage_mv: u32, freq_mhz: u32) -> bool {
    (CORE_FREQ_MIN_MHZ..=CORE_FREQ_HARD_MAX_MHZ).contains(&freq_mhz)
        && (CORE_VF_MIN_MV..=CORE_VF_HARD_MAX_MV).contains(&voltage_mv)
}

/// Keep only plausible graphics-core VF points from a raw `read_vf_curve_modern` read,
/// discarding non-core / memory-domain / implausible points. Pure + testable.
#[cfg(windows)]
fn sane_core_points(curve: &[(usize, u32, u32)]) -> Vec<(usize, u32, u32)> {
    curve
        .iter()
        .copied()
        .filter(|(_, mv, f)| is_sane_core_point(*mv, *f))
        .collect()
}

/// The selected contiguous graphics-core VF cluster (stage 2). Geometry only; pure.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
struct CoreCluster {
    v_min_mv: u32,
    v_max_mv: u32,
    f_min_mhz: u32,
    f_max_mhz: u32,
    point_count: usize,
    /// Unique core VF voltages in the chosen cluster, ascending. These are the real, writable bins
    /// the bin-based descent walks; `v_min_mv` (= `bins_mv.first()`) is the hardware-derived floor.
    bins_mv: Vec<u32>,
}

/// Select the actual stock core VF cluster from already-sane points: sort by voltage, split
/// into contiguous runs wherever the voltage gap exceeds `CORE_CLUSTER_GAP_MV`, and pick the
/// LARGEST run (ties → the lowest-voltage run, i.e. the real dense core). FAILS CLOSED if the
/// chosen run has fewer than `MIN_CORE_CLUSTER_POINTS` (ambiguous / not a real curve). This is
/// what isolates an outlier like a lone 1150 mV point from the dense ~600–1075 mV core curve.
/// Pure + testable; no hardware.
#[cfg(windows)]
fn select_core_cluster(sane: &[(usize, u32, u32)]) -> Result<CoreCluster, String> {
    if sane.is_empty() {
        return Err("no sane core points to cluster — failing closed".into());
    }
    let mut pts = sane.to_vec();
    pts.sort_by_key(|(idx, mv, _)| (*mv, *idx));
    let mut clusters: Vec<Vec<(usize, u32, u32)>> = Vec::new();
    for p in pts {
        match clusters.last_mut() {
            Some(c) if p.1.saturating_sub(c.last().unwrap().1) <= CORE_CLUSTER_GAP_MV => c.push(p),
            _ => clusters.push(vec![p]),
        }
    }
    // Largest by point count; ties → lowest starting voltage (the real core is dense + low).
    clusters.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].1.cmp(&b[0].1)));
    let chosen = &clusters[0];
    if chosen.len() < MIN_CORE_CLUSTER_POINTS {
        return Err(format!(
            "largest core VF cluster has only {} point(s) (< {} required) — ambiguous, failing closed",
            chosen.len(),
            MIN_CORE_CLUSTER_POINTS
        ));
    }
    // Unique voltages, ascending (the chosen run is already sorted by voltage) — the real bins the
    // bin-based descent will walk.
    let mut bins_mv: Vec<u32> = chosen.iter().map(|(_, mv, _)| *mv).collect();
    bins_mv.dedup();
    Ok(CoreCluster {
        v_min_mv: chosen.first().unwrap().1,
        v_max_mv: chosen.last().unwrap().1,
        f_min_mhz: chosen.iter().map(|(_, _, f)| *f).min().unwrap(),
        f_max_mhz: chosen.iter().map(|(_, _, f)| *f).max().unwrap(),
        point_count: chosen.len(),
        bins_mv,
    })
}

/// Stock reference derived from the SELECTED core VF cluster (not the global max of all sane
/// points), plus rejection + cluster diagnostics.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
struct CoreSeed {
    stock_boost_max_mhz: u32,
    stock_sustained_mhz: u32,
    safe_start_mv: u32,
    raw_count: usize,
    retained_count: usize,
    rejected_count: usize,
    rejected_max_freq_mhz: Option<u32>,
    rejected_max_voltage_mv: Option<u32>,
    cluster_point_count: usize,
    cluster_v_min_mv: u32,
    cluster_v_max_mv: u32,
    cluster_f_min_mhz: u32,
    cluster_f_max_mhz: u32,
    /// The selected core cluster's real VF voltages, ascending. The bin-based descent walks these;
    /// `cluster_v_min_mv` (= first) is the hardware-derived voltage floor (no hardcoded bound).
    cluster_bins_mv: Vec<u32>,
    outliers_above_count: usize,
    warnings: Vec<String>,
}

/// Derive the stock reference: generic sanity filter (stage 1) → select the contiguous core VF
/// cluster (stage 2) → derive boost/sustained/safe_start from the CLUSTER TOP (never the global
/// max of sane points, which can include isolated high-voltage outliers). FAILS CLOSED (`Err`)
/// when no sane points remain, the cluster is too small/ambiguous, or a derived value exceeds a
/// hard guard. Records rejected-point + cluster diagnostics and soft-limit warnings. Pure.
#[cfg(windows)]
fn derive_core_seed(curve: &[(usize, u32, u32)]) -> Result<CoreSeed, String> {
    let raw_count = curve.len();
    let sane = sane_core_points(curve);
    let retained_count = sane.len();
    let rejected_count = raw_count - retained_count;
    let rejected_max_freq_mhz = curve
        .iter()
        .filter(|(_, mv, f)| !is_sane_core_point(*mv, *f))
        .map(|(_, _, f)| *f)
        .max();
    let rejected_max_voltage_mv = curve
        .iter()
        .filter(|(_, mv, f)| !is_sane_core_point(*mv, *f))
        .map(|(_, mv, _)| *mv)
        .max();
    if sane.is_empty() {
        return Err(format!(
            "no sane graphics-core VF points among {raw_count} read (rejected max freq={:?} MHz, \
             max voltage={:?} mV) — failing closed; a GPU outside the sanity domain needs a code update",
            rejected_max_freq_mhz, rejected_max_voltage_mv
        ));
    }
    // Stage 2: select the real contiguous core cluster (fails closed if ambiguous/too small).
    let cluster = select_core_cluster(&sane)?;
    let stock_boost_max_mhz = cluster.f_max_mhz;
    let safe_start_mv = cluster.v_max_mv;
    // Isolated high-voltage sane points ABOVE the cluster top (e.g. a lone 1150 mV outlier).
    let outliers_above_count = sane.iter().filter(|(_, mv, _)| *mv > cluster.v_max_mv).count();
    // Defensive belt-and-suspenders (the filter + cluster already bound these).
    if stock_boost_max_mhz > CORE_FREQ_HARD_MAX_MHZ {
        return Err(format!(
            "derived boost {stock_boost_max_mhz} MHz exceeds core hard max {CORE_FREQ_HARD_MAX_MHZ} — failing closed"
        ));
    }
    if safe_start_mv > CORE_VF_HARD_MAX_MV {
        return Err(format!(
            "derived safe_start {safe_start_mv} mV exceeds core hard max {CORE_VF_HARD_MAX_MV} — failing closed"
        ));
    }
    let mut warnings = Vec::new();
    if stock_boost_max_mhz > CORE_FREQ_SOFT_WARN_MHZ {
        warnings.push(format!(
            "stock boost {stock_boost_max_mhz} MHz is above the soft-warn {CORE_FREQ_SOFT_WARN_MHZ} MHz"
        ));
    }
    // NOTE: the voltage soft-max warning is emitted by `run_build_frontier` via
    // `soft_max_voltage_warning`, where the effective (post --safe-start-cap) descent start is
    // known — so it can distinguish the derived curve top from the capped descent start.
    if outliers_above_count > 0 {
        warnings.push(format!(
            "rejected {outliers_above_count} isolated high-voltage outlier(s) above the core cluster top ({safe_start_mv} mV)"
        ));
    }
    Ok(CoreSeed {
        stock_boost_max_mhz,
        stock_sustained_mhz: stock_boost_max_mhz,
        safe_start_mv,
        raw_count,
        retained_count,
        rejected_count,
        rejected_max_freq_mhz,
        rejected_max_voltage_mv,
        cluster_point_count: cluster.point_count,
        cluster_v_min_mv: cluster.v_min_mv,
        cluster_v_max_mv: cluster.v_max_mv,
        cluster_f_min_mhz: cluster.f_min_mhz,
        cluster_f_max_mhz: cluster.f_max_mhz,
        cluster_bins_mv: cluster.bins_mv,
        outliers_above_count,
        warnings,
    })
}

/// Restore the GPU to stock: zero the core offset, clear the modern VF-curve offsets, and
/// release any NVML clock cap. Idempotent; called on every exit path of the supervised run.
#[cfg(windows)]
fn reset_to_stock() {
    let _ = nidavellir_gpu_nvapi::set_core_offset_mhz(0);
    let _ = nidavellir_gpu_nvapi::reset_vf_curve();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
}

/// A no-hardware "unverified" probe result: tells `build_frontier` to stop this clock's
/// descent (curve_verified=false) without recording a point. Used for the boundary guard,
/// the post-crash short-circuit, and any apply/verify failure.
#[cfg(windows)]
fn unverified_probe() -> ProbeSample {
    ProbeSample {
        outcome: ProbeOutcome::Unstable,
        curve_verified: false,
        avg_clock_mhz: 0,
        p5_clock_mhz: None,
        power_w: 0.0,
        max_power_w: 0.0,
        power_capped_frac: 0.0,
        measured_voltage_mv: None,
        vf_bin_mv: None,
        telemetry_quality: DwellQuality::Unavailable,
        voltage_quality: DwellQuality::Unavailable,
        confidence: 0.0,
        budget_drained: false,
        aborted: false,
        crashed: false,
    }
}

/// The REAL hardware probe for one `(target, vbin)` — the seam `build_frontier` calls under
/// `--confirm`. Snaps `vbin` to a real VF bin, arms the Safe Loop, applies the elastic VF
/// ceiling, read-only-verifies it (shared `classify_live_ceiling` + 11C diag), runs a
/// game-power dwell, clears the flag, and maps the result. On a dwell CRASH it resets to
/// stock and sets `abort` so the remaining probes short-circuit (whole run drains safely).
/// A normal `Unstable`/unverified result only stops THIS clock's descent (not the run).
/// `stock_top_mhz` (the seeded cluster boost top) enables the narrow stock-equivalent
/// verification for a boost-top target (Phase 2B.2-c.1) — see `gpu_verify`.
#[cfg(windows)]
fn real_probe_step(
    store: &SafeLoopStore,
    abort: &AtomicBool,
    descent: &FrontierDescent,
    tol_mhz: u32,
    target: u32,
    vbin: u32,
    stock_top_mhz: u32,
    stock_curve: &[(usize, u32, u32)],
    static_base: &[(usize, u32, u32)],
) -> ProbeSample {
    use nidavellir_gpu_nvapi as gpu;
    // 1. abort / boundary guards — no hardware. The abort short-circuit is flagged `aborted` so
    //    the scheduler drains (and never B2-falls-back) after a crash elsewhere; the boundary
    //    guard is a plain unverified stop.
    if abort.load(Ordering::SeqCst) {
        let mut p = unverified_probe();
        p.aborted = true;
        return p;
    }
    if vbin < descent.lowest_safe_mv {
        return unverified_probe();
    }
    // 2. snap the requested voltage to a real VF-table bin (same as apply/verify).
    let live = gpu::read_vf_curve_modern();
    let Some((ceiling_idx, ceiling_mv)) = gpu::nearest_vf_bin_at_or_above(&live, vbin) else {
        return unverified_probe();
    };
    // 3. arm the Safe Loop BEFORE the VF write (a crash anytime after → not reapplied on boot).
    let intent = TuningPoint::from_axes([
        ("gpu_freq_mhz", target as i64),
        ("gpu_vf_bin_mv", ceiling_mv as i64),
    ]);
    let _ = store.arm_boot_flag(&BootFlag::new(intent, "f1b_frontier"));
    // 4. apply the build-frontier MONOTONE-DOWN, static-base-anchored VF ceiling. This writer
    //    fails closed (Err) if the static base is unavailable/incomplete — it must NEVER fall
    //    back to the live-anchored apply_vf_ceiling (audit B1). Safe Loop stays armed (above).
    match gpu::apply_vf_ceiling_monotone(ceiling_mv, target) {
        Ok(down_caps) => {
            // Compact diagnostic from a pure preview over the once-per-run static base.
            let preview = gpu::plan_vf_ceiling_monotone(static_base, ceiling_mv, target);
            let benign_zeros = preview
                .iter()
                .filter(|e| e.in_flatten_set && e.desired_offset_mhz == 0)
                .count();
            let predicted_max = preview
                .iter()
                .filter(|e| e.in_flatten_set)
                .map(|e| e.base_mhz as i32 + e.desired_offset_mhz)
                .max()
                .unwrap_or(0);
            info!(
                "build-frontier probe: write_mode=monotone_static down_caps={down_caps} \
                 benign_zeros={benign_zeros} static_base_points={} \
                 monotone_predicted_max={predicted_max} positive_offsets=0 \
                 ceiling_mv={ceiling_mv} target={target}",
                static_base.len()
            );
        }
        Err(e) => {
            // Fail closed (audit B1/B3): reset, clear the armed boot flag, take the safe path.
            warn!("build-frontier probe: apply_vf_ceiling_monotone({ceiling_mv} mV, {target} MHz) failed closed: {e}");
            reset_to_stock();
            let _ = store.clear_boot_flag();
            return unverified_probe();
        }
    }
    // 5. read-only verify the JUST-applied transient ceiling (shared path) + log 11C diag.
    // Pass the once-per-run STATIC VF-table base for the NoDownCapNeeded benign-zero rescue.
    let after = gpu::read_vf_curve_modern();
    let eval = crate::gpu_verify::classify_live_ceiling(
        &after, ceiling_idx, ceiling_mv, target, tol_mhz, Some(stock_top_mhz), Some(static_base),
    );
    // Accept the normal offset-presence verdict OR the narrow stock-equivalent path (a boost-top
    // target whose missing offsets are bins already at target in stock) OR the NoDownCapNeeded
    // benign-zero rescue (sub-target bins that need no down-cap; static-table-evidence-gated).
    let verified = eval.state == nidavellir_core::ipc::CurveVerification::VerifiedCurve
        || eval.stock_equivalent
        || eval.no_down_cap_rescue;
    let verdict = if eval.stock_equivalent {
        "StockEquivalentCeiling".to_string()
    } else if eval.no_down_cap_rescue {
        "NoDownCapNeededCeiling".to_string()
    } else {
        format!("{:?}", eval.state)
    };
    info!(
        "build-frontier probe: target={target} ceiling_mv={ceiling_mv} verify={verdict} \
         offsets={}/{} stock_equiv_bins={} no_down_cap_needed={} eff_cov={:.3} \
         plateau={:?}..{:?} overshoot={:?}",
        eval.offset_present, eval.expected_n, eval.stock_equivalent_bins,
        eval.no_down_cap_needed, eval.effective_coverage,
        eval.diag.getstatus_plateau_min_mhz, eval.diag.getstatus_plateau_max_mhz,
        eval.diag.max_target_overshoot_mhz
    );
    if !verified {
        // Read-only failed-probe diagnostic BEFORE reset (registers still hold the write).
        // Joins the once-per-run STATIC VF-table base with the post-write offset readback to
        // label NoDownCapNeeded bins vs real gaps. Diagnostic ONLY — no verdict change.
        let diag = crate::gpu_verify::failed_probe_diag_line(stock_curve, static_base, ceiling_mv, target);
        info!(
            "build-frontier probe DIAG (read-only, no verdict change): target={target} \
             ceiling_mv={ceiling_mv} ceiling_idx={ceiling_idx} raw_cov={:.3} eff_cov={:.3} \
             overshoot_veto={} static_base_missing={} plateau={:?}..{:?} \
             overshoot={:?} undershoot={:?} | {diag}",
            if eval.expected_n > 0 { eval.offset_present as f32 / eval.expected_n as f32 } else { 0.0 },
            eval.effective_coverage, eval.overshoot_veto, eval.static_base_missing,
            eval.diag.getstatus_plateau_min_mhz, eval.diag.getstatus_plateau_max_mhz,
            eval.diag.max_target_overshoot_mhz, eval.diag.max_target_undershoot_mhz,
        );
        // The ceiling did not take — don't dwell; stop this clock's descent.
        reset_to_stock();
        let _ = store.clear_boot_flag();
        return unverified_probe();
    }
    // 6. game-power dwell, then clear the flag.
    let measured = load_and_measure(DWELL_MS);
    let _ = store.clear_boot_flag();
    // 7. map; record the actually-applied bin; abort the whole run on a hard crash.
    let crashed = matches!(measured.result, StabilityResult::Crash);
    let mut s = measured_to_probe(&measured, true, FRONTIER_PROBE_CONFIDENCE);
    s.vf_bin_mv = Some(ceiling_mv);
    if crashed {
        // Hard failure for the scheduler: this target's bracket must never seed the next target.
        s.crashed = true;
        reset_to_stock();
        abort.store(true, Ordering::SeqCst);
        warn!("build-frontier probe: dwell CRASH at {ceiling_mv} mV / {target} MHz — aborting run.");
    }
    s
}

/// Order the build-frontier log lines for operator output: scheduler/frontier decisions
/// (`result.log` — bracket carry-forward, warm-start, fallbacks, probes_used) FIRST, then the
/// profile-synthesis lines (`result.profiles.log`). Any scheduler line that also appears in the
/// synthesis log is dropped so a shared string is not emitted twice. Pure + testable — does not
/// touch `build_frontier` or `FrontierBuildResult`.
#[cfg(windows)]
fn ordered_frontier_logs<'a>(scheduler: &'a [String], synthesis: &'a [String]) -> Vec<&'a String> {
    scheduler
        .iter()
        .filter(|l| !synthesis.contains(l))
        .chain(synthesis.iter())
        .collect()
}

/// Supervised console entry for the F1b multi-clock frontier. Always prints the plan. WITHOUT
/// `confirm` it is a read-only DRY-RUN (no Safe Loop arm, no apply, no dwell, no VF write).
/// WITH `confirm` it runs the real supervised hardware path (transient VF ceilings + game-power
/// dwells) and ALWAYS restores stock afterwards. It NEVER applies or persists a profile, and
/// writes neither `forge_state.json` nor `gpu_knowledge.json`.
#[cfg(windows)]
pub fn run_build_frontier(store: &SafeLoopStore, confirm: bool, limits: FrontierLimits) {
    use nidavellir_gpu_nvapi as gpu;
    if !gpu::vf_curve_supported() {
        warn!("build-frontier: modern VF curve API unsupported on this GPU/driver — aborting.");
        return;
    }
    let live = gpu::read_vf_curve_modern();
    if live.is_empty() {
        warn!("build-frontier: VF curve readback returned no points — aborting.");
        return;
    }
    // Once-per-run STATIC VF-table base (GetStatus vf_tuple_base), index-aligned with `live`.
    // Feeds ONLY the read-only NoDownCapNeeded benign-zero rescue; empty (driver base
    // unsupported) → the verifier falls back to strict (no rescue). NOT a hardware write.
    let static_base = gpu::read_vf_base_curve_modern();
    info!(
        "build-frontier: static VF-table base evidence: {} points (NoDownCapNeeded rescue {})",
        static_base.len(),
        if static_base.is_empty() { "unavailable → strict" } else { "available" }
    );
    // SANITY-DOMAIN GUARD (Phase 2B.2-b.3): derive the stock reference ONLY from sane
    // graphics-core VF points — NEVER from the unfiltered global max, which can include
    // non-core / memory-domain / spurious points (a 3060 Ti dry-run produced 7001 MHz /
    // 1237 mV). Fail closed if no sane core points remain.
    let seed = match derive_core_seed(&live) {
        Ok(s) => s,
        Err(e) => {
            println!("=== build-frontier ABORTED (fail-closed) ===");
            println!("{e}");
            warn!("build-frontier: {e}");
            return;
        }
    };

    // One NON-LOAD telemetry snapshot for regime context. At idle this tends to read
    // Unconstrained; we CLAMP that to PowerLimited so the first supervised run never explores
    // ABOVE stock (no OC on a first run). Live regime-driven OC is a later refinement.
    let snap = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next();
    let (cap_frac, power_w, limit_w, temp_c) = match &snap {
        Some(r) => (
            if r.power_capped == Some(true) { 1.0 } else { 0.0 },
            r.power_w.unwrap_or(0.0),
            r.power_limit_w.unwrap_or(0.0),
            r.temperature_c,
        ),
        None => (0.0, 0.0, 0.0, None),
    };
    let regime_raw = classify_regime(cap_frac, power_w, limit_w, temp_c);
    let regime = if matches!(regime_raw, Regime::Unconstrained) {
        Regime::PowerLimited
    } else {
        regime_raw
    };

    // HARDWARE-DERIVED voltage floor: the lowest real graphics-core VF bin from the seeded
    // cluster (NOT a hardcoded value). Discovery descends naturally to this bin; the per-probe
    // verifier/dwell/Safe-Loop stop it earlier on any failure. `derive_core_seed` already failed
    // closed above if no sane core cluster exists, so this floor is always a real, writable bin.
    let hw_floor_mv = seed.cluster_v_min_mv;
    // Validate first-run limiter flags against the derived floor (fail closed on absurd values).
    if let Err(e) = validate_limits(&limits, hw_floor_mv) {
        println!("=== build-frontier ABORTED (invalid limits) ===");
        println!("{e}");
        warn!("build-frontier: {e}");
        return;
    }
    let raw_targets = candidate_clocks(
        seed.stock_sustained_mhz,
        seed.stock_boost_max_mhz,
        regime,
        FRONTIER_CLOCK_STEP_MHZ,
        FRONTIER_FLOOR_FRAC,
    );
    // Apply first-run limiters: --max-targets truncation + --safe-start-cap (never below the
    // hardware floor).
    let (targets, safe_start_mv) =
        apply_frontier_limits(raw_targets, seed.safe_start_mv, hw_floor_mv, &limits);
    // Defensive: a sane seed cannot produce an out-of-range target, but fail closed if it ever does.
    if targets.iter().any(|&t| t > CORE_FREQ_HARD_MAX_MHZ) {
        println!("=== build-frontier ABORTED (fail-closed) ===");
        println!("candidate target exceeds core hard max {CORE_FREQ_HARD_MAX_MHZ} MHz: {targets:?}");
        warn!("build-frontier: candidate target > {CORE_FREQ_HARD_MAX_MHZ} MHz — failing closed");
        return;
    }
    // BIN-BASED descent over the real core VF bins in [hardware floor..=safe_start]. The floor is
    // discovered from the curve; the sequence contains only real, writable bins.
    let descent = derive_descent(&seed.cluster_bins_mv, safe_start_mv, FRONTIER_VOLT_STEP_MV);
    if descent.bins_desc.is_empty() {
        // FAIL CLOSED (constraint 5): no real VF bin at or below the descent start means the
        // hardware floor could not be addressed — never fall back to an invented voltage grid.
        println!("=== build-frontier ABORTED (fail-closed: no VF bins in descent range) ===");
        println!(
            "no real core VF bin at or below the descent start {safe_start_mv} mV \
             (cluster bins {:?}) — cannot derive a hardware floor; aborting.",
            seed.cluster_bins_mv
        );
        warn!("build-frontier: no real VF bin ≤ {safe_start_mv} mV — failing closed (no hardware floor)");
        return;
    }
    let plan = plan_frontier(targets.clone(), &descent, DWELL_MS, limits.max_probes_per_target);
    // Effective dwell budget after --max-probes (the run hard-stops at this many probes).
    let capped_dwells = limits
        .max_probes
        .map_or(plan.est_dwell_count, |mp| plan.est_dwell_count.min(mp));
    let capped_secs = capped_dwells as u64 * (DWELL_MS + PROBE_OVERHEAD_MS) / 1000;

    println!("=== build-frontier PLAN (dry-run preview) ===");
    println!(
        "VF points          : {} read, {} sane-core retained, {} rejected",
        seed.raw_count, seed.retained_count, seed.rejected_count
    );
    println!(
        "rejected extremes  : max freq {:?} MHz, max voltage {:?} mV (non-core / implausible)",
        seed.rejected_max_freq_mhz, seed.rejected_max_voltage_mv
    );
    println!(
        "core cluster       : {} pts, {}..{} mV, {}..{} MHz (selected stock core VF domain)",
        seed.cluster_point_count, seed.cluster_v_min_mv, seed.cluster_v_max_mv,
        seed.cluster_f_min_mhz, seed.cluster_f_max_mhz
    );
    println!(
        "outliers above     : {} sane point(s) above the cluster top (isolated high-V, rejected)",
        seed.outliers_above_count
    );
    println!(
        "stock reference    : boost~{} MHz, sustained~{} MHz (from core cluster top)",
        seed.stock_boost_max_mhz, seed.stock_sustained_mhz
    );
    println!("safe_start source  : stock core cluster top ({} mV)", seed.safe_start_mv);
    println!(
        "hardware floor     : {} mV (lowest real core VF bin; descent never goes below it — \
         discovered, not hardcoded)",
        hw_floor_mv
    );
    println!(
        "limits             : max_targets={:?} max_probes={:?} max_probes_per_target={:?} safe_start_cap={:?}",
        limits.max_targets, limits.max_probes, limits.max_probes_per_target, limits.safe_start_cap_mv
    );
    println!(
        "scheduler mode     : {}",
        if limits.max_probes_per_target.is_some() {
            "coverage-bounded (per-target depth cap — reach multiple clocks before deepening one)"
        } else {
            "depth-first (full descent per target until floor / failure / global budget)"
        }
    );
    println!(
        "budget semantics   : global --max-probes is the hard cap; per-target cap only limits depth per target"
    );
    println!(
        "warm-start         : {} (margin {} step(s) = {} mV; first/hard-failed targets start at cap)",
        if limits.warm_start_brackets {
            "ENABLED — easier targets reuse the previous verified bracket"
        } else {
            "off — every target starts at safe_start_cap"
        },
        FRONTIER_WARM_START_MARGIN_STEPS,
        FRONTIER_WARM_START_MARGIN_STEPS * FRONTIER_VOLT_STEP_MV
    );
    for line in bind_seeking_plan_lines(limits.bind_seeking) {
        println!("{line}");
    }
    for line in phase_b_plan_lines(
        limits.power_bound_knee_seeking,
        limits.phase_b_probes.unwrap_or(FRONTIER_PHASE_B_PROBES),
    ) {
        println!("{line}");
    }
    println!("targets (MHz)      : {:?}", plan.targets);
    println!(
        "voltage descent    : bin-based, {} real VF bins {} mV -> {} mV (hardware floor)",
        plan.bins_per_descent, plan.safe_start_mv, plan.lowest_safe_mv
    );
    println!("descent bins (mV)  : {:?}", plan.descent_bins);
    let first_pass: Vec<u32> = plan
        .descent_bins
        .iter()
        .copied()
        .take(plan.effective_bins_per_descent as usize)
        .collect();
    println!(
        "first-pass bins(mV): {:?} ({} bin(s)/target — shallowest first under the per-target cap)",
        first_pass, plan.effective_bins_per_descent
    );
    if let Some(mp) = limits.max_probes {
        let eff = plan.effective_bins_per_descent.max(1);
        // ceil(mp / eff) targets get at least one probe before the global budget drains.
        let reachable = mp.div_ceil(eff).min(plan.targets.len() as u32);
        let skipped = plan.targets.len() as u32 - reachable;
        println!(
            "global-cap reach   : {} of {} target(s) reachable under --max-probes {} ({} skipped)",
            reachable,
            plan.targets.len(),
            mp,
            skipped
        );
    }
    println!(
        "worst-case dwells  : {} (~{} s, no early stop){}",
        capped_dwells,
        capped_secs,
        if limits.max_probes.is_some() { " [capped by --max-probes]" } else { "" }
    );
    println!("regime             : raw={regime_raw:?} used={regime:?}");
    if crate::gpu_apply::load_applied().is_some() {
        println!(
            "WARNING            : a profile appears applied (gpu_applied.json) — for STOCK frontier \
             seeding, reset to stock and re-run the dry-run (numbers reflect the applied curve)."
        );
        warn!("build-frontier: a profile appears applied; reset to stock for accurate stock seeding");
    }
    for w in &seed.warnings {
        println!("WARNING            : {w}");
    }
    // Voltage soft-max warning with curve-top-vs-capped-descent context (Phase 2B.2-c.0 polish).
    if let Some(w) =
        soft_max_voltage_warning(seed.safe_start_mv, descent.safe_start_mv, CORE_VF_SOFT_MAX_MV)
    {
        println!("WARNING            : {w}");
        warn!("build-frontier: {w}");
    }
    println!("SAFETY             : {}", plan.safety_notice);
    info!(
        "build-frontier PLAN: raw={} sane={} rejected={} rej_max_freq={:?} rej_max_mv={:?} \
         cluster_pts={} cluster_v={}..{} cluster_f={}..{} outliers_above={} boost~{} targets={:?} \
         {}..{} mV step {} bins/descent={} est_dwells={} est_wall_s={} regime_raw={:?} \
         regime_used={:?} bind_seeking={} confirm={}",
        seed.raw_count, seed.retained_count, seed.rejected_count, seed.rejected_max_freq_mhz,
        seed.rejected_max_voltage_mv, seed.cluster_point_count, seed.cluster_v_min_mv,
        seed.cluster_v_max_mv, seed.cluster_f_min_mhz, seed.cluster_f_max_mhz,
        seed.outliers_above_count, seed.stock_boost_max_mhz, plan.targets, plan.safe_start_mv,
        plan.lowest_safe_mv, plan.voltage_step_mv, plan.bins_per_descent, capped_dwells,
        capped_secs, regime_raw, regime, limits.bind_seeking, confirm
    );

    if !confirm {
        println!("(dry-run — pass --confirm to execute the SUPERVISED hardware run)");
        info!("build-frontier: DRY-RUN — no Safe Loop arm, no apply, no dwell, no VF write.");
        return;
    }

    warn!("build-frontier: CONFIRMED — supervised hardware run begins (game-power dwells; can TDR/reboot).");
    let policy = ForgePolicy::balanced();
    let abort = AtomicBool::new(false);
    let probe_count = std::sync::atomic::AtomicU32::new(0);
    let probe = |target: u32, vbin: u32| {
        // --max-probes hard stop: short-circuit (no hardware) once the budget is spent. Flagged
        // `budget_drained` so the scheduler treats it as a drain (never a verify failure / B2
        // fallback trigger).
        if let Some(mp) = limits.max_probes {
            if probe_count.fetch_add(1, Ordering::SeqCst) >= mp {
                let mut p = unverified_probe();
                p.budget_drained = true;
                return p;
            }
        }
        real_probe_step(
            store, &abort, &descent, FRONTIER_VERIFY_TOL_MHZ, target, vbin,
            seed.stock_boost_max_mhz, &live, &static_base,
        )
    };
    // Warm-start bracket carry-forward is opt-in (`--warm-start-brackets`); disabled → every
    // target starts at the cap, identical to the legacy behavior.
    let carry = BracketCarryConfig::from_descent(
        &descent,
        limits.warm_start_brackets,
        FRONTIER_WARM_START_MARGIN_STEPS,
    );
    // F1c (opt-in): when `--power-bound-knee-seeking` is set, a Phase-A power-bound collapse triggers
    // a focused Phase-B deep descent. OFF → the exact single-pass `build_frontier` call (unchanged).
    let phase_b_budget = limits
        .power_bound_knee_seeking
        .then(|| limits.phase_b_probes.unwrap_or(FRONTIER_PHASE_B_PROBES));
    let result = if let Some(budget) = phase_b_budget {
        info!(
            "build-frontier: power-bound knee-seeking ENABLED (opt-in) — Phase-B deep-descent budget \
             {budget} probe(s); global --max-probes stays the master cap."
        );
        build_frontier_two_phase(
            &targets,
            &descent,
            &policy,
            &carry,
            limits.max_probes_per_target,
            limits.bind_seeking,
            Some(budget),
            probe,
        )
        .result
    } else {
        build_frontier(
            &targets,
            &descent,
            &policy,
            &carry,
            limits.max_probes_per_target,
            limits.bind_seeking,
            probe,
        )
    };

    // ALWAYS restore stock after the run (success, partial, or abort). No profile is applied.
    reset_to_stock();
    let _ = store.clear_boot_flag();

    if abort.load(Ordering::SeqCst) {
        warn!("build-frontier: run ABORTED after a crash/TDR.");
    }
    println!("=== build-frontier RESULT ({} frontier points) ===", result.frontier.len());
    for p in &result.frontier {
        println!(
            "  target={:?} achieved={} MHz  vf_bin={:?} mV  power={:.0} W  p5={:?}  pcf={:.3}{}",
            p.target_clock_mhz, p.clock_mhz, p.vf_table_voltage_mv, p.power_w, p.p5_clock_mhz,
            p.power_capped_frac,
            if is_power_bound_point(p) { "  [power-bound]" } else { "" }
        );
    }
    // Frontier classification (F1b audit): how many points carry real clock-frontier information
    // vs. power-bound plateau points, and whether synthesis differentiated or collapsed.
    let pb = result.profiles.power_bound_excluded;
    let useful = result.frontier.len().saturating_sub(pb);
    println!(
        "frontier classes   : {useful} useful / {pb} power-bound (pcf >= {:.2}) — synthesis {}",
        POWER_BOUND_FRAC,
        if result.profiles.power_bound_collapse {
            "POWER-BOUND COLLAPSE (best-effort, NOT a differentiated VF frontier)"
        } else {
            "differentiated"
        }
    );
    let fmt = |label: &str, pt: &Option<PowerSweepPoint>| match pt {
        Some(p) => format!(
            "{label}: target={:?} achieved={} MHz @ vf_bin {:?} mV, {:.0} W",
            p.target_clock_mhz, p.clock_mhz, p.vf_table_voltage_mv, p.power_w
        ),
        None => format!("{label}: (none)"),
    };
    println!("{}", fmt("Godforge   ", &result.profiles.godforge));
    println!("{}", fmt("Brokkr's   ", &result.profiles.brokkrs));
    println!("{}", fmt("Deep Calm  ", &result.profiles.deep_calm));
    // Surface the scheduler/frontier decision log (bracket carry-forward / warm-start /
    // fallbacks / probes_used) BEFORE the profile-synthesis log, deduped against it.
    for l in ordered_frontier_logs(&result.log, &result.profiles.log) {
        info!("build-frontier: {l}");
    }
    info!("build-frontier: done — GPU restored to stock; no profile applied or persisted.");
}

/// Non-Windows stub — the frontier build is Windows-only (NVAPI/NVML).
#[cfg(not(windows))]
pub fn run_build_frontier(_store: &SafeLoopStore, _confirm: bool, _limits: FrontierLimits) {
    tracing::warn!("build-frontier is Windows-only");
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
                // Classify telemetry confidence; voltage is the weak link (sparse).
                let cp_q = clock_power_quality(m.sample_count);
                let voltage_q = voltage_quality(m.volt_sample_count);
                let telemetry_q = worst_quality(cp_q, voltage_q);
                info!(
                    "dwell_stats: target={target} offset=+{offset} avg_clock={} min_clock={} \
                     p5_clock={} avg_power={:.0}W peak_power={:.0}W cap={:.0}% avg_mv={:?} \
                     min_mv={:?} max_mv={:?} voltage_samples={} voltage_quality={:?} \
                     samples={} dur={}ms telemetry={:?}",
                    m.clock_mhz, m.min_clock_mhz, m.p5_clock_mhz, m.power_w, m.max_power_w,
                    m.capped_frac * 100.0, m.volt_avg_mv, m.volt_min_mv, m.volt_max_mv,
                    m.volt_sample_count, voltage_q, m.sample_count, m.duration_ms, telemetry_q
                );
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
                    min_clock_mhz: Some(m.min_clock_mhz),
                    p5_clock_mhz: Some(m.p5_clock_mhz),
                    avg_measured_voltage_mv: m.volt_avg_mv,
                    min_measured_voltage_mv: m.volt_min_mv,
                    max_measured_voltage_mv: m.volt_max_mv,
                    voltage_sample_count: Some(m.volt_sample_count),
                    voltage_quality: Some(voltage_q),
                    dwell_sample_count: Some(m.sample_count),
                    dwell_duration_ms: Some(m.duration_ms),
                    start_temp_c: m.start_temp_c,
                    end_temp_c: m.end_temp_c,
                    avg_temp_c: m.avg_temp_c,
                    telemetry_quality: Some(telemetry_q),
                    // Single-clock live sweep: no multi-clock frontier target (F1b Phase 2B.1).
                    target_clock_mhz: None,
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
            ..Default::default()
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
            ..Default::default()
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
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
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
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.godforge.unwrap().clock_mhz, 1815);
    }

    #[cfg(windows)]
    #[test]
    fn forge_falls_back_when_nothing_is_trusted() {
        // Immature data → nothing clears the gate → best-effort, still returns profiles.
        let frontier = vec![(fp(1830, 200.0), 0.21), (fp(1770, 164.0), 0.21)];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert!(p.godforge.is_some());
        assert!(p.log.iter().any(|l| l.contains("best-effort")));
    }

    // ── F1b Phase 1: multi-clock frontier synthesis with policy floors ──────────
    #[cfg(windows)]
    #[test]
    fn f1b_rtx3060ti_power_capped_frontier() {
        // Hard power-capped frontier (200 W); Balanced floors 98% / 90% of Godforge.
        let frontier = vec![
            (fp(1830, 190.0), 0.95),
            (fp(1815, 177.0), 0.95),
            (fp(1800, 170.0), 0.95),
            (fp(1770, 156.0), 0.95),
            (fp(1740, 150.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.godforge.unwrap().clock_mhz, 1830, "Godforge = highest sustainable clock");
        // Brokkr's floor 0.98*1830 = 1793.4 → only 1815/1800 eligible; max R = 1815.
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 1815, "Brokkr's = best R within 98% floor");
        // Deep Calm floor 0.90*1830 = 1647 → all eligible; max MHz/W = 1740.
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 1740, "Deep Calm = best MHz/W within 90% floor");
    }

    #[cfg(windows)]
    #[test]
    fn f1b_rtx4090_high_headroom_frontier() {
        // Unconstrained frontier with headroom; Godforge may be a real OC.
        let frontier = vec![
            (fp(2880, 405.0), 0.95),
            (fp(2860, 365.0), 0.95),
            (fp(2840, 335.0), 0.95),
            (fp(2800, 285.0), 0.95),
            (fp(2760, 260.0), 0.95),
            (fp(2700, 245.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.godforge.unwrap().clock_mhz, 2880, "Godforge = highest sustainable clock");
        // Floor 0.98*2880 = 2822.4 → 2860/2840 eligible. Max-R rule: 2860 (R≈14.3) beats
        // 2840 (R≈12.4) → principled choice is 2860 (stays nearest Godforge).
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 2860, "Brokkr's = max R within 98% floor");
        // Floor 0.90*2880 = 2592 → all eligible; max MHz/W = 2700.
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 2700, "Deep Calm = best MHz/W within 90% floor");
    }

    #[cfg(windows)]
    #[test]
    fn f1b_single_clock_collapse_is_handled() {
        // The old single-clock failure mode: one clock at several powers. Synthesis must
        // still return all three profiles and log the collapse (no panic / no empty).
        let frontier = vec![
            (fp(1770, 156.0), 0.95),
            (fp(1770, 165.0), 0.95),
            (fp(1770, 170.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.godforge.unwrap().clock_mhz, 1770);
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 1770);
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 1770); // no clock<gc candidate → Godforge
        assert!(p.log.iter().any(|l| l.contains("single sustainable clock")));
    }

    #[cfg(windows)]
    #[test]
    fn f1b_brokkrs_floor_boundary() {
        // Godforge 2000; Brokkr's floor 0.98 = 1960. A point at 1960 is eligible; 1959 is not.
        let at_floor = vec![(fp(2000, 200.0), 0.95), (fp(1960, 150.0), 0.95)];
        let p = synthesize_forge_profiles(&at_floor, &ForgePolicy::balanced());
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 1960, "exactly at floor → eligible");

        let below_floor = vec![(fp(2000, 200.0), 0.95), (fp(1959, 150.0), 0.95)];
        let p2 = synthesize_forge_profiles(&below_floor, &ForgePolicy::balanced());
        assert_eq!(p2.brokkrs.unwrap().clock_mhz, 2000, "below floor → no candidate → Godforge");
    }

    #[cfg(windows)]
    #[test]
    fn f1b_deep_calm_floor_boundary() {
        // Godforge 2000; Deep Calm floor 0.90 = 1800. 1799 has the best MHz/W but is below
        // the floor → excluded; best within floor (1850) wins.
        let frontier = vec![
            (fp(2000, 200.0), 0.95), // 10.0 MHz/W
            (fp(1850, 150.0), 0.95), // 12.33 MHz/W (within floor)
            (fp(1799, 100.0), 0.95), // 17.99 MHz/W but below the 1800 floor
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 1850, "below 90% floor excluded despite best MHz/W");
    }

    // ── F1b power-bound collapse classification (audit patch) ───────────────────────────────────
    /// A stable frontier point pinned at the power cap (`power_capped_frac = pcf`).
    #[cfg(windows)]
    fn pb_fp(clock_mhz: u32, power_w: f32, pcf: f32) -> PowerSweepPoint {
        let mut p = fp(clock_mhz, power_w);
        p.power_capped_frac = pcf;
        p
    }

    #[cfg(windows)]
    #[test]
    fn power_bound_frac_threshold_and_invalid() {
        assert!(is_power_bound_frac(POWER_BOUND_FRAC), "exactly at the threshold is power-bound");
        assert!(is_power_bound_frac(1.0), "fully capped is power-bound");
        assert!(!is_power_bound_frac(0.94), "just below the threshold is not power-bound");
        assert!(!is_power_bound_frac(0.0), "uncapped is not power-bound");
        // Missing / invalid fraction must NOT be marked power-bound (an unknown cap state is not a
        // plateau): fail open for classification (it fails CLOSED for regime binding, tested elsewhere).
        for bad in [f32::NAN, -0.1, 1.5] {
            assert!(!is_power_bound_frac(bad), "invalid pcf {bad} is not power-bound");
        }
        assert!(is_power_bound_point(&pb_fp(1800, 199.0, 1.0)));
        assert!(!is_power_bound_point(&fp(1800, 150.0))); // fp() sets pcf = 0.0
    }

    #[cfg(windows)]
    #[test]
    fn frontier_all_power_bound_is_collapse() {
        // The exact hardware failure mode: jittery clocks (1798/1811/1819) all pinned at pcf 1.0.
        // Exact-distinct-clock detection sees 3 "distinct" clocks; pcf saturation catches the collapse.
        let collapsed = vec![
            (pb_fp(1819, 199.0, 1.0), 0.21),
            (pb_fp(1811, 199.0, 1.0), 0.21),
            (pb_fp(1798, 199.0, 1.0), 0.21),
        ];
        assert!(frontier_power_bound_collapse(&collapsed), "all-power-bound jittery plateau = collapse");
        // A frontier with >= 2 useful points is NOT a collapse, even with a power-bound point present.
        let mixed = vec![
            (pb_fp(1850, 199.0, 1.0), 0.95),
            (fp(1830, 180.0), 0.95),
            (fp(1740, 150.0), 0.95),
        ];
        assert!(!frontier_power_bound_collapse(&mixed));
        // No power-bound points at all → never a power-bound collapse.
        assert!(!frontier_power_bound_collapse(&[(fp(1830, 180.0), 0.95)]));
    }

    #[cfg(windows)]
    #[test]
    fn synthesis_flags_power_bound_collapse_and_does_not_differentiate() {
        // All points power-bound → synthesis must NOT present a differentiated frontier. It returns a
        // flagged best-effort (still Some, never empty) and logs the explicit collapse diagnostic.
        let frontier = vec![
            (pb_fp(1819, 199.0, 1.0), 0.21),
            (pb_fp(1811, 199.0, 1.0), 0.21),
            (pb_fp(1798, 199.0, 1.0), 0.21),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert!(p.power_bound_collapse, "all-power-bound frontier flagged as collapse");
        assert_eq!(p.power_bound_excluded, 3, "all three points are power-bound");
        assert!(p.godforge.is_some(), "best-effort still returns a point (never empty)");
        assert!(p.log.iter().any(|l| l.contains("power-bound collapse")),
            "emits the explicit power-bound collapse diagnostic");
    }

    #[cfg(windows)]
    #[test]
    fn synthesis_excludes_power_bound_points_from_differentiation() {
        // Mixed frontier: a HIGH-clock power-bound plateau point (1850 @ pcf 1.0) plus >= 2 useful
        // points. The power-bound point must be EXCLUDED, so Godforge = the highest USEFUL clock
        // (1830), never the saturated 1850 plateau.
        let frontier = vec![
            (pb_fp(1850, 199.0, 1.0), 0.95), // power-bound, highest raw clock — must be excluded
            (fp(1830, 180.0), 0.95),
            (fp(1815, 170.0), 0.95),
            (fp(1740, 150.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert!(!p.power_bound_collapse, "two+ useful points → not a collapse");
        assert_eq!(p.power_bound_excluded, 1);
        assert_eq!(p.godforge.unwrap().clock_mhz, 1830,
            "Godforge is the highest USEFUL clock, not the power-bound 1850 plateau");
    }

    #[cfg(windows)]
    #[test]
    fn synthesis_without_power_bound_points_is_unchanged() {
        // Regression: no power-bound points → excluded = 0, collapse = false, identical selection.
        let frontier = vec![
            (fp(1830, 200.0), 0.95),
            (fp(1815, 181.0), 0.95),
            (fp(1740, 158.0), 0.95),
        ];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(p.power_bound_excluded, 0);
        assert!(!p.power_bound_collapse);
        assert_eq!(p.godforge.unwrap().clock_mhz, 1830);
    }

    #[cfg(windows)]
    #[test]
    fn f1b_sustainability_uses_p5_when_present() {
        // Two points at the same average clock (1830) but different p5: the dippy one
        // (p5 1700) must rank below the stable one (p5 1830) for Godforge.
        let mut stable = fp(1830, 190.0);
        stable.p5_clock_mhz = Some(1830);
        let mut dippy = fp(1830, 185.0); // lower power, but dips
        dippy.p5_clock_mhz = Some(1700);
        let frontier = vec![(stable, 0.95), (dippy, 0.95)];
        let p = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        // sustained(stable)=1830 > sustained(dippy)=1700 → Godforge = stable, despite its
        // higher power (p5 dominates the tie that average clock would have hidden).
        assert_eq!(p.godforge.unwrap().p5_clock_mhz, Some(1830));
    }

    #[cfg(windows)]
    #[test]
    fn f1b_legacy_points_without_p5_fall_back_to_avg_clock() {
        // fp() leaves p5_clock_mhz = None → sustained() falls back to clock_mhz; synthesis
        // still works exactly as the average-clock model (no panic on missing p5).
        let a = fp(1830, 190.0);
        let b = fp(1770, 156.0);
        assert_eq!(a.p5_clock_mhz, None);
        let p = synthesize_forge_profiles(&[(a, 0.95), (b, 0.95)], &ForgePolicy::balanced());
        assert_eq!(p.godforge.unwrap().clock_mhz, 1830);
        assert_eq!(p.deep_calm.unwrap().clock_mhz, 1770);
    }

    #[cfg(windows)]
    #[test]
    fn f1b_regime_classification() {
        assert_eq!(classify_regime(1.0, 200.0, 200.0, Some(70.0)), Regime::PowerLimited);
        assert_eq!(classify_regime(0.0, 330.0, 450.0, Some(65.0)), Regime::Unconstrained);
        assert_eq!(classify_regime(0.0, 300.0, 450.0, Some(85.0)), Regime::ThermalLimited);
        assert_eq!(classify_regime(0.9, 440.0, 450.0, Some(86.0)), Regime::Mixed);
    }

    #[cfg(windows)]
    #[test]
    fn f1b_candidate_clock_ranges() {
        // Power-capped: never probe above the sustained clock; descend to the 90% floor.
        let capped = candidate_clocks(1830, 1920, Regime::PowerLimited, 15, 0.90);
        assert_eq!(*capped.first().unwrap(), 1830);
        assert!(capped.iter().all(|&c| c <= 1830));
        assert!(*capped.last().unwrap() >= ((1830.0_f64 * 0.90).round() as u32));
        // Unconstrained: explore a few steps ABOVE the stock boost ceiling (real OC).
        let oc = candidate_clocks(2800, 2880, Regime::Unconstrained, 20, 0.90);
        assert!(oc.iter().any(|&c| c > 2800), "unconstrained explores above stock");
    }

    // ── F1b Phase 2A: simulated multi-clock outer-loop scaffolding ──────────────
    #[cfg(windows)]
    fn stable_sample(clock: u32, power: f32, conf: f64) -> ProbeSample {
        ProbeSample {
            outcome: ProbeOutcome::Stable,
            curve_verified: true,
            avg_clock_mhz: clock,
            p5_clock_mhz: Some(clock.saturating_sub(5)),
            power_w: power,
            max_power_w: power + 5.0,
            power_capped_frac: 0.0,
            measured_voltage_mv: None,
            vf_bin_mv: None,
            telemetry_quality: DwellQuality::Medium,
            voltage_quality: DwellQuality::Medium,
            confidence: conf,
            budget_drained: false,
            aborted: false,
            crashed: false,
        }
    }

    #[cfg(windows)]
    fn unstable_sample() -> ProbeSample {
        ProbeSample {
            outcome: ProbeOutcome::Unstable,
            curve_verified: true,
            avg_clock_mhz: 0,
            p5_clock_mhz: None,
            power_w: 0.0,
            max_power_w: 0.0,
            power_capped_frac: 0.0,
            measured_voltage_mv: None,
            vf_bin_mv: None,
            telemetry_quality: DwellQuality::Unavailable,
            voltage_quality: DwellQuality::Unavailable,
            confidence: 0.0,
            budget_drained: false,
            aborted: false,
            crashed: false,
        }
    }

    /// A verified-curve probe whose dwell CRASHED (hard failure → run abort). Models
    /// `real_probe_step`'s crash branch for the pure `build_frontier` seam.
    #[cfg(windows)]
    fn crashed_sample() -> ProbeSample {
        let mut s = unstable_sample();
        s.crashed = true;
        s
    }

    /// The abort-guard short-circuit (a prior crash set the run abort flag): no hardware ran.
    #[cfg(windows)]
    fn aborted_sample() -> ProbeSample {
        let mut s = unverified_probe();
        s.aborted = true;
        s
    }

    /// The `--max-probes` budget short-circuit: no hardware ran; a drain, not a verify failure.
    #[cfg(windows)]
    fn budget_sample() -> ProbeSample {
        let mut s = unverified_probe();
        s.budget_drained = true;
        s
    }

    // ── Phase 2B.1: measured_to_probe (pure conversion) + target_clock plumbing ───
    #[cfg(windows)]
    fn m_good(result: StabilityResult) -> Measured {
        Measured {
            result,
            clock_mhz: 1815,
            power_w: 180.0,
            max_power_w: 188.0,
            power_std_w: 2.0,
            capped_frac: 0.2,
            volt_mv: 869,
            sample_count: 120, // ≥100 → High clock/power quality
            duration_ms: 15_000,
            min_clock_mhz: 1770,
            p5_clock_mhz: 1800,
            volt_min_mv: Some(840),
            volt_avg_mv: Some(862),
            volt_max_mv: Some(869),
            volt_sample_count: 24, // 10..=49 → Medium voltage quality
            start_temp_c: Some(60.0),
            end_temp_c: Some(66.0),
            avg_temp_c: Some(63.0),
        }
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_stable_with_good_telemetry_is_stable() {
        let s = measured_to_probe(&m_good(StabilityResult::Stable), true, 0.9);
        assert!(matches!(s.outcome, ProbeOutcome::Stable));
        assert!(s.curve_verified);
        assert_eq!(s.avg_clock_mhz, 1815);
        assert_eq!(s.p5_clock_mhz, Some(1800)); // sustained-clock preserved
        assert_eq!(s.measured_voltage_mv, Some(862)); // filtered avg, telemetry only
        assert_eq!(s.telemetry_quality, DwellQuality::High);
        assert_eq!(s.voltage_quality, DwellQuality::Medium);
        assert_eq!(s.confidence, 0.9);
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_silent_error_is_unstable() {
        let s = measured_to_probe(&m_good(StabilityResult::SilentError), true, 0.9);
        assert!(matches!(s.outcome, ProbeOutcome::Unstable));
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_crash_or_tdr_is_unstable() {
        // A TDR / device-lost dwell surfaces as Measured::degenerate(Crash, …).
        let s = measured_to_probe(&Measured::degenerate(StabilityResult::Crash, 0), true, 0.0);
        assert!(matches!(s.outcome, ProbeOutcome::Unstable));
        assert_eq!(s.p5_clock_mhz, None); // no samples → None, not a 0 clock
        assert_eq!(s.measured_voltage_mv, None); // missing voltage → None, never 0
        assert_eq!(s.telemetry_quality, DwellQuality::Unavailable);
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_stable_but_low_telemetry_is_unstable() {
        // A Stable verdict with too few samples must NOT become a trusted stable probe.
        let mut m = m_good(StabilityResult::Stable);
        m.sample_count = 10; // Low (< 30)
        let s = measured_to_probe(&m, true, 0.9);
        assert!(matches!(s.outcome, ProbeOutcome::Unstable));
        assert_eq!(s.telemetry_quality, DwellQuality::Low);
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_stable_without_p5_is_unstable() {
        let mut m = m_good(StabilityResult::Stable);
        m.p5_clock_mhz = 0; // no sustained-clock signal
        let s = measured_to_probe(&m, true, 0.9);
        assert!(matches!(s.outcome, ProbeOutcome::Unstable));
        assert_eq!(s.p5_clock_mhz, None);
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_missing_voltage_is_none_not_zero() {
        let mut m = m_good(StabilityResult::Stable);
        m.volt_avg_mv = None;
        m.volt_sample_count = 0;
        let s = measured_to_probe(&m, true, 0.9);
        assert_eq!(s.measured_voltage_mv, None); // never a fake 0
        assert_eq!(s.voltage_quality, DwellQuality::Unavailable);
        // Missing voltage alone does not flip a well-sampled stable dwell to unstable.
        assert!(matches!(s.outcome, ProbeOutcome::Stable));
    }

    #[cfg(windows)]
    #[test]
    fn probe_to_point_records_target_clock() {
        let s = stable_sample(1815, 180.0, 0.9);
        let p = probe_to_point(1830, 850, &s);
        assert_eq!(p.target_clock_mhz, Some(1830)); // the asked-for clock
        assert_eq!(p.clock_mhz, 1815); // the measured achieved clock
        assert_eq!(p.vf_table_voltage_mv, Some(850));
    }

    // ── Phase 2B.2-b.1: seeding + dry-run plan + vf_bin propagation (pure) ────────
    #[cfg(windows)]
    #[test]
    fn f1b_seed_targets_from_regime() {
        // Power-limited regime → never probe above the sustained clock; floored at 90%.
        let regime = classify_regime(1.0, 200.0, 200.0, Some(70.0));
        assert_eq!(regime, Regime::PowerLimited);
        let targets = candidate_clocks(1830, 1920, regime, 30, 0.90);
        assert_eq!(targets.first().copied(), Some(1830)); // top = sustained, not boost
        assert!(targets.iter().all(|&c| c <= 1830));
        assert!(*targets.last().unwrap() >= (1830.0 * 0.90) as u32);
    }

    #[cfg(windows)]
    #[test]
    fn derive_descent_uses_real_bins_and_derived_floor() {
        // Bin-based: the descent walks ONLY real curve bins in [floor..=cap]; the floor is the
        // LOWEST real bin (discovered from the curve), never a hardcoded value; a bin above the cap
        // is excluded.
        let bins = [606u32, 700, 837, 850, 1062, 1075];
        let d = derive_descent(&bins, 1062, 25);
        assert_eq!(d.safe_start_mv, 1062); // highest real bin ≤ cap (1075 excluded by the cap)
        assert_eq!(d.lowest_safe_mv, 606); // DERIVED hardware floor = lowest real bin, not 875
        assert_eq!(d.bins_desc, vec![1062, 850, 837, 700, 606]); // descending real bins only
        assert_eq!(d.voltage_step_mv, 25); // nominal margin unit, not a descent grid
        // Every descent voltage is an actual curve bin — never an invented step-grid voltage.
        assert!(d.bins_desc.iter().all(|v| bins.contains(v)));
        // Degenerate (no real bin ≤ cap) → empty descent so the caller FAILS CLOSED; no invented
        // floor is fabricated.
        assert!(derive_descent(&bins, 500, 25).bins_desc.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn plan_frontier_estimates_dwells_and_time() {
        let d = step_descent(1000, 25, 900);
        let plan = plan_frontier(vec![1830, 1800, 1770], &d, 15_000, None);
        assert_eq!(plan.bins_per_descent, 5); // (1000-900)/25 + 1
        assert_eq!(plan.est_dwell_count, 15); // 3 targets × 5 bins (worst case, no early stop)
        assert_eq!(plan.est_wall_secs, 300); // 15 × (15000 + 5000) / 1000
        assert_eq!(plan.targets.len(), 3);
        assert!(plan.safety_notice.contains("SUPERVISED"));
    }

    // ── Bin-based descent over REAL (irregular-spaced) VF bins ───────────────────
    /// A hardware-like core curve whose voltage bins are NOT on a 25 mV grid — the case the
    /// bin-based descent exists for (a step grid would invent voltages between these).
    #[cfg(windows)]
    const IRREGULAR_BINS: [u32; 7] = [606, 700, 812, 850, 900, 975, 1062];

    #[cfg(windows)]
    #[test]
    fn plan_frontier_counts_real_irregular_bins() {
        let d = derive_descent(&IRREGULAR_BINS, 1062, 25);
        let plan = plan_frontier(vec![1830, 1800], &d, 15_000, None);
        assert_eq!(plan.bins_per_descent, 7); // the REAL bin count, not a step-grid span
        assert_eq!(plan.descent_bins, vec![1062, 975, 900, 850, 812, 700, 606]);
        assert!(plan.descent_bins.iter().all(|v| IRREGULAR_BINS.contains(v)));
        assert_eq!(plan.est_dwell_count, 14); // 2 targets × 7 real bins (worst case)
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_descends_only_real_bins_to_floor() {
        use std::cell::RefCell;
        let d = derive_descent(&IRREGULAR_BINS, 1062, 25);
        let probed = RefCell::new(Vec::<u32>::new());
        let probe = |target: u32, vbin: u32| {
            probed.borrow_mut().push(vbin);
            stable_sample(target, 180.0, 0.95) // stable everywhere → descend to the hardware floor
        };
        let r = build_frontier(&[1830u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        let seq = probed.borrow().clone();
        // Every probed voltage is a REAL curve bin — never an invented step-grid voltage.
        assert!(seq.iter().all(|v| IRREGULAR_BINS.contains(v)), "probed only real bins: {seq:?}");
        // The probe sequence is exactly the real descending bin domain, ending at the floor bin.
        assert_eq!(seq, vec![1062, 975, 900, 850, 812, 700, 606]);
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(606), "deepest stable = hardware floor bin");
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_snaps_margin_up_to_real_bin_never_below_lv() {
        use std::cell::RefCell;
        // Target A floors at a real bin (850). The warm-start margin target for B (850 + 25 = 875)
        // is NOT a bin → it must snap UP to the conservative real bin ≥ 875 (i.e. 900), never below
        // A's verified floor (B1), and never to an invented voltage.
        let d = derive_descent(&IRREGULAR_BINS, 1062, 25); // [1062,975,900,850,812,700,606]
        let carry = BracketCarryConfig::from_descent(&d, true, 1); // margin = 1 step = 25 mV
        let b_first = RefCell::new(None);
        let probe = |target: u32, vbin: u32| {
            if target == 1830 {
                if vbin >= 850 { stable_sample(1830, 180.0, 0.95) } else { unstable_sample() }
            } else {
                if b_first.borrow().is_none() {
                    *b_first.borrow_mut() = Some(vbin);
                }
                stable_sample(target, 175.0, 0.95)
            }
        };
        let r = build_frontier(&[1830u32, 1800], &d, &ForgePolicy::balanced(), &carry, None, false, probe);
        let first_b = b_first.borrow().expect("B was probed");
        assert_eq!(first_b, 900, "warm start snaps the 875 margin target UP to the real 900 bin");
        assert!(IRREGULAR_BINS.contains(&first_b), "start bin is a real curve bin");
        assert!(first_b >= 850, "B1: never starts below A's verified floor (850)");
        assert_eq!(r.frontier.len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_disabled_starts_at_top_real_bin_over_real_domain() {
        use std::cell::RefCell;
        let d = derive_descent(&IRREGULAR_BINS, 1062, 25);
        let b_first = RefCell::new(None);
        let probe = |target: u32, vbin: u32| {
            if target == 1800 && b_first.borrow().is_none() {
                *b_first.borrow_mut() = Some(vbin);
            }
            if vbin >= 850 { stable_sample(target, 180.0, 0.95) } else { unstable_sample() }
        };
        let r = build_frontier(
            &[1830u32, 1800], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        // Disabled carry → every target starts at the cap = the TOP real bin (1062), no warm-start.
        assert_eq!(*b_first.borrow(), Some(1062));
        assert_eq!(r.frontier.len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn max_probes_caps_irregular_bin_descent() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // The global --max-probes cap still bounds total executions over the (deeper) real bin
        // domain — bin-based descent does not widen exposure past the budget.
        let d = derive_descent(&IRREGULAR_BINS, 1062, 25);
        let calls = AtomicU32::new(0);
        let executed = AtomicU32::new(0);
        let max = 4u32;
        let probe = |target: u32, _vbin: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return budget_sample();
            }
            executed.fetch_add(1, SeqCst);
            stable_sample(target, 180.0, 0.95)
        };
        let _ = build_frontier(
            &[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        assert_eq!(executed.load(SeqCst), max); // exactly the budget over the bin domain
    }

    // ── Phase 2B.2-b.3/b.4: core sanity filter + stock core VF cluster (pure) ─────
    /// A dense, monotonic graphics-core VF cluster (freq rises 1:1 with mV above a base).
    #[cfg(windows)]
    fn dense_core(v_lo: u32, v_hi: u32, step: u32) -> Vec<(usize, u32, u32)> {
        let mut out = Vec::new();
        let (mut mv, mut i) = (v_lo, 0usize);
        while mv <= v_hi {
            out.push((i, mv, 1400 + (mv - v_lo)));
            mv += step;
            i += 1;
        }
        out
    }

    /// Test-only `FrontierDescent` whose bin domain is a legacy step grid
    /// `{safe_start, safe_start-step, …, floor}` (the floors used in these tests are on-grid), so
    /// the bin-based scheduler walks the exact vbin sequence the old step descent did. Real runs
    /// build `bins_desc` from actual VF bins via `derive_descent`.
    #[cfg(windows)]
    fn step_descent(safe_start_mv: u32, step: u32, floor: u32) -> FrontierDescent {
        let step = step.max(1);
        let mut bins_desc = Vec::new();
        let mut v = safe_start_mv;
        while v >= floor {
            bins_desc.push(v);
            if v < floor + step {
                break;
            }
            v -= step;
        }
        FrontierDescent { bins_desc, safe_start_mv, voltage_step_mv: step, lowest_safe_mv: floor }
    }

    #[cfg(windows)]
    #[test]
    fn sane_core_points_rejects_noncore_values() {
        // Stage-1 generic filter rejects memory-domain garbage; keeps plausible core points.
        let sane = sane_core_points(&[(0, 900, 1700), (1, 1237, 1900), (2, 900, 7001)]);
        assert_eq!(sane.len(), 1); // only (900, 1700)
        assert!(!is_sane_core_point(1237, 1900)); // voltage > hard max 1150
        assert!(!is_sane_core_point(900, 7001)); // freq > hard max 3500
        assert!(!is_sane_core_point(900, 0)); // zero freq
        assert!(!is_sane_core_point(500, 1900)); // voltage < min 600
        assert!(is_sane_core_point(700, 1600)); // plausible
        assert!(is_sane_core_point(1075, 2200)); // plausible high-end
    }

    #[cfg(windows)]
    #[test]
    fn cluster_rejects_isolated_high_voltage_outlier() {
        // Dense core to 1075 mV PLUS a lone 1150 mV point (75 mV gap > 60) → outlier rejected;
        // safe_start derives from the cluster top (1075), NOT the 1150 outlier.
        let mut c = dense_core(700, 1075, 25); // 16 pts: 1075 mV / 1775 MHz top
        c.push((99, 1150, 1850)); // isolated outlier
        let seed = derive_core_seed(&c).unwrap();
        assert_eq!(seed.safe_start_mv, 1075); // NOT 1150
        assert_eq!(seed.stock_boost_max_mhz, 1775); // cluster top freq, NOT the 1850 outlier
        assert_eq!(seed.cluster_point_count, 16);
        assert_eq!(seed.outliers_above_count, 1);
        assert!(seed.warnings.iter().any(|w| w.contains("outlier")));
    }

    #[cfg(windows)]
    #[test]
    fn cluster_ending_at_1075_derives_1075() {
        let seed = derive_core_seed(&dense_core(700, 1075, 25)).unwrap();
        assert_eq!(seed.safe_start_mv, 1075);
        assert_eq!(seed.outliers_above_count, 0);
        assert!(seed.warnings.is_empty()); // 1075 < soft max 1125, 1775 < soft-warn 3200
    }

    #[cfg(windows)]
    #[test]
    fn cluster_legitimately_ending_at_1150_derives_1150() {
        // A contiguous curve all the way to 1150 mV (no gap) → safe_start 1150. The voltage
        // soft-max warning now lives in run_build_frontier (curve top vs capped descent), so it
        // is no longer part of seed.warnings — see soft_max_warning_* tests below.
        let seed = derive_core_seed(&dense_core(700, 1150, 25)).unwrap();
        assert_eq!(seed.safe_start_mv, 1150);
        assert_eq!(seed.outliers_above_count, 0);
    }

    #[cfg(windows)]
    #[test]
    fn derive_core_seed_fails_closed_when_empty_or_ambiguous() {
        // No sane points at all (only memory garbage) → fail closed.
        let only_bad: Vec<(usize, u32, u32)> = vec![(200, 1237, 7001), (201, 1300, 6800)];
        assert!(derive_core_seed(&only_bad).is_err());
        assert!(derive_core_seed(&[]).is_err());
        // 3 widely-spaced sane points (gaps > 60 mV) → largest cluster = 1 < MIN(8) → fail closed.
        let sparse: Vec<(usize, u32, u32)> = vec![(0, 700, 1500), (1, 820, 1600), (2, 950, 1700)];
        assert!(derive_core_seed(&sparse).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn candidate_targets_seed_from_cluster_not_outlier() {
        // Cluster boost 1775; the 1850-MHz / 1150-mV outlier and 7001-MHz garbage must not leak.
        let mut c = dense_core(700, 1075, 25);
        c.push((99, 1150, 1850));
        c.push((200, 1237, 7001));
        let seed = derive_core_seed(&c).unwrap();
        assert_eq!(seed.stock_boost_max_mhz, 1775);
        let targets = candidate_clocks(
            seed.stock_sustained_mhz,
            seed.stock_boost_max_mhz,
            Regime::PowerLimited,
            30,
            0.90,
        );
        assert!(targets.iter().all(|&t| t <= CORE_FREQ_HARD_MAX_MHZ));
        assert!(targets.iter().all(|&t| t <= 1775));
        assert_eq!(targets.first().copied(), Some(1775));
    }

    #[cfg(windows)]
    #[test]
    fn core_seed_diagnostics_report_cluster_range() {
        let seed = derive_core_seed(&dense_core(700, 1075, 25)).unwrap();
        assert_eq!((seed.cluster_v_min_mv, seed.cluster_v_max_mv), (700, 1075));
        assert_eq!(seed.cluster_f_min_mhz, 1400);
        assert_eq!(seed.cluster_f_max_mhz, 1775);
        assert!(seed.cluster_point_count >= MIN_CORE_CLUSTER_POINTS);
        assert_eq!((seed.raw_count, seed.retained_count), (16, 16));
        // The seed exposes the cluster's real voltage bins (ascending) — the bin-based descent
        // domain. The HARDWARE FLOOR is the lowest of these, never a hardcoded value.
        assert_eq!(seed.cluster_bins_mv.first().copied(), Some(seed.cluster_v_min_mv));
        assert_eq!(seed.cluster_bins_mv.last().copied(), Some(seed.cluster_v_max_mv));
        assert_eq!(seed.cluster_bins_mv.first(), Some(&700)); // discovered floor, not 875
        assert!(seed.cluster_bins_mv.windows(2).all(|w| w[0] < w[1])); // strictly ascending, unique
        // Feeding the seed bins into the descent yields a real-bin sequence down to that floor.
        let d = derive_descent(&seed.cluster_bins_mv, seed.safe_start_mv, FRONTIER_VOLT_STEP_MV);
        assert_eq!(d.lowest_safe_mv, 700);
        assert_eq!(d.safe_start_mv, 1075);
        assert!(d.bins_desc.iter().all(|v| seed.cluster_bins_mv.contains(v)));
    }

    #[cfg(windows)]
    #[test]
    fn soft_max_warning_none_when_within_soft_max() {
        assert!(soft_max_voltage_warning(1075, 1075, 1125).is_none());
        assert!(soft_max_voltage_warning(1125, 1125, 1125).is_none()); // equal → no warning
    }

    #[cfg(windows)]
    #[test]
    fn soft_max_warning_above_soft_max_no_cap() {
        // Curve top above soft max, descent NOT capped (effective == curve top) → curve-top only.
        let w = soft_max_voltage_warning(1150, 1150, 1125).unwrap();
        assert!(w.contains("curve top 1150"));
        assert!(w.contains("soft max 1125"));
        assert!(!w.contains("capped"));
    }

    #[cfg(windows)]
    #[test]
    fn soft_max_warning_above_soft_max_with_cap() {
        // Curve top above soft max, descent capped below it → both values shown.
        let w = soft_max_voltage_warning(1150, 1075, 1125).unwrap();
        assert!(w.contains("curve top 1150"));
        assert!(w.contains("soft max 1125"));
        assert!(w.contains("capped to 1075"));
    }

    // ── Phase 2B.2-c.0: first-run limiter flags (pure) ───────────────────────────
    #[cfg(windows)]
    #[test]
    fn validate_limits_fails_closed_on_absurd() {
        let floor = 700u32; // a hardware-derived floor (validate_limits is floor-agnostic)
        assert!(validate_limits(&FrontierLimits { max_targets: Some(0), ..Default::default() }, floor).is_err());
        assert!(validate_limits(&FrontierLimits { max_probes: Some(0), ..Default::default() }, floor).is_err());
        // per-target cap of 0 → invalid; >= 1 → ok.
        assert!(validate_limits(&FrontierLimits { max_probes_per_target: Some(0), ..Default::default() }, floor).is_err());
        assert!(validate_limits(&FrontierLimits { max_probes_per_target: Some(2), ..Default::default() }, floor).is_ok());
        // cap at or below the crash floor → invalid.
        assert!(validate_limits(&FrontierLimits { safe_start_cap_mv: Some(floor), ..Default::default() }, floor).is_err());
        assert!(validate_limits(&FrontierLimits { safe_start_cap_mv: Some(floor - 1), ..Default::default() }, floor).is_err());
        // cap above the floor → ok; defaults → ok.
        assert!(validate_limits(&FrontierLimits { safe_start_cap_mv: Some(floor + 50), ..Default::default() }, floor).is_ok());
        assert!(validate_limits(&FrontierLimits::default(), floor).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn apply_limits_truncates_targets_and_caps_safe_start() {
        let limits = FrontierLimits { max_targets: Some(2), safe_start_cap_mv: Some(1075), ..Default::default() };
        let (t, ss) = apply_frontier_limits(vec![1935, 1905, 1875, 1845], 1150, 875, &limits);
        assert_eq!(t, vec![1935, 1905]); // top 2 kept
        assert_eq!(ss, 1075); // capped below the derived 1150
    }

    #[cfg(windows)]
    #[test]
    fn apply_limits_cap_never_raises_and_defaults_preserve() {
        // Cap ABOVE derived → no raise (keep derived).
        let (_, ss) = apply_frontier_limits(
            vec![1935],
            1075,
            875,
            &FrontierLimits { safe_start_cap_mv: Some(1200), ..Default::default() },
        );
        assert_eq!(ss, 1075);
        // No flags → targets + derived safe_start unchanged.
        let (t, ss2) = apply_frontier_limits(vec![1935, 1905], 1150, 875, &FrontierLimits::default());
        assert_eq!((t, ss2), (vec![1935, 1905], 1150));
    }

    #[cfg(windows)]
    #[test]
    fn max_probes_caps_real_probe_execution() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // Mirror run_build_frontier's --max-probes wrapper: cap real executions at 3.
        let calls = AtomicU32::new(0);
        let executed = AtomicU32::new(0);
        let max = 3u32;
        let probe = |target: u32, vbin: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return unverified_probe(); // budget spent → short-circuit, no "hardware"
            }
            executed.fetch_add(1, SeqCst);
            if vbin >= 900 {
                stable_sample(target, 180.0, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1000, 25, 800);
        let _ = build_frontier(&[1935, 1905, 1875], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(executed.load(SeqCst), max); // exactly 3 real probes ran before the cap
    }

    #[cfg(windows)]
    #[test]
    fn probe_to_point_prefers_vf_bin_over_descent_vbin() {
        let mut s = stable_sample(1815, 180.0, 0.9);
        s.vf_bin_mv = Some(850); // the actually-applied snapped bin
        let p = probe_to_point(1830, 999, &s); // descent vbin (999) deliberately differs
        assert_eq!(p.vf_table_voltage_mv, Some(850)); // recorded from vf_bin_mv, not 999
    }

    #[cfg(windows)]
    #[test]
    fn probe_to_point_falls_back_to_vbin_when_no_vf_bin() {
        let s = stable_sample(1815, 180.0, 0.9); // vf_bin_mv == None
        let p = probe_to_point(1830, 850, &s);
        assert_eq!(p.vf_table_voltage_mv, Some(850)); // fallback to the descent vbin
    }

    #[cfg(windows)]
    #[test]
    fn measured_to_probe_leaves_vf_bin_none() {
        // The pure mapper does not know the applied bin; the real probe fills it later.
        let s = measured_to_probe(&m_good(StabilityResult::Stable), true, 0.9);
        assert_eq!(s.vf_bin_mv, None);
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_short_circuits_after_abort_flag() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
        // Models the real probe's abort behavior: 1830 is stable at high bins, then "crashes"
        // deeper and trips an abort flag; every later probe short-circuits to Unstable. Proves
        // later targets are NOT fully descended (1 probe each, not 5).
        let abort = AtomicBool::new(false);
        let calls = AtomicU32::new(0);
        let probe = |target: u32, vbin: u32| {
            calls.fetch_add(1, SeqCst);
            if abort.load(SeqCst) {
                return unstable_sample();
            }
            if target == 1830 && vbin >= 950 {
                return stable_sample(1830, 190.0, 0.95);
            }
            if target == 1830 {
                abort.store(true, SeqCst); // crash deeper → abort the whole run
                return unstable_sample();
            }
            unstable_sample()
        };
        let d = step_descent(1000, 25, 900);
        let r = build_frontier(&[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        // 1830: 1000/975/950 stable (3) + 925 abort (1) = 4 calls; 1800 & 1770: 1 call each.
        assert_eq!(calls.load(SeqCst), 6);
        assert_eq!(r.frontier.len(), 1);
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(950)); // deepest stable bin kept
    }

    #[cfg(windows)]
    #[test]
    fn sim_frontier_3060ti_power_capped_picks_expected_profiles() {
        let targets = [1830u32, 1815, 1800, 1770, 1740];
        let probe = |target: u32, vbin: u32| -> ProbeSample {
            // (lowest-stable bin, power at that bin) per target — the descent bottoms here.
            let (min_mv, base) = match target {
                1830 => (925u32, 190.0f32),
                1815 => (900, 177.0),
                1800 => (875, 170.0),
                1770 => (850, 156.0),
                1740 => (825, 150.0),
                _ => (2000, 999.0),
            };
            if vbin >= min_mv {
                stable_sample(target, base + (vbin - min_mv) as f32 * 0.2, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1000, 25, 700);
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(r.frontier.len(), 5);
        assert_eq!(r.profiles.godforge.unwrap().clock_mhz, 1830);
        assert_eq!(r.profiles.brokkrs.unwrap().clock_mhz, 1815);
        assert_eq!(r.profiles.deep_calm.unwrap().clock_mhz, 1740);
    }

    #[cfg(windows)]
    #[test]
    fn sim_frontier_4090_headroom_picks_expected_profiles() {
        let targets = [2880u32, 2860, 2840, 2800, 2760, 2700];
        let probe = |target: u32, vbin: u32| -> ProbeSample {
            let (min_mv, base) = match target {
                2880 => (1075u32, 405.0f32),
                2860 => (1050, 365.0),
                2840 => (1025, 335.0),
                2800 => (975, 285.0),
                2760 => (950, 260.0),
                2700 => (925, 245.0),
                _ => (2000, 999.0),
            };
            if vbin >= min_mv {
                stable_sample(target, base + (vbin - min_mv) as f32 * 0.2, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1100, 25, 800);
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(r.frontier.len(), 6);
        assert_eq!(r.profiles.godforge.unwrap().clock_mhz, 2880, "Godforge = highest clock");
        assert_eq!(r.profiles.brokkrs.unwrap().clock_mhz, 2860, "Brokkr's = max R within 98% floor");
        assert_eq!(r.profiles.deep_calm.unwrap().clock_mhz, 2700, "Deep Calm = max MHz/W within 90% floor");
    }

    #[cfg(windows)]
    #[test]
    fn sim_inner_loop_stops_at_first_instability() {
        // Stable for vbin >= 900, unstable below → deepest stable is the 900 bin.
        let probe = |target: u32, vbin: u32| {
            if vbin >= 900 {
                stable_sample(target, 200.0 + (vbin - 900) as f32 * 0.1, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1000, 25, 700);
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(r.frontier.len(), 1);
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(900), "deepest stable bin kept");
        assert!(r.frontier[0].stable);
        assert_eq!(r.frontier[0].clock_mhz, 2000);
    }

    #[cfg(windows)]
    #[test]
    fn sim_respects_known_unsafe_boundary() {
        use std::cell::Cell;
        let min_probed = Cell::new(u32::MAX);
        // Would be stable even at 800 mV, but the loop must never probe below 950.
        let probe = |target: u32, vbin: u32| {
            min_probed.set(min_probed.get().min(vbin));
            if vbin >= 800 {
                stable_sample(target, 200.0, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1000, 25, 950);
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert!(min_probed.get() >= 950, "never probe below the known-unsafe floor");
        assert!(r.frontier[0].vf_table_voltage_mv.unwrap() >= 950);
    }

    #[cfg(windows)]
    #[test]
    fn sim_curve_verification_failure_rejects_or_aborts_point() {
        // The 1830 ceiling never verifies → its clock is dropped; 1770 is kept.
        let probe = |target: u32, _vbin: u32| {
            let mut s = stable_sample(target, 180.0, 0.95);
            if target == 1830 {
                s.curve_verified = false;
            }
            s
        };
        let d = step_descent(1000, 25, 700);
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(r.frontier.len(), 1, "unverified clock rejected");
        assert_eq!(r.frontier[0].clock_mhz, 1770);
        assert_eq!(r.profiles.godforge.unwrap().clock_mhz, 1770);
        assert!(r.log.iter().any(|l| l.contains("curve not verified")));
    }

    #[cfg(windows)]
    #[test]
    fn sim_partial_frontier_still_synthesizes_if_enough_points() {
        // 1815 can never stabilize (min above safe_start) → dropped; 1830 + 1770 remain.
        let probe = |target: u32, vbin: u32| {
            let min_mv = match target {
                1830 => 900u32,
                1815 => 1200, // > safe_start → first probe unstable → dropped
                1770 => 850,
                _ => 2000,
            };
            if vbin >= min_mv {
                stable_sample(target, if target == 1830 { 190.0 } else { 156.0 }, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(1000, 25, 700);
        let r = build_frontier(&[1830u32, 1815, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert_eq!(r.frontier.len(), 2, "partial frontier (1815 dropped)");
        assert_eq!(r.profiles.godforge.unwrap().clock_mhz, 1830);
        assert!(r.profiles.deep_calm.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn sim_single_clock_collapse_does_not_panic() {
        // Every target reports the SAME measured clock (1770) → collapse, handled.
        let probe = |_target: u32, vbin: u32| {
            if vbin >= 850 {
                stable_sample(1770, 150.0 + (vbin - 850) as f32 * 0.2, 0.95)
            } else {
                unstable_sample()
            }
        };
        let d = step_descent(950, 25, 700);
        let r = build_frontier(&[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert!(r.profiles.godforge.is_some());
        assert!(r.profiles.brokkrs.is_some());
        assert!(r.profiles.deep_calm.is_some());
        assert!(r.profiles.log.iter().any(|l| l.contains("single sustainable clock")));
    }

    #[cfg(windows)]
    #[test]
    fn sim_no_valid_points_returns_safe_failure() {
        let probe = |_t: u32, _v: u32| unstable_sample(); // nothing ever stable
        let d = step_descent(1000, 25, 700);
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe);
        assert!(r.frontier.is_empty());
        assert!(r.profiles.godforge.is_none());
        assert!(r.profiles.brokkrs.is_none());
        assert!(r.profiles.deep_calm.is_none());
    }

    // ── F1b warm-start voltage-bracket carry-forward (pure scheduler primitive) ────────
    /// build-frontier-shaped carry config: cap 1075, floor 875, step 25, margin 1 step.
    #[cfg(windows)]
    fn carry_cfg(enabled: bool) -> BracketCarryConfig {
        BracketCarryConfig { enabled, safe_start_cap_mv: 1075, floor_mv: 875, step_mv: 25, margin_steps: 1 }
    }

    #[cfg(windows)]
    fn bracket_with(target: u32, lowest_verified: Option<u32>, stop: BracketStop) -> TargetBracket {
        TargetBracket {
            target_mhz: target,
            highest_start_mv: 1075,
            lowest_verified_mv: lowest_verified,
            first_failed_below_verified_mv: None,
            stop_reason: stop,
            bracket_source_target: None,
            bracket_reuse_start_mv: None,
            bracket_reuse_margin_mv: 0,
            warm_started: false,
            fell_back_to_cap: false,
            probes_used: 0,
        }
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_disabled_always_starts_at_cap() {
        let prev = bracket_with(1815, Some(950), BracketStop::CleanFloor);
        let d = warm_start_mv(Some(&prev), &carry_cfg(false));
        assert_eq!(d.start_mv, 1075);
        assert!(!d.warm_started);
        assert_eq!(d.reason, WarmStartReason::Disabled);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_first_target_starts_at_cap() {
        let d = warm_start_mv(None, &carry_cfg(true));
        assert_eq!(d.start_mv, 1075);
        assert!(!d.warm_started);
        assert_eq!(d.reason, WarmStartReason::FirstTarget);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_carries_prev_floor_plus_one_step() {
        let prev = bracket_with(1815, Some(950), BracketStop::CleanFloor);
        let d = warm_start_mv(Some(&prev), &carry_cfg(true));
        assert_eq!(d.start_mv, 975); // 950 + one 25 mV step
        assert!(d.warm_started);
        assert_eq!(d.source_target, Some(1815));
        assert!(d.start_mv >= 950); // B1: never below the previous verified floor
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_no_bracket_falls_back_to_cap() {
        let prev = bracket_with(1815, None, BracketStop::SoftUnstable);
        let d = warm_start_mv(Some(&prev), &carry_cfg(true));
        assert_eq!(d.start_mv, 1075);
        assert!(!d.warm_started);
        assert_eq!(d.reason, WarmStartReason::NoBracket);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_hard_failed_prev_does_not_seed() {
        // Even with a verified floor, a crash/abort previous target must not seed (B1).
        for stop in [BracketStop::HardFailure, BracketStop::Aborted] {
            let prev = bracket_with(1815, Some(950), stop);
            let d = warm_start_mv(Some(&prev), &carry_cfg(true));
            assert_eq!(d.start_mv, 1075, "stop {stop:?} must fall back to cap");
            assert!(!d.warm_started);
            assert_eq!(d.reason, WarmStartReason::HardFailure);
        }
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_margin_clamps_to_cap() {
        // lv 1060 + 25 = 1085 > cap 1075 → collapses to cap (nothing to skip).
        let prev = bracket_with(1815, Some(1060), BracketStop::CleanFloor);
        let d = warm_start_mv(Some(&prev), &carry_cfg(true));
        assert_eq!(d.start_mv, 1075);
        assert!(!d.warm_started);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_never_below_floor() {
        // A verified floor exactly at the run floor → warm start is floor + one step, never below.
        let prev = bracket_with(1815, Some(875), BracketStop::CleanFloor);
        let cfg = carry_cfg(true);
        let d = warm_start_mv(Some(&prev), &cfg);
        assert_eq!(d.start_mv, 900);
        assert!(d.start_mv >= cfg.floor_mv);
    }

    #[cfg(windows)]
    #[test]
    fn warm_start_decision_is_deterministic() {
        let cfg = carry_cfg(true);
        let prev = bracket_with(1815, Some(950), BracketStop::CleanFloor);
        assert_eq!(warm_start_mv(Some(&prev), &cfg), warm_start_mv(Some(&prev), &cfg));
    }

    #[cfg(windows)]
    #[test]
    fn ordered_frontier_logs_scheduler_first_and_deduped() {
        // result.log (scheduler) lines come first; result.profiles.log (synthesis) lines after;
        // a string present in both is emitted once (kept in the synthesis position).
        let scheduler = vec![
            "bracket_carry enabled=true target=1935 ...".to_string(),
            "shared line".to_string(),
        ];
        let synthesis = vec!["shared line".to_string(), "FORGE: Godforge ...".to_string()];
        let out: Vec<&str> = ordered_frontier_logs(&scheduler, &synthesis)
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            out,
            vec![
                "bracket_carry enabled=true target=1935 ...",
                "shared line",
                "FORGE: Godforge ...",
            ]
        );
        assert_eq!(out.iter().filter(|s| **s == "shared line").count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn ordered_frontier_logs_emits_all_scheduler_lines_when_disjoint() {
        // The common case: the two vectors share nothing → every line is emitted, scheduler first.
        let scheduler = vec!["bracket A".to_string(), "bracket B".to_string()];
        let synthesis = vec!["FORGE x".to_string()];
        let out: Vec<&str> = ordered_frontier_logs(&scheduler, &synthesis)
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(out, vec!["bracket A", "bracket B", "FORGE x"]);
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_warm_start_matches_frontier_with_fewer_probes() {
        use std::cell::RefCell;
        let targets = [1815u32, 1785, 1755];
        let d = step_descent(1075, 25, 875);
        // Monotone synthetic: every target is verified+stable at/above 925 mV, unstable below.

        let off_calls = RefCell::new(0u32);
        let off_probe = |t: u32, v: u32| {
            *off_calls.borrow_mut() += 1;
            if v >= 925 { stable_sample(t, 180.0, 0.95) } else { unstable_sample() }
        };
        let off = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, off_probe);

        let on_calls = RefCell::new(0u32);
        let on_probe = |t: u32, v: u32| {
            *on_calls.borrow_mut() += 1;
            if v >= 925 { stable_sample(t, 180.0, 0.95) } else { unstable_sample() }
        };
        let on = build_frontier(&targets, &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, on_probe);

        // Identical frontier (same deepest verified bin per target), but fewer probes.
        let off_bins: Vec<_> = off.frontier.iter().map(|p| p.vf_table_voltage_mv).collect();
        let on_bins: Vec<_> = on.frontier.iter().map(|p| p.vf_table_voltage_mv).collect();
        assert_eq!(off_bins, on_bins);
        assert_eq!(on.frontier.len(), 3);
        assert!(
            *on_calls.borrow() < *off_calls.borrow(),
            "warm-start must probe fewer bins: on={} off={}",
            on_calls.borrow(),
            off_calls.borrow()
        );
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_b2_fallback_on_warm_start_verify_failure() {
        // 1815 verifies+stable down to 925 → seeds 950 for 1785. 1785's ceiling only VERIFIES at
        // high voltage (≥1000 mV); the warm-start probe at 950 fails verify → B2 falls back to the
        // cap, finds 1785's high-voltage ceiling, and does NOT drop the target.
        let probe = |target: u32, vbin: u32| match target {
            1815 => if vbin >= 925 { stable_sample(1815, 180.0, 0.95) } else { unstable_sample() },
            _ => if vbin >= 1000 { stable_sample(1785, 175.0, 0.95) } else { unverified_probe() },
        };
        let d = step_descent(1075, 25, 875);
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe);
        assert_eq!(r.frontier.len(), 2); // 1785 recovered via fallback, not dropped
        let t1785 = r.frontier.iter().find(|p| p.target_clock_mhz == Some(1785)).expect("1785 present");
        assert_eq!(t1785.vf_table_voltage_mv, Some(1000));
        assert!(r.log.iter().any(|l| l.contains("warm_start_verify_failed")));
        assert!(r.log.iter().any(|l| l.contains("fell_back_to_cap=true")));
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_verify_only_unstable_prev_does_not_seed() {
        use std::cell::RefCell;
        // 1815 VERIFIES everywhere but the dwell is Unstable → no verified floor (B1): 1785 must
        // start at the cap, not warm-start from a verify-only point.
        let first_vbin_1785 = RefCell::new(None);
        let probe = |target: u32, vbin: u32| {
            if target == 1815 {
                return unstable_sample(); // curve_verified=true, dwell Unstable
            }
            if first_vbin_1785.borrow().is_none() {
                *first_vbin_1785.borrow_mut() = Some(vbin);
            }
            if vbin >= 925 { stable_sample(1785, 175.0, 0.95) } else { unstable_sample() }
        };
        let d = step_descent(1075, 25, 875);
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe);
        assert_eq!(*first_vbin_1785.borrow(), Some(1075)); // started at the cap, no warm-start
        assert_eq!(r.frontier.len(), 1);
        assert_eq!(r.frontier[0].target_clock_mhz, Some(1785));
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_crash_prevents_carry_forward() {
        use std::cell::{Cell, RefCell};
        // 1815 verifies+stable down to 1000, then crashes at 975 → hard failure + run abort. 1815
        // must NOT seed 1785 (B1); 1785 starts at the cap and then drains on the abort guard.
        let aborted = Cell::new(false);
        let first_vbin_1785 = RefCell::new(None);
        let probe = |target: u32, vbin: u32| {
            // Record 1785's first probe voltage BEFORE any abort-drain short-circuit, so the test
            // can confirm it started at the cap (not warm-started from the crashed target).
            if target == 1785 && first_vbin_1785.borrow().is_none() {
                *first_vbin_1785.borrow_mut() = Some(vbin);
            }
            if aborted.get() {
                return aborted_sample();
            }
            if target == 1815 {
                if vbin >= 1000 {
                    return stable_sample(1815, 185.0, 0.95);
                }
                aborted.set(true);
                return crashed_sample();
            }
            stable_sample(1785, 175.0, 0.95)
        };
        let d = step_descent(1075, 25, 875);
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe);
        assert_eq!(*first_vbin_1785.borrow(), Some(1075)); // not seeded from the crashed target
        assert_eq!(r.frontier.len(), 1);
        assert_eq!(r.frontier[0].target_clock_mhz, Some(1815));
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(1000)); // deepest stable before the crash
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_warm_start_budget_drain_no_fallback_burst() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // A budget drain must NOT be mistaken for a verify failure (no B2 burst) and must honor
        // max-probes exactly.
        let calls = AtomicU32::new(0);
        let executed = AtomicU32::new(0);
        let max = 9u32;
        let probe = |target: u32, vbin: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return budget_sample();
            }
            executed.fetch_add(1, SeqCst);
            if vbin >= 925 { stable_sample(target, 180.0, 0.95) } else { unstable_sample() }
        };
        let d = step_descent(1075, 25, 875);
        let _ = build_frontier(&[1815u32, 1785, 1755], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe);
        assert_eq!(executed.load(SeqCst), max); // exactly the budget, no fallback burst beyond it
    }

    // ── F1b Option B: per-target probe cap (--max-probes-per-target) ──────────────────
    #[cfg(windows)]
    #[test]
    fn per_target_cap_none_preserves_full_descent() {
        use std::cell::RefCell;
        // No cap → identical probe sequence to the legacy full descent (down to the floor).
        let d = step_descent(1075, 25, 875); // 9 bins
        let probed = RefCell::new(Vec::<u32>::new());
        let probe = |target: u32, vbin: u32| {
            probed.borrow_mut().push(vbin);
            stable_sample(target, 180.0, 0.95)
        };
        let r = build_frontier(
            &[1830u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        assert_eq!(probed.borrow().len(), 9, "no cap → descend every bin");
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(875)); // reached the hardware floor
    }

    #[cfg(windows)]
    #[test]
    fn per_target_cap_covers_all_targets_before_deepening() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // 7 targets, per-target cap 2, no global drain → each target gets EXACTLY 2 shallow probes
        // and a point; none is dropped. Regression for "all probes spent on one target".
        let d = step_descent(1075, 25, 875); // bins 1075,1050,...,875
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let executed = AtomicU32::new(0);
        let probe = |target: u32, _vbin: u32| {
            executed.fetch_add(1, SeqCst);
            stable_sample(target, 180.0, 0.95)
        };
        let r = build_frontier(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), Some(2), false, probe,
        );
        assert_eq!(executed.load(SeqCst), 14, "7 targets × 2 probes");
        assert_eq!(r.frontier.len(), 7, "every target characterized — none dropped");
        // Each target stops at its 2nd (deepest probed) bin = 1050.
        assert!(r.frontier.iter().all(|p| p.vf_table_voltage_mv == Some(1050)));
    }

    #[cfg(windows)]
    #[test]
    fn per_target_cap_global_max_probes_still_hard() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // Per-target cap 2 with a GLOBAL --max-probes of 5 (modeled in the probe closure, as the real
        // run does): exactly 5 real probes run; later targets drain. The per-target cap can only
        // reduce/redistribute exposure — it never raises the global ceiling.
        let d = step_descent(1075, 25, 875);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let calls = AtomicU32::new(0);
        let executed = AtomicU32::new(0);
        let max = 5u32;
        let probe = |target: u32, _vbin: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return budget_sample();
            }
            executed.fetch_add(1, SeqCst);
            stable_sample(target, 180.0, 0.95)
        };
        let _ = build_frontier(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), Some(2), false, probe,
        );
        assert_eq!(executed.load(SeqCst), max, "global --max-probes stays the hard cap");
    }

    #[cfg(windows)]
    #[test]
    fn per_target_cap_stop_is_clean_and_carry_eligible() {
        // A descent stopped by the per-target cap (still verified+stable) is NOT a hard failure and —
        // having a verified floor — IS eligible to warm-start the next target (unlike a global drain).
        let prev = bracket_with(1815, Some(950), BracketStop::PerTargetCap);
        assert!(!prev.is_hard_failed());
        let d = warm_start_mv(Some(&prev), &carry_cfg(true));
        assert!(d.warm_started, "clean per-target-cap bracket seeds the next target");
        assert_eq!(d.start_mv, 975); // 950 + one 25 mV step
        assert_eq!(d.source_target, Some(1815));
    }

    #[cfg(windows)]
    #[test]
    fn per_target_cap_records_stop_reason_and_never_probes_bin_n_plus_1() {
        use std::cell::RefCell;
        // Direct descend_target check: cap 2 over an all-stable descent stops with PerTargetCap and a
        // verified floor at the 2nd bin, having probed EXACTLY 2 bins (never bin N+1).
        let d = step_descent(1075, 25, 875);
        let probed = RefCell::new(Vec::<u32>::new());
        let probe = |target: u32, vbin: u32| {
            probed.borrow_mut().push(vbin);
            stable_sample(target, 180.0, 0.95)
        };
        let (bracket, point) = descend_target(1935, 1075, &d, Some(2), false, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::PerTargetCap);
        assert_eq!(bracket.probes_used, 2);
        assert_eq!(*probed.borrow(), vec![1075u32, 1050]); // bin N+1 (1025) never probed
        assert_eq!(bracket.lowest_verified_mv, Some(1050));
        assert_eq!(point.unwrap().0.vf_table_voltage_mv, Some(1050));
    }

    // ── F1c two-phase power-bound knee-seeking (pure) ────────────────────────────────────────────
    /// A verified + dwell-stable probe with an explicit achieved clock / power / power-cap fraction.
    /// p5 is pinned to `clock` (not `clock-5`) so plateau/knee test numbers are exact. Confidence is
    /// the first-run 0.21 (synthesis drops its gate to best-effort — the realistic frontier case).
    #[cfg(windows)]
    fn pb_sample(clock: u32, power: f32, pcf: f32) -> ProbeSample {
        let mut s = stable_sample(clock, power, 0.21);
        s.p5_clock_mhz = Some(clock);
        s.power_capped_frac = pcf;
        s
    }

    #[cfg(windows)]
    #[test]
    fn plateau_clock_is_median_of_power_bound_clocks() {
        // The jittery saturated plateau (1798/1811/1819 @ pcf 1.0): median is robust where exact-
        // distinct detection saw "3 clocks". A non-power-bound point is IGNORED for the plateau.
        let frontier = vec![
            (pb_fp(1819, 199.0, 1.0), 0.21),
            (pb_fp(1798, 199.0, 1.0), 0.21),
            (pb_fp(1811, 199.0, 1.0), 0.21),
            (fp(1500, 120.0), 0.21), // off-cap → not part of the plateau
        ];
        assert_eq!(detect_plateau_clock(&frontier), Some(1811), "median of the power-bound clocks");
    }

    #[cfg(windows)]
    #[test]
    fn plateau_clock_none_without_enough_power_bound_points() {
        // One power-bound point is not a plateau; zero power-bound points is not a plateau.
        assert_eq!(detect_plateau_clock(&[(pb_fp(1810, 199.0, 1.0), 0.21)]), None);
        assert_eq!(detect_plateau_clock(&[(fp(1830, 180.0), 0.95), (fp(1770, 150.0), 0.95)]), None);
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_target_is_lowest_candidate_at_or_above_plateau() {
        let candidates = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        // Plateau ~1810 → lowest candidate >= 1810 is 1815 (not a wasted 1935, not a below-plateau 1785).
        assert_eq!(select_phase_b_target(&candidates, 1810), Some(1815));
        // Exact match returns itself.
        assert_eq!(select_phase_b_target(&candidates, 1815), Some(1815));
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_target_falls_back_to_nearest_when_all_below_plateau() {
        let candidates = [1755u32, 1785, 1815];
        // No candidate reaches a 1900 plateau → nearest (highest) candidate.
        assert_eq!(select_phase_b_target(&candidates, 1900), Some(1815));
        // Empty candidate set → None.
        assert_eq!(select_phase_b_target(&[], 1810), None);
    }

    #[cfg(windows)]
    #[test]
    fn knee_transition_classification() {
        // No previous point → a descent starts saturated; the first off-cap point is the knee.
        assert_eq!(classify_knee_transition(None, 1.0), KneeTransition::AboveKnee);
        assert_eq!(classify_knee_transition(None, 0.90), KneeTransition::KneeCrossed);
        // Still saturated (>= 0.95, incl. exactly at the threshold) → keep descending.
        assert_eq!(classify_knee_transition(Some(1.0), 0.96), KneeTransition::AboveKnee);
        assert_eq!(classify_knee_transition(Some(1.0), POWER_BOUND_FRAC), KneeTransition::AboveKnee);
        // Saturated → off-cap is the knee crossing.
        assert_eq!(classify_knee_transition(Some(1.0), 0.94), KneeTransition::KneeCrossed);
        // Already off-cap → the below-knee efficiency tail.
        assert_eq!(classify_knee_transition(Some(0.80), 0.60), KneeTransition::BelowKneeTail);
    }

    #[cfg(windows)]
    #[test]
    fn detect_knee_finds_first_pcf_crossing() {
        // Descending trajectory: saturated, saturated, then leaves saturation at index 2.
        let traj = vec![
            (pb_fp(1810, 199.0, 1.0), 0.21),
            (pb_fp(1810, 199.0, 0.98), 0.21),
            (pb_fp(1790, 185.0, 0.85), 0.21), // knee here
            (pb_fp(1760, 170.0, 0.60), 0.21),
        ];
        assert_eq!(detect_power_bound_knee(&traj), Some(2));
        // Never leaves saturation → no knee (collapse stands).
        let saturated = vec![
            (pb_fp(1810, 199.0, 1.0), 0.21),
            (pb_fp(1808, 199.0, 0.99), 0.21),
            (pb_fp(1812, 199.0, 0.97), 0.21),
        ];
        assert_eq!(detect_power_bound_knee(&saturated), None);
    }

    /// The synthetic power-bound card used by the descent / two-phase tests: above the operating-
    /// voltage knee (ceiling >= 950 mV) the ceiling is inert (pcf 1.0, clock pinned at the power cap);
    /// below it the ceiling bites and pcf/clock/power fall — the below-knee efficiency tail.
    #[cfg(windows)]
    fn knee_probe(_target: u32, v: u32) -> ProbeSample {
        if v >= 950 {
            pb_sample(1810, 199.0, 1.0)
        } else if v >= 925 {
            pb_sample(1790, 185.0, 0.85)
        } else if v >= 900 {
            pb_sample(1760, 170.0, 0.70)
        } else if v >= 875 {
            pb_sample(1730, 158.0, 0.55)
        } else {
            pb_sample(1700, 150.0, 0.45)
        }
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_descent_crosses_knee_and_records_tail() {
        // Deep descent from the cap (1075) over real bins down to 800; budget 12 (> the depth needed).
        let d = step_descent(1075, 25, 800); // 1075,1050,...,800 = 12 bins
        let traj = descend_phase_b(1815, 1075, &d, 12, &knee_probe);
        // Descends THROUGH the 6 inert top bins (>= 950 @ pcf 1.0), past the knee (925 @ 0.85), and
        // stops CLEANLY once clearly off the cap (850 @ 0.45 <= BIND_CAP_FRAC).
        assert_eq!(traj.stop_reason, BracketStop::LeftPowerRegime);
        assert_eq!(traj.points.len(), 10, "1075..850 inclusive");
        assert_eq!(detect_power_bound_knee(&traj.points), Some(6), "first off-cap point is the 7th (925 mV)");
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert!(useful >= 2, "below-knee tail has >= 2 useful points (got {useful})");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_descent_budget_bounds_depth() {
        // Budget 4 stops the descent before the knee (all 4 bins are above the operating voltage).
        let d = step_descent(1075, 25, 800);
        let traj = descend_phase_b(1815, 1075, &d, 4, &knee_probe);
        assert_eq!(traj.probes_used, 4);
        assert_eq!(traj.stop_reason, BracketStop::PerTargetCap);
        assert_eq!(detect_power_bound_knee(&traj.points), None, "too shallow to reach the knee");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_descent_stays_saturated_reaches_floor_no_knee() {
        // A card pinned at the cap across the WHOLE curve: descend to the floor, never a knee. This is
        // the only legitimate "true collapse" — proven by reaching the hardware floor still saturated.
        let d = step_descent(1075, 25, 800);
        let traj = descend_phase_b(1815, 1075, &d, 99, &|_t: u32, _v: u32| pb_sample(1810, 199.0, 1.0));
        assert_eq!(traj.stop_reason, BracketStop::CleanFloor);
        assert_eq!(traj.points.len(), 12, "every bin probed to the floor");
        assert_eq!(detect_power_bound_knee(&traj.points), None);
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_off_matches_single_pass_build_frontier() {
        // OFF (phase_b_budget = None) must be byte-for-byte the single-pass build_frontier result.
        let d = step_descent(1075, 25, 875);
        let targets = [1830u32, 1800, 1770];
        let probe = |t: u32, _v: u32| pb_sample(t, 180.0, 0.0); // off-cap, distinct clocks → differentiates
        let single = build_frontier(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, None, probe,
        );
        assert!(!two.phase_b_ran, "Phase B never runs when the budget is None");
        let f_single: Vec<u32> = single.frontier.iter().map(|p| p.clock_mhz).collect();
        let f_two: Vec<u32> = two.result.frontier.iter().map(|p| p.clock_mhz).collect();
        assert_eq!(f_single, f_two, "identical frontier");
        assert_eq!(
            single.profiles.godforge.map(|p| p.clock_mhz),
            two.result.profiles.godforge.map(|p| p.clock_mhz),
            "identical Godforge",
        );
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_collapse_triggers_deep_descent_and_differentiates() {
        // The headline case. Phase A (shallow cap 3) only sees the inert top bins (1075/1050/1025 @
        // pcf 1.0) → power-bound collapse, exactly like the validated hardware run. Phase B (budget 12)
        // CONTINUES below Phase A's deepest bin (1025) — starting at 1000 — and descends PAST the knee →
        // the merged frontier differentiates honestly.
        let d = step_descent(1075, 25, 800);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            Some(3), false, Some(12), knee_probe,
        );
        assert!(two.phase_b_ran, "collapse must trigger Phase B");
        assert_eq!(two.plateau_clock, Some(1810), "plateau = median power-bound clock");
        assert_eq!(two.focus_target, Some(1815), "lowest candidate >= plateau");
        // Phase B starts at 1000 (below Phase A's 1025 floor): trajectory 1000/975/950 (saturated) then
        // 925 off-cap → knee at index 3 (vs index 6 if it had restarted from the 1075 cap).
        assert_eq!(two.knee_index, Some(3), "knee detected after the skipped Phase-A bins");
        assert!(!two.result.profiles.power_bound_collapse, "deep descent de-collapsed the frontier");
        assert_eq!(
            two.result.profiles.godforge.map(|p| p.clock_mhz),
            Some(1790),
            "Godforge = highest sustained off-cap (knee-region) clock, not the 1810 power-bound plateau",
        );
        let useful = two.result.frontier.iter().filter(|p| !is_power_bound_point(p)).count();
        assert!(useful >= 2, "merged frontier carries the below-knee useful tail (got {useful})");
    }

    // ── F1c follow-up: Phase B continues below Phase A's explored floor (budget efficiency) ─────────
    #[cfg(windows)]
    #[test]
    fn phase_a_deepest_bin_finds_focus_floor() {
        // One retained point per target; its applied VF bin is the deepest Phase-A bin for that target.
        let frontier = vec![
            (probe_to_point(1935, 1025, &pb_sample(1810, 199.0, 1.0)), 0.21),
            (probe_to_point(1815, 1025, &pb_sample(1810, 199.0, 1.0)), 0.21),
        ];
        assert_eq!(phase_a_deepest_bin(&frontier, 1815), Some(1025));
        assert_eq!(phase_a_deepest_bin(&frontier, 1755), None, "target with no retained point");
        assert_eq!(phase_a_deepest_bin(&[], 1815), None, "empty frontier");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_start_below_returns_next_lower_real_bin() {
        let d = step_descent(1075, 25, 875); // bins 1075,1050,1025,1000,975,950,925,900,875
        assert_eq!(phase_b_start_below(&d, 1025), Some(1000), "highest real bin strictly below 1025");
        assert_eq!(phase_b_start_below(&d, 1000), Some(975));
        assert_eq!(phase_b_start_below(&d, 875), None, "no real bin below the hardware floor");
        assert_eq!(phase_b_start_below(&d, 1010), Some(1000), "a between-bins floor re-anchors to 1000");
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_phase_b_starts_below_phase_a_floor() {
        use std::cell::RefCell;
        // Instrument the probe to record every (target, bin). Phase A (cap 3) probes the focus target at
        // 1075/1050/1025; Phase B must CONTINUE at 1000 and below — never re-probing the top bins.
        let d = step_descent(1075, 25, 800);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let probed: RefCell<Vec<(u32, u32)>> = RefCell::new(Vec::new());
        let probe = |t: u32, v: u32| {
            probed.borrow_mut().push((t, v));
            knee_probe(t, v)
        };
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            Some(3), false, Some(12), probe,
        );
        assert!(two.phase_b_ran);
        assert_eq!(two.focus_target, Some(1815));
        let focus_bins: Vec<u32> =
            probed.borrow().iter().filter(|(t, _)| *t == 1815).map(|(_, v)| *v).collect();
        assert_eq!(
            focus_bins,
            vec![1075, 1050, 1025, 1000, 975, 950, 925, 900, 875, 850],
            "Phase A probes 1075/1050/1025; Phase B continues at 1000↓ — no re-probe of the top bins",
        );
        let mut uniq = focus_bins.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), focus_bins.len(), "no Phase-A bin re-probed by Phase B");
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_phase_b_no_phase_a_history_uses_safe_start_fallback() {
        // The focus target is UNSTABLE in Phase A (dropped, no retained point); the other targets are
        // power-bound stable → collapse with a plateau, and the dropped target is the lowest candidate
        // ≥ plateau. With no Phase-A history for it, Phase B falls back to the safe-start cap.
        let d = step_descent(1075, 25, 800);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let probe = |t: u32, _v: u32| {
            if t == 1815 { unstable_sample() } else { pb_sample(1810, 199.0, 1.0) }
        };
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            Some(3), false, Some(12), probe,
        );
        assert_eq!(two.focus_target, Some(1815));
        assert!(two.phase_b_ran, "Phase B still runs, from the fallback start");
        assert!(
            two.result.log.iter().any(|l| l.contains("no retained Phase-A point") && l.contains("fallback")),
            "logs the safe-start fallback",
        );
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_phase_b_skipped_when_phase_a_reached_floor() {
        // Phase A FULL descent (no per-target cap) over an all-power-bound card: the focus target
        // reaches the hardware floor in Phase A, so there is no deeper bin → Phase B is skipped cleanly
        // and the honest collapse is preserved (no unbounded behavior).
        let d = step_descent(1075, 25, 1000); // tiny: bins 1075,1050,1025,1000 (floor 1000)
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            None, false, Some(12), |_t: u32, _v: u32| pb_sample(1810, 199.0, 1.0),
        );
        assert!(!two.phase_b_ran, "no deeper bin below Phase A's floor → Phase B skipped");
        assert!(two.result.profiles.power_bound_collapse, "honest collapse preserved");
        assert!(
            two.result.log.iter().any(|l| l.contains("already reached the hardware floor")),
            "logs the no-deeper-bin skip",
        );
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_no_knee_keeps_honest_collapse() {
        // Phase A collapses; Phase B descends to the floor but the card stays pinned at the cap → no
        // knee → the merged frontier stays collapsed and the honest refusal is PRESERVED.
        let d = step_descent(1075, 25, 800);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            Some(3), false, Some(12), |_t: u32, _v: u32| pb_sample(1810, 199.0, 1.0),
        );
        assert!(two.phase_b_ran);
        assert_eq!(two.knee_index, None, "never left saturation");
        assert!(two.result.profiles.power_bound_collapse, "true collapse: honest refusal stands");
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_phase_a_differentiated_skips_phase_b() {
        // Phase A already produces a differentiated frontier (off-cap, distinct clocks) → no Phase B.
        let d = step_descent(1075, 25, 875);
        let targets = [1830u32, 1800, 1770];
        let two = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            None, false, Some(12), |t: u32, _v: u32| pb_sample(t, 180.0, 0.0),
        );
        assert!(!two.phase_b_ran, "no collapse → Phase B is skipped");
        assert!(!two.result.profiles.power_bound_collapse);
    }

    #[cfg(windows)]
    #[test]
    fn two_phase_global_max_probes_bounds_total() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // Global --max-probes (modeled in the closure, as the real run does) bounds Phase A + Phase B
        // TOGETHER: the Phase-B budget can never raise the global ceiling.
        let d = step_descent(1075, 25, 800);
        let targets = [1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let calls = AtomicU32::new(0);
        let executed = AtomicU32::new(0);
        let max = 10u32;
        let probe = |_t: u32, _v: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return budget_sample();
            }
            executed.fetch_add(1, SeqCst);
            pb_sample(1810, 199.0, 1.0) // always power-bound → Phase A collapses, Phase B descends
        };
        let _ = build_frontier_two_phase(
            &targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d),
            Some(3), false, Some(12), probe,
        );
        assert_eq!(executed.load(SeqCst), max, "global --max-probes stays the master cap across both phases");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_probes_zero_fails_closed() {
        let floor = 875;
        assert!(validate_limits(&FrontierLimits { phase_b_probes: Some(0), ..Default::default() }, floor).is_err());
        assert!(validate_limits(&FrontierLimits { phase_b_probes: Some(1), ..Default::default() }, floor).is_ok());
        // Default (None) is valid and means single-pass.
        assert!(validate_limits(&FrontierLimits::default(), floor).is_ok());
        assert!(!FrontierLimits::default().power_bound_knee_seeking, "knee-seeking is opt-in / default OFF");
        assert_eq!(FrontierLimits::default().phase_b_probes, None, "no Phase-B budget by default");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_plan_lines_report_mode_and_thresholds() {
        let off = phase_b_plan_lines(false, 12);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("off"));
        assert!(!off[0].contains("ENABLED"));
        let on = phase_b_plan_lines(true, 12);
        assert!(on.iter().any(|l| l.contains("ENABLED")));
        assert!(on.iter().any(|l| l.contains("budget 12")));
        assert!(on.iter().any(|l| l.contains("--max-probes stays the MASTER cap")));
        assert!(on.iter().any(|l| l.contains("CONTINUES below the focus target's deepest Phase-A bin")));
        assert!(on.iter().any(|l| l.contains("no profile applied")));
    }

    // ── F1b bind-seeking (regime-only: eligibility + left-power-regime) ─────────────────────────
    /// A verified + dwell-stable probe with EXACT avg/p5 clock (both set to `avg_mhz`) and a given
    /// power-cap fraction so the classifier inputs are unambiguous (the generic `stable_sample`
    /// hardcodes cap_frac = 0). The clock arm is retired; `cap_frac` drives the only (regime) rule.
    #[cfg(windows)]
    fn bind_sample(avg_mhz: u32, cap_frac: f32) -> ProbeSample {
        let mut s = stable_sample(avg_mhz, 180.0, 0.95);
        s.p5_clock_mhz = Some(avg_mhz);
        s.avg_clock_mhz = avg_mhz;
        s.power_capped_frac = cap_frac;
        s
    }

    #[cfg(windows)]
    #[test]
    fn classify_binding_start_bin_never_binds() {
        let t = BindThresholds::v2();
        // A sample that WOULD regime-bind (left the power regime, cap 0.3) must NOT bind on the start
        // bin (not eligible) — the start-bin guard is retained. It binds at the 2nd (eligible) bin.
        assert!(!classify_binding(1800, &bind_sample(1800, 0.3), &t, false).bound,
            "start bin is never binding even when the regime metric matches");
        let d = classify_binding(1800, &bind_sample(1800, 0.3), &t, true);
        assert!(d.bound && d.reason == BindReason::Regime, "second bin binds on the regime rule");
    }

    #[cfg(windows)]
    #[test]
    fn classify_binding_clock_near_target_does_not_bind_when_power_bound() {
        // CORE post-audit guarantee: the clock-near-target arm is RETIRED, so a probe whose average
        // clock sits exactly AT the target never binds while the card is power-bound/pinned. Saturated
        // pcf (1.0), near-saturated (0.95/0.9), and merely-pinned (0.6) all fail the regime rule.
        let t = BindThresholds::v2();
        for pcf in [1.0_f32, 0.95, 0.9, 0.6] {
            let d = classify_binding(1800, &bind_sample(1800, pcf), &t, true);
            assert!(!d.bound,
                "avg-clock at target must NOT bind when power-bound/pinned (pcf {pcf}) — clock arm retired");
            assert_eq!(d.reason, BindReason::None);
        }
    }

    #[cfg(windows)]
    #[test]
    fn classify_binding_regime_only_after_eligibility() {
        let t = BindThresholds::v2();
        // The regime rule is the only stop arm. cap_frac == 0.50 regime-binds — but ONLY when
        // eligible (never on the start bin); the clock metric is irrelevant (arm retired).
        assert!(!classify_binding(1800, &bind_sample(1900, 0.50), &t, false).bound,
            "cap_frac 0.50 does NOT bind on the start bin (not eligible)");
        let d = classify_binding(1800, &bind_sample(1900, 0.50), &t, true);
        assert!(d.bound && d.reason == BindReason::Regime, "cap_frac 0.50 binds (regime) when eligible");
        // cap_frac == 0.51 (still power-pinned) never binds, even eligible, whatever the clock.
        assert!(!classify_binding(1800, &bind_sample(1810, 0.51), &t, true).bound,
            "cap_frac 0.51 (still power-pinned) does not bind");
    }

    #[cfg(windows)]
    #[test]
    fn classify_binding_invalid_cap_frac_fails_closed_on_regime() {
        let t = BindThresholds::v2();
        // An invalid cap fraction (NaN / out of range) → no regime binding (fail closed), so the probe
        // does not bind even when eligible.
        for bad in [f32::NAN, -0.1, 1.5] {
            let d = classify_binding(1800, &bind_sample(1900, bad), &t, true);
            assert!(!d.bound, "invalid power_capped_frac {bad} must not regime-bind");
            assert!(d.power_capped_frac.is_none(), "invalid cap fraction reported as None");
        }
    }

    #[cfg(windows)]
    #[test]
    fn classify_binding_rejects_unverified_unstable_and_drains() {
        let t = BindThresholds::v2();
        // Eligible + regime metrics would bind (cap 0.0 ⇒ left the power regime), but an unverified
        // curve never binds.
        let mut unverified = bind_sample(1800, 0.0);
        unverified.curve_verified = false;
        assert!(!classify_binding(1800, &unverified, &t, true).bound, "unverified never binds");
        // An unstable dwell (outcome Unstable) never binds.
        assert!(!classify_binding(1800, &unstable_sample(), &t, true).bound, "unstable never binds");
        // Drain/crash/abort flags never bind even when the metrics + eligibility look binding.
        let mut drained = bind_sample(1800, 0.0);
        drained.budget_drained = true;
        assert!(!classify_binding(1800, &drained, &t, true).bound, "budget-drained never binds");
        let mut crashed = bind_sample(1800, 0.0);
        crashed.crashed = true;
        assert!(!classify_binding(1800, &crashed, &t, true).bound, "crashed never binds");
        let mut aborted = bind_sample(1800, 0.0);
        aborted.aborted = true;
        assert!(!classify_binding(1800, &aborted, &t, true).bound, "aborted never binds");
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_off_descends_full_depth_even_when_binding() {
        // Every probe WOULD regime-bind (cap 0 ⇒ left the power regime), but bind-seeking OFF must
        // descend the full depth to the floor exactly like today.
        let d = step_descent(1075, 25, 875); // 9 bins
        let probe = |target: u32, _v: u32| bind_sample(target, 0.0);
        let (bracket, point) = descend_target(1830, 1075, &d, None, false, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::CleanFloor);
        assert_eq!(bracket.lowest_verified_mv, Some(875));
        assert_eq!(point.unwrap().0.vf_table_voltage_mv, Some(875));
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_stops_at_first_binding_point() {
        use std::cell::RefCell;
        // Shallow bins still power-pinned (cap 0.9 → no regime bind); the card leaves the power regime
        // (cap 0.2) at vbin <= 1000, where bind-seeking stops.
        let d = step_descent(1075, 25, 875); // bins 1075,1050,1025,1000,...
        let probed = RefCell::new(Vec::<u32>::new());
        let probe = |target: u32, vbin: u32| {
            probed.borrow_mut().push(vbin);
            if vbin <= 1000 { bind_sample(target, 0.2) } else { bind_sample(target + 100, 0.9) }
        };
        let (bracket, point) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::LeftPowerRegime);
        assert_eq!(*probed.borrow(), vec![1075u32, 1050, 1025, 1000]); // stopped at the first binding bin
        assert_eq!(bracket.lowest_verified_mv, Some(1000));
        assert_eq!(point.unwrap().0.vf_table_voltage_mv, Some(1000));
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_skips_start_bin_and_binds_at_second() {
        use std::cell::RefCell;
        // Every bin WOULD regime-bind (cap 0.2 ⇒ left the power regime). The guard must NOT stop at the
        // start bin; it descends one real bin and binds at the 2nd probed bin (the first eligible one).
        // This is the regression guard for the v1 degenerate single-bin frontier.
        let d = step_descent(1075, 25, 875); // bins 1075, 1050, 1025, ...
        let probed = RefCell::new(Vec::<u32>::new());
        let probe = |target: u32, vbin: u32| {
            probed.borrow_mut().push(vbin);
            bind_sample(target, 0.2) // cap 0.2 ⇒ regime-binding at every (eligible) bin
        };
        let (bracket, point) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::LeftPowerRegime);
        assert_eq!(*probed.borrow(), vec![1075u32, 1050], "start bin skipped; binds at the 2nd bin");
        assert_eq!(bracket.lowest_verified_mv, Some(1050));
        assert_eq!(point.unwrap().0.vf_table_voltage_mv, Some(1050));
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_continues_through_nonbinding_until_cap() {
        // Always non-binding → bind-seeking keeps descending; the per-target cap still stops it.
        let d = step_descent(1075, 25, 875);
        let probe = |target: u32, _v: u32| bind_sample(target + 100, 0.9);
        let (bracket, _point) = descend_target(1800, 1075, &d, Some(3), true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::PerTargetCap, "no binding → cap still stops");
        assert_eq!(bracket.probes_used, 3, "descended past the first bin through non-binding probes");
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_no_binding_reaches_clean_floor() {
        // Never binds, no cap → descends every bin to the hardware floor (CleanFloor).
        let d = step_descent(1075, 25, 875); // 9 bins
        let probe = |target: u32, _v: u32| bind_sample(target + 100, 0.9);
        let (bracket, _p) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::CleanFloor);
        assert_eq!(bracket.lowest_verified_mv, Some(875));
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_verifier_failure_precedes_binding() {
        // First bin unverified → SoftUnverified before any binding evaluation; no point recorded.
        let d = step_descent(1075, 25, 875);
        let probe =
            |target: u32, vbin: u32| if vbin == 1075 { unverified_probe() } else { bind_sample(target, 0.0) };
        let (bracket, point) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::SoftUnverified);
        assert!(point.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_instability_precedes_binding() {
        // A verified-but-unstable dwell stops SoftUnstable before binding logic (binding is only
        // evaluated on the Stable arm).
        let d = step_descent(1075, 25, 875);
        let probe =
            |target: u32, vbin: u32| if vbin == 1075 { unstable_sample() } else { bind_sample(target, 0.0) };
        let (bracket, _p) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::SoftUnstable);
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_crash_and_abort_precede_binding() {
        let d = step_descent(1075, 25, 875);
        let crash =
            |target: u32, vbin: u32| if vbin == 1075 { crashed_sample() } else { bind_sample(target, 0.0) };
        let (b1, _) = descend_target(1800, 1075, &d, None, true, &crash);
        assert_eq!(b1.stop_reason, BracketStop::HardFailure);
        assert!(b1.is_hard_failed());
        let abort =
            |target: u32, vbin: u32| if vbin == 1075 { aborted_sample() } else { bind_sample(target, 0.0) };
        let (b2, _) = descend_target(1800, 1075, &d, None, true, &abort);
        assert_eq!(b2.stop_reason, BracketStop::Aborted);
        assert!(b2.is_hard_failed());
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_budget_drain_precedes_binding() {
        // A global --max-probes drain stops BudgetExhausted before binding logic (and is NOT a finding).
        let d = step_descent(1075, 25, 875);
        let probe =
            |target: u32, vbin: u32| if vbin == 1075 { budget_sample() } else { bind_sample(target, 0.0) };
        let (bracket, _p) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::BudgetExhausted);
    }

    #[cfg(windows)]
    #[test]
    fn left_power_regime_is_clean_and_carry_eligible() {
        // LeftPowerRegime is NOT a hard failure and — having a verified floor — seeds the next target
        // when warm-start is enabled (same carry-forward path as a clean PerTargetCap).
        let prev = bracket_with(1815, Some(950), BracketStop::LeftPowerRegime);
        assert!(!prev.is_hard_failed());
        let d = warm_start_mv(Some(&prev), &carry_cfg(true));
        assert!(d.warm_started, "a clean LeftPowerRegime bracket seeds the next target");
        assert_eq!(d.start_mv, 975); // 950 + one 25 mV step
        assert_eq!(d.source_target, Some(1815));
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_bind_seeking_off_matches_legacy_depth() {
        // Whole-pipeline default: bind-seeking OFF descends to the floor even when every probe binds.
        let d = step_descent(1075, 25, 875); // 9 bins
        let probe = |target: u32, _v: u32| bind_sample(target, 0.0);
        let r = build_frontier(
            &[1830u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        assert_eq!(r.frontier.len(), 1);
        assert_eq!(r.frontier[0].vf_table_voltage_mv, Some(875)); // reached the hardware floor
    }

    #[cfg(windows)]
    #[test]
    fn build_frontier_bind_seeking_stops_each_target_at_second_bin() {
        // Whole-pipeline bind-seeking v2: every probe would bind, but the start bin is skipped, so
        // each target stops at the 2nd bin (the first eligible one) — never the start cap. Under v1
        // this collapsed to the cap (1075) for every target; v2 forces a real descent step first.
        let d = step_descent(1075, 25, 875);
        let probe = |target: u32, _v: u32| bind_sample(target, 0.2);
        let r = build_frontier(
            &[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, true, probe,
        );
        assert_eq!(r.frontier.len(), 3);
        assert!(r.frontier.iter().all(|p| p.vf_table_voltage_mv == Some(1050)),
            "each target binds at the 2nd bin (1050), not the start cap (1075)");
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_plan_lines_reports_mode_thresholds_and_eligibility() {
        // OFF → one line, no binding logic implied, no thresholds.
        let off = bind_seeking_plan_lines(false);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("off"));
        assert!(!off.iter().any(|l| l.contains("clock_overshoot")));
        // ON → regime-only mode + the single regime threshold + start-bin eligibility + live caveat.
        let on = bind_seeking_plan_lines(true);
        assert!(on.iter().any(|l| l.contains("ENABLED")));
        assert!(on.iter().any(|l| l.contains("regime-only")));
        assert!(on.iter().any(|l| l.contains("power_capped_frac <= 0.50")));
        // The clock arm is retired — the dry-run must NOT advertise a clock-overshoot threshold.
        assert!(!on.iter().any(|l| l.contains("avg_clock_overshoot")));
        // The dry-run must state the start bin is not bind-eligible.
        assert!(on.iter().any(|l| l.contains("NOT bind-eligible")));
        assert!(on.iter().any(|l| l.contains("depends on live")));
    }

    #[cfg(windows)]
    #[test]
    fn plan_frontier_reports_per_target_cap_and_reduced_dwells() {
        let d = step_descent(1075, 25, 875); // 9 bins
        let targets = vec![1935u32, 1905, 1875, 1845, 1815, 1785, 1755];
        let plan = plan_frontier(targets, &d, 15_000, Some(2));
        assert_eq!(plan.bins_per_descent, 9); // full descent depth unchanged
        assert_eq!(plan.max_probes_per_target, Some(2));
        assert_eq!(plan.effective_bins_per_descent, 2);
        assert_eq!(plan.est_dwell_count, 14); // 7 targets × 2 (was 7 × 9 = 63 uncapped)
        // No cap → effective equals the full descent.
        let plan_full = plan_frontier(vec![1935u32, 1905], &d, 15_000, None);
        assert_eq!(plan_full.effective_bins_per_descent, 9);
        assert_eq!(plan_full.est_dwell_count, 18);
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
        // The richer dwell-stat fields also default cleanly on legacy points.
        assert_eq!(p.p5_clock_mhz, None);
        assert_eq!(p.voltage_sample_count, None);
        assert_eq!(p.telemetry_quality, None);
    }

    #[test]
    fn p5_clock_normal_small_and_empty() {
        // 20 samples (1700..=1719) → p5 index = floor(19*0.05)=0 → the lowest, 1700.
        let cs: Vec<u32> = (1700..1720).collect();
        assert_eq!(p5_clock_mhz(&cs), Some(1700));
        assert_eq!(p5_clock_mhz(&[1830]), Some(1830)); // single sample
        assert_eq!(p5_clock_mhz(&[]), None); // empty
    }

    #[test]
    fn voltage_stats_aggregates_and_handles_empty() {
        let (mn, avg, mx, c) = voltage_stats(&[837, 850, 869]).unwrap();
        assert_eq!((mn, mx, c), (837, 869, 3));
        assert_eq!(avg, (837 + 850 + 869) / 3);
        assert_eq!(voltage_stats(&[]), None);
    }

    #[test]
    fn quality_classification_thresholds() {
        assert_eq!(clock_power_quality(0), DwellQuality::Unavailable);
        assert_eq!(clock_power_quality(10), DwellQuality::Low);
        assert_eq!(clock_power_quality(50), DwellQuality::Medium);
        assert_eq!(clock_power_quality(200), DwellQuality::High);
        assert_eq!(voltage_quality(0), DwellQuality::Unavailable);
        assert_eq!(voltage_quality(5), DwellQuality::Low);
        assert_eq!(voltage_quality(20), DwellQuality::Medium);
        assert_eq!(voltage_quality(60), DwellQuality::High);
        // Overall takes the worst metric (voltage is the weak link here).
        assert_eq!(
            worst_quality(DwellQuality::High, DwellQuality::Medium),
            DwellQuality::Medium
        );
    }

    #[cfg(windows)]
    #[test]
    fn power_point_roundtrips_dwell_stats() {
        let mut p = fp(1770, 166.0);
        p.p5_clock_mhz = Some(1765);
        p.min_clock_mhz = Some(1755);
        p.avg_measured_voltage_mv = Some(862);
        p.voltage_sample_count = Some(24);
        p.voltage_quality = Some(DwellQuality::Medium);
        p.telemetry_quality = Some(DwellQuality::Medium);
        p.avg_temp_c = Some(64.0);
        let json = serde_json::to_string(&p).expect("encode");
        let back: PowerSweepPoint = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.p5_clock_mhz, Some(1765));
        assert_eq!(back.avg_measured_voltage_mv, Some(862));
        assert_eq!(back.voltage_quality, Some(DwellQuality::Medium));
        assert_eq!(back.telemetry_quality, Some(DwellQuality::Medium));
        assert_eq!(back.avg_temp_c, Some(64.0));
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
