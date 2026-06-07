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
    log: Vec<String>,
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

    // Confidence gate (reuses V2): trust only well-tested points; else best-effort.
    let pool: Vec<(PowerSweepPoint, f64)> = {
        let trusted: Vec<(PowerSweepPoint, f64)> = frontier
            .iter()
            .copied()
            .filter(|(_, c)| *c >= policy.confidence_threshold)
            .collect();
        if trusted.is_empty() {
            let best = frontier.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
            log.push(format!(
                "FORGE: no point met confidence ≥ {:.2} (best {best:.2}) — best-effort synthesis",
                policy.confidence_threshold
            ));
            frontier.to_vec()
        } else {
            trusted
        }
    };
    if pool.is_empty() {
        return ForgeProfiles { godforge: None, brokkrs: None, deep_calm: None, log };
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
    telemetry_quality: DwellQuality,
    voltage_quality: DwellQuality,
    /// Accumulated stability confidence (Wilson LB) for this point — feeds the gate.
    confidence: f64,
}

/// Voltage-bin descent config for a target clock. The descent never probes below
/// `lowest_safe_mv` (the known-crash floor from Forge Knowledge — a config input here).
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct FrontierDescent {
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
        vf_table_voltage_mv: Some(vbin),
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
        telemetry_quality,
        voltage_quality,
        confidence,
    }
}

/// Build a multi-clock frontier by descending each candidate clock's voltage bins via
/// the injected `probe` closure, then synthesize the three profiles with `policy`.
///
/// Inner loop (per target): start at `safe_start_mv`, descend by `voltage_step_mv`,
/// never below `lowest_safe_mv`; stop on the first `Unstable` (keep the deepest stable);
/// stop if `curve_verified` is false (the ceiling did not apply — can't trust deeper);
/// drop the clock if no stable point was found. Outer loop: process candidate clocks in
/// order, allow a partial frontier, then synthesize. Pure — the closure is the only
/// seam to (future) hardware. Never runs stress / writes the VF curve.
#[cfg(windows)]
#[allow(dead_code)] // wired to the real measurement closure in Phase 2B
fn build_frontier(
    candidate_clocks: &[u32],
    descent: &FrontierDescent,
    policy: &ForgePolicy,
    probe: impl Fn(u32, u32) -> ProbeSample,
) -> FrontierBuildResult {
    let mut paired: Vec<(PowerSweepPoint, f64)> = Vec::new();
    let mut log = Vec::new();

    for &target in candidate_clocks {
        let mut deepest: Option<(PowerSweepPoint, f64)> = None;
        let mut v = descent.safe_start_mv;
        while v >= descent.lowest_safe_mv {
            let s = probe(target, v);
            if !s.curve_verified {
                log.push(format!(
                    "{target} MHz @ {v} mV: curve not verified (simulated) — stop descent"
                ));
                break;
            }
            match s.outcome {
                ProbeOutcome::Stable => {
                    deepest = Some((probe_to_point(target, v, &s), s.confidence));
                    if v < descent.voltage_step_mv {
                        break;
                    }
                    v -= descent.voltage_step_mv;
                }
                ProbeOutcome::Unstable => {
                    log.push(format!("{target} MHz @ {v} mV: unstable — keep deepest stable"));
                    break;
                }
            }
        }
        match deepest {
            Some(p) => paired.push(p),
            None => log.push(format!("{target} MHz: no stable point in safe range — dropped")),
        }
    }

    let profiles = synthesize_forge_profiles(&paired, policy);
    FrontierBuildResult {
        frontier: paired.into_iter().map(|(p, _)| p).collect(),
        profiles,
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
            telemetry_quality: DwellQuality::Medium,
            voltage_quality: DwellQuality::Medium,
            confidence: conf,
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
            telemetry_quality: DwellQuality::Unavailable,
            voltage_quality: DwellQuality::Unavailable,
            confidence: 0.0,
        }
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
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 1100, voltage_step_mv: 25, lowest_safe_mv: 800 };
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 950 };
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&[1830u32, 1815, 1770], &d, &ForgePolicy::balanced(), probe);
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
        let d = FrontierDescent { safe_start_mv: 950, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), probe);
        assert!(r.profiles.godforge.is_some());
        assert!(r.profiles.brokkrs.is_some());
        assert!(r.profiles.deep_calm.is_some());
        assert!(r.profiles.log.iter().any(|l| l.contains("single sustainable clock")));
    }

    #[cfg(windows)]
    #[test]
    fn sim_no_valid_points_returns_safe_failure() {
        let probe = |_t: u32, _v: u32| unstable_sample(); // nothing ever stable
        let d = FrontierDescent { safe_start_mv: 1000, voltage_step_mv: 25, lowest_safe_mv: 700 };
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), probe);
        assert!(r.frontier.is_empty());
        assert!(r.profiles.godforge.is_none());
        assert!(r.profiles.brokkrs.is_none());
        assert!(r.profiles.deep_calm.is_none());
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
