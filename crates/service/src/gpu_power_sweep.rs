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

#[cfg(windows)]
use nidavellir_core::f2_observation::{
    F2Observation, F2QualificationCoverage, F2QualificationPattern, F2QualificationPhaseMetric,
    F2QualificationStrength, F2QualificationVerdict,
};
use nidavellir_core::gpu_sweep::StabilityResult;
use nidavellir_core::ipc::{DwellQuality, PowerSweepPoint, PowerSweepProgress};
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};
#[cfg(windows)]
use nidavellir_gpu_stress::{RenderGoldens, VfQualifierPattern, VfQualifierPhase, VfWorkload};
use tracing::{info, warn};

// Long enough for power to RAMP UP and stabilize (real loads like Heaven take
// seconds to reach their sustained draw — it's a ramp, not a spike). We discard
// the ramp and retain mean, sustained high-percentile and raw-maximum power separately.
const DWELL_MS: u64 = 15000;
const RAMP_DISCARD_MS: u128 = 6000;
/// Sustained high-power percentile used by F2 frontier decisions and profile calibration.
pub(crate) const POWER_PEAK_PERCENTILE: u32 = 99;
/// With fewer than 100 retained samples, a 99th percentile cannot discard a full top 1%.
/// The explicit conservative fallback is therefore the measured raw maximum.
const POWER_PEAK_MIN_SAMPLES: usize = 100;
/// Plausible GPU core-voltage range (mV). Samples outside are dropped from the
/// measured-voltage stats as sensor glitches (a 0 mV / out-of-range read is noise).
const VOLT_SANE_MIN_MV: u32 = 500;
const VOLT_SANE_MAX_MV: u32 = 1250;
#[cfg(windows)]
const F2_QUALIFIER_TARGET_TOL_MHZ: u32 = 30;
#[cfg(windows)]
const V8_GOLDEN_SAMPLE_MS: u64 = 2_000;
/// Each exact post-margin Apply pattern gets five minutes. A clean v8 three-pattern gate therefore
/// soaks the actual deployable pair for fifteen minutes; inconclusive debt may extend it.
#[cfg(windows)]
const F2_APPLY_QUALIFICATION_DWELL_MS: u64 = 300_000;

/// Upward-recovery budget when a clock's STARTING bin fails qualification. A start-bin rejection
/// usually means the warm-start/isotonic prediction (seeded by a neighbouring clock's boundary)
/// overshot this clock's real boundary — not that the clock is unsustainable. Climbing one
/// physical bin per retry, bounded, recovers the clock instead of discarding it. Generic search
/// parameter — never derived from any specific GPU's known-good points.
const F2_START_RECOVERY_MAX_CLIMBS: usize = 4;

/// Smallest sane VF bin voltage strictly above `mv`, or `None` at the top of the curve. Pure.
#[cfg(windows)]
fn f2_next_bin_above(sane_curve: &[(usize, u32, u32)], mv: u32) -> Option<u32> {
    sane_curve
        .iter()
        .map(|&(_, bin_mv, _)| bin_mv)
        .filter(|&bin_mv| bin_mv > mv)
        .min()
}
/// Initial telemetry threshold for a missing-valid-NVML-sample stall. Leva 1 records the signal only;
/// proactive reset remains disabled until the hardware gate proves the signal has acceptable
/// precision and the stress loop has a safe cooperative-cancellation path.
#[cfg(windows)]
pub(crate) const PREHANG_STALL_MS: u64 = 300;
/// Policy margin above the learned F2 voltage boundary. The requested millivolts are always snapped
/// upward to an exact physical VF-table bin before Apply; the effective margin is exposed in IPC.
#[cfg(windows)]
const APPLY_MARGIN_MV: u32 = 12;

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
    #[allow(dead_code)] // feeds `select_brokkrs_v2` (retained + unit-tested for the knowledge path)
    fn threshold(self) -> f64 {
        match self {
            SweepProfile::Conservative => 0.95,
            SweepProfile::Balanced => 0.85,
            SweepProfile::Aggressive => 0.70,
        }
    }
}

/// Instability severity, ordered (mirrors the L1/L2/L3 fail tiers). Stored per point
/// and per frontier so the algorithm can weigh a cheap SilentError differently from
/// an expensive HardReboot.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug,
)]
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

/// Live power-sweep BUTTON mode. Every mode traverses the same complete F2 clock×voltage frontier.
/// Modes differ only in dwell duration and independent validation passes at each discovered boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerSweepMode {
    Fast,
    #[default]
    Standard,
    Long,
}

impl PowerSweepMode {
    fn id(self) -> &'static str {
        match self {
            PowerSweepMode::Fast => "fast",
            PowerSweepMode::Standard => "standard",
            PowerSweepMode::Long => "long",
        }
    }

    /// Human label for the UI-facing note / logs.
    #[cfg(windows)]
    fn label(self) -> &'static str {
        match self {
            PowerSweepMode::Fast => "rápida",
            PowerSweepMode::Standard => "padrão",
            PowerSweepMode::Long => "longa",
        }
    }
    /// Mode tuning knobs `(max_probes, max_probes_per_target, validation_passes)`. These are
    /// hardware-relative PROBE counts (not fixed MHz); the per-probe verifier/dwell/Safe-Loop guards
    /// inside the shared core remain the safety net regardless of these values.
    #[cfg(windows)]
    fn tuning(self) -> (u32, u32, u32) {
        match self {
            PowerSweepMode::Fast => (FAST_MAX_PROBES, FAST_MAX_PROBES_PER_TARGET, 1),
            PowerSweepMode::Standard => (BUTTON_MAX_PROBES, BUTTON_MAX_PROBES_PER_TARGET, 1),
            PowerSweepMode::Long => (LONG_MAX_PROBES, LONG_MAX_PROBES_PER_TARGET, LONG_VALIDATION_PASSES,
            ),
        }
    }

    #[cfg(windows)]
    fn f2_policy(self) -> F2ForgeModePolicy {
        match self {
            PowerSweepMode::Fast => F2ForgeModePolicy {
                discovery_dwell_ms: 10_000,
                qualification_dwell_ms: 0,
                qualification_passes: 0,
                final_gate_dwell_ms: 0,
                final_gate_passes: 0,
            },
            PowerSweepMode::Standard => F2ForgeModePolicy {
                discovery_dwell_ms: 10_000,
                qualification_dwell_ms: 60_000,
                qualification_passes: F2_DESCENT_DETECTOR_PASSES,
                final_gate_dwell_ms: 0,
                final_gate_passes: 0,
            },
            PowerSweepMode::Long => F2ForgeModePolicy {
                discovery_dwell_ms: 10_000,
                qualification_dwell_ms: 60_000,
                qualification_passes: F2_DESCENT_DETECTOR_PASSES,
                final_gate_dwell_ms: 0,
                final_gate_passes: 0,
            },
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct F2ForgeModePolicy {
    discovery_dwell_ms: u64,
    qualification_dwell_ms: u64,
    qualification_passes: usize,
    final_gate_dwell_ms: u64,
    final_gate_passes: usize,
}

/// v13 single-detector descent: during the per-bin descent, run ONLY the binding detector — the
/// FIRST pattern in `REQUIRED_QUALIFICATION_PATTERNS` (Texture, the empirically-most-sensitive graceful
/// silent-error detector, confirmed across 3 HW runs to always fail first). This finds the boundary in
/// one qualification dwell per bin instead of the full set, ~halving the descent (the bulk of the run).
/// It ALSO sets the boundary-reconciliation confirmation count (`f2_required_qualification_passes`), so
/// the two always agree. The DEPLOYMENT guarantee is UNCHANGED: the exact-Apply gate
/// (`run_confirmed_f2_apply_qualification`) still runs the COMPLETE `REQUIRED_QUALIFICATION_PATTERNS`
/// set on the applied point, and both publish gates (`f2_profile_points_have_current_apply_qualification`)
/// require that full pass — a Texture-only boundary that another pattern would fail above is caught at
/// exact-Apply, which excludes the pair and the loop re-synthesizes a higher bin.
#[cfg(windows)]
const F2_DESCENT_DETECTOR_PASSES: usize = 1;

#[cfg(windows)]
fn f2_required_qualification_passes(policy: F2ForgeModePolicy) -> usize {
    policy.final_gate_passes.max(policy.qualification_passes)
}

#[cfg(windows)]
fn f2_profiles_meet_qualification(
    policy: F2ForgeModePolicy,
    profiles: &[Option<PowerSweepPoint>],
    confidence_threshold: f64,
) -> bool {
    let required_confirmations = f2_required_qualification_passes(policy) as u32;
    required_confirmations > 0
        && profiles.iter().all(Option::is_some)
        && profiles.iter().flatten().all(|point| {
            point.confidence.unwrap_or(0.0) >= confidence_threshold
                && point.validation_count.unwrap_or(0) >= required_confirmations
        })
        && f2_profile_points_have_current_apply_qualification(profiles)
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
        if let Ok(mut progress) = self.progress.lock() {
            if progress.running && progress.phase != "stopping" {
                progress.phase = "stopping".into();
                progress.note = Some(
                    "Parando o Forge e restaurando a GPU para stock com segurança…".into(),
                );
            }
        }
    }
    /// Explicit reset/recovery path: unblock the UI after a panic/TDR-interrupted worker and mark the
    /// visible Forge state as clean. Used only after the operator requested stock reset.
    pub fn recover_after_reset(&self, note: impl Into<String>) {
        self.stop.store(true, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut prog) = self.progress.lock() {
            let note = note.into();
            let mut reset = idle();
            reset.note = Some(note.clone());
            reset.log.push(note);
            *prog = reset;
        }
    }
    /// Start the live F2 forge in `Standard` mode (the plain `StartPowerSweep` IPC).
    pub fn start(&self, store: SafeLoopStore) -> bool {
        self.start_with_mode(store, PowerSweepMode::Standard)
    }
    /// Start the live multi-clock forge in a specific button `mode` (Fast / Standard / Long). All
    /// modes run the same complete physical frontier and fail-closed motor; only dwell duration and
    /// independent validation count vary.
    pub fn start_with_mode(&self, store: SafeLoopStore, mode: PowerSweepMode) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            // The live forge button now runs the F2 ANCHORED UNDERVOLT forge (the proven method for
            // power-bound cards F1 flatten-down cannot differentiate). The F1 `run_power_sweep` is kept
            // intact (Phase 3 decides its fate) but is no longer routed here.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(windows)]
                measure_multiclock_undervolt_forge(&progress, &stop, &store, mode);
                #[cfg(not(windows))]
                {
                    let _ = (&progress, &stop, &store, mode);
                }
            }));
            if result.is_err() {
                warn!("F2 power sweep worker panicked; marking Forge idle after fail-closed interruption");
                if let Ok(mut prog) = progress.lock() {
                    prog.running = false;
                    prog.phase = "interrupted".into();
                    prog.estimated_remaining_ms = None;
                    prog.estimated_total_upper_ms = None;
                    prog.profiles_qualified = false;
                    prog
                        .log
                        .push("Forge interrompido por falha interna/TDR; execute Reset para limpar recovery antes de novo Forge.".into());
                    prog.note = Some(
                        "Forge interrompido por falha interna/TDR; reset manual recomendado antes de continuar."
                            .into(),
                    );
                }
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
/// Bound the IPC-visible log so polling does not repeatedly clone and serialize an unbounded run.
/// Every completed evidence point remains durable in `f2_observations.jsonl`.
const FORGE_LIVE_LOG_TAIL: usize = 240;

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

/// Serialize a live or completed progress checkpoint and trim the log tail so the file stays small.
/// The stored `running` bit lets startup label an interrupted run without ever resurrecting it.
/// Returns `None` if serialization fails (caller simply skips the write).
fn encode_forge_state(gpu_key: &str, prog: &PowerSweepProgress) -> Option<String> {
    let mut snapshot = prog.clone();
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
        return ForgeStateLoad::SchemaMismatch { found: file.schema_version,
        };
    }
    if file.gpu_key != gpu_key {
        return ForgeStateLoad::GpuMismatch { stored: file.gpu_key,
        };
    }
    let mut prog = file.progress;
    if prog.is_undervolt
        && prog.profiles_qualified
        && !f2_profile_points_have_current_apply_qualification(&[
            prog.godforge,
            prog.brokkrs,
            prog.deep_calm,
        ])
    {
        prog.profiles_qualified = false;
        if prog.phase == "finished" {
            prog.phase = "provisional".into();
        }
        prog.note = Some(
            "Perfis F2 restaurados são anteriores à qualificação automática v8; execute Forge novamente."
                .into(),
        );
    }
    if prog.running {
        prog.phase = "interrupted".into();
        prog.estimated_remaining_ms = None;
        prog.estimated_total_upper_ms = None;
        prog.note = Some(format!(
            "A execução anterior foi interrompida; {} dwell(s) já aprendidos permanecem salvos.",
            prog.learned_points
        ));
    }
    prog.running = false;
    ForgeStateLoad::Loaded(Box::new(prog))
}

fn f2_profile_points_have_current_apply_qualification(
    profiles: &[Option<PowerSweepPoint>],
) -> bool {
    profiles.iter().all(Option::is_some)
        && profiles.iter().flatten().all(|point| {
            point.apply_qualified
                && point.apply_qualification_version
                    == Some(
                        nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION,
                    )
        })
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

/// Write a rich, human-readable log of the current/last F2 forge run to a timestamped file under the
/// data dir: run metadata, contract versions, published profiles, frontier summary, the live progress
/// log, and EVERY recorded dwell (clock/voltage/power/temp/outcome/pattern). Read-only — reads the
/// persisted observation store + the live progress; touches no hardware. Cross-platform (pure fs).
pub fn export_forge_log(
    prog: &PowerSweepProgress,
) -> Result<nidavellir_core::ipc::ForgeLogExport, String> {
    use nidavellir_core::f2_observation::{
        F2ObservationStore, F2_DISCOVERY_CONTRACT_VERSION, F2_QUALIFICATION_CONTRACT_VERSION,
    };
    let data_dir = nidavellir_core::safe_loop::default_data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create data dir: {e}"))?;
    let generated = nidavellir_core::f2_observation::now_rfc3339();
    let observations = F2ObservationStore::system().load_all();

    let fmt_profile = |label: &str, p: &Option<PowerSweepPoint>| -> String {
        match p {
            Some(pt) => {
                let clock = pt.target_clock_mhz.unwrap_or(pt.clock_mhz);
                let mv = pt.vf_table_voltage_mv.or(pt.boundary_voltage_mv).unwrap_or(pt.voltage_mv);
                let watts = pt.power_p99_w.unwrap_or(pt.power_w);
                format!(
                    "  {label:<14} {clock} MHz @ {mv} mV · p99 {watts:.0} W · p5 {:?} · p95 {:?}\n",
                    pt.p5_clock_mhz, pt.p95_clock_mhz
                )
            }
            None => format!("  {label:<14} (none)\n"),
        }
    };

    let mut out = String::new();
    out.push_str("===============================================================\n");
    out.push_str(" NIDAVELLIR - F2 FORGE RUN LOG\n");
    out.push_str("===============================================================\n");
    out.push_str(&format!("generated    : {generated}\n"));
    out.push_str(&format!(
        "contracts    : discovery v{F2_DISCOVERY_CONTRACT_VERSION} - qualification v{F2_QUALIFICATION_CONTRACT_VERSION}\n"
    ));
    out.push_str(&format!("mode         : {}\n", prog.mode.as_deref().unwrap_or("-")));
    out.push_str(&format!("phase        : {}\n", prog.phase));
    out.push_str(&format!("profiles_ok  : {}\n", prog.profiles_qualified));
    out.push_str(&format!("power cap    : {:.0} W\n", prog.power_limit_w));
    out.push_str(&format!(
        "Cmax / floor : {:?} MHz / {:?} MHz ({} frontier clocks)\n",
        prog.cmax_clock_mhz,
        prog.frontier_floor_clock_mhz,
        prog.frontier_clock_count.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
    ));
    out.push_str(&format!("elapsed      : {:.1} min\n", prog.elapsed_ms as f64 / 60_000.0));
    if let Some(note) = &prog.note {
        out.push_str(&format!("note         : {note}\n"));
    }
    out.push_str("\n-- Published profiles ------------------------------------------\n");
    out.push_str(&fmt_profile("Godforge", &prog.godforge));
    out.push_str(&fmt_profile("Brokkr's Best", &prog.brokkrs));
    out.push_str(&fmt_profile("Deep Calm", &prog.deep_calm));

    out.push_str("\n-- Live progress log -------------------------------------------\n");
    for line in &prog.log {
        out.push_str(line);
        out.push('\n');
    }

    out.push_str(&format!(
        "\n-- Recorded dwells ({} observations) ---------------------------\n",
        observations.len()
    ));
    out.push_str(
        "timestamp | kind | target@anchor | outcome | avg/p5/p95 MHz | avg/p99/peak W | temp | pattern/verdict/fail | flags\n",
    );
    let s = |v: Option<u32>| v.map(|x| x.to_string()).unwrap_or_else(|| "-".into());
    for o in &observations {
        let cov = o.qualification_coverage.as_ref();
        let pattern = cov.and_then(|c| c.pattern).map(|p| format!("{p:?}")).unwrap_or_else(|| "-".into());
        let verdict = cov.map(|c| format!("{:?}", c.verdict)).unwrap_or_else(|| "-".into());
        let fail = cov.and_then(|c| c.failure_phase.clone()).unwrap_or_default();
        let mut flags = Vec::new();
        if o.silent_error { flags.push("silent"); }
        if o.thermal_throttled { flags.push("throttle"); }
        if o.device_lost { flags.push("device_lost"); }
        if o.tdr_or_crash { flags.push("tdr"); }
        if o.blacklisted { flags.push("blacklisted"); }
        out.push_str(&format!(
            "{} | {:?} | {}@{} | {:?} | {}/{}/{} | {}/{}/{} | {} | {}/{}/{} | {}\n",
            o.timestamp, o.evidence_kind, o.target_mhz, o.anchor_mv, o.outcome,
            s(o.avg_clock_mhz), s(o.sustained_clock_mhz), s(o.sustained_upper_clock_mhz),
            o.watts.map(|w| w.to_string()).unwrap_or_else(|| "-".into()),
            o.power_p99_w.map(|w| format!("{w:.0}")).unwrap_or_else(|| "-".into()),
            s(o.max_watts),
            o.max_temp_c.map(|t| format!("{t:.0}C")).unwrap_or_else(|| "-".into()),
            pattern, verdict, fail, flags.join(","),
        ));
    }

    let stamp: String = generated
        .chars()
        .map(|c| if c == ':' || c == '.' { '-' } else { c })
        .collect();
    let path = data_dir.join(format!("nidavellir-forge-log-{stamp}.txt"));
    std::fs::write(&path, &out).map_err(|e| format!("write log: {e}"))?;
    let bytes = out.len() as u64;
    let raw = data_dir.join(nidavellir_core::f2_observation::F2_OBSERVATIONS_FILE);

    Ok(nidavellir_core::ipc::ForgeLogExport {
        path: path.display().to_string(),
        raw_observations_path: raw.display().to_string(),
        bytes,
        observation_count: observations.len(),
        note: format!(
            "Log salvo: {} dwell(s), {:.0} KB",
            observations.len(),
            bytes as f64 / 1024.0
        ),
    })
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
            let mut prog = *prog;
            if prog.is_undervolt && prog.profiles_qualified {
                let observations =
                    nidavellir_core::f2_observation::F2ObservationStore::system().load_all();
                let mut profiles = [prog.godforge, prog.brokkrs, prog.deep_calm];
                match publish_f2_profile_set_power_from_apply_qualification(
                    &mut profiles,
                    &observations,
                    None,
                    gpu_key,
                ) {
                    Ok(updated) if updated > 0 => {
                        prog.godforge = profiles[0];
                        prog.brokkrs = profiles[1];
                        prog.deep_calm = profiles[2];
                        prog.recommended = prog.brokkrs;
                        prog.log.push(format!(
                            "FORGE: p99 publicado restaurado pelo maior conjunto v8 aprovado ({updated} perfil(is) elevado(s))."
                        ));
                        save_forge_state(gpu_key, &prog);
                    }
                    Ok(_) => {}
                    Err(e) => warn!("forge_state p99 v8 refresh skipped: {e}"),
                }
            }
            info!(
                "forge_state loaded (gpu='{}', {} points)",
                gpu_key,
                prog.points.len()
            );
            Some(prog)
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

#[cfg(windows)]
fn current_gpu_key() -> String {
    if let Some(gpu) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
    {
        if let Some(uuid) = gpu.uuid {
            return format!("nvml:{uuid}");
        }
        return format!("nvml-index-{}:{}", gpu.index, gpu.name);
    }
    nidavellir_gpu_nvapi::read_curve()
        .map(|c| format!("nvapi:{}", c.name))
        .unwrap_or_else(|_| "unknown-gpu".into())
}

/// Build the power-sweep handle, seeded from the persisted forge result when one
/// matches this GPU. The GPU key is derived the same way the sweep keys its
/// knowledge (`read_curve().name`), so keys match reliably.
#[cfg(windows)]
pub fn restore_handle() -> PowerSweepHandle {
    let handle = PowerSweepHandle::default();
    let gpu_key = current_gpu_key();
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
pub fn clear_persisted_forge_state() -> Result<(), String> {
    match std::fs::remove_file(forge_state_path()) {
        Ok(()) => {
            info!("forge_state cleared by manual reset");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("forge_state cleanup failed: {e}")),
    }
}

#[cfg(not(windows))]
pub fn clear_persisted_forge_state() -> Result<(), String> {
    Ok(())
}

/// Read-only: load the persisted forge result for THIS GPU (the completed
/// `PowerSweepProgress` from `forge_state.json`), or `None` if absent/mismatched.
/// Path-independent (no `PowerSweepHandle` / `AppState` needed), so both the IPC
/// verifier and the `verify-applied` console subcommand can locate the applied
/// point's dwell stats. Never mutates anything.
#[cfg(windows)]
pub fn load_restored_progress() -> Option<PowerSweepProgress> {
    let gpu_key = current_gpu_key();
    load_forge_state(&gpu_key)
}

#[cfg(not(windows))]
pub fn load_restored_progress() -> Option<PowerSweepProgress> {
    None
}

#[cfg(windows)]
fn set(progress: &Arc<Mutex<PowerSweepProgress>>, mut p: PowerSweepProgress) {
    if p.log.len() > FORGE_LIVE_LOG_TAIL {
        let drop = p.log.len() - FORGE_LIVE_LOG_TAIL;
        p.log.drain(0..drop);
    }
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
    ctx: &mut nidavellir_gpu_stress::GpuCtx) -> FailTier {
    match res {
        StabilityResult::SilentError | StabilityResult::Unstable => FailTier::L1Instability,
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
// Several stats fields are the full dwell telemetry record `load_and_measure` always captures;
// the multi-clock forge path consumes the verdict/clock/power summary (the richer fields flow into
// frontier points via `measured_to_probe`/`probe_to_point`). Kept intact as the measurement contract.
#[cfg(windows)]
#[allow(dead_code)]
struct Measured {
    result: StabilityResult,
    cancelled: bool,
    clock_mhz: u32,
    power_w: f32,
    max_power_w: f32,
    power_p99_w: Option<f32>,
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
    p95_clock_mhz: u32,
    /// Ramp-filtered + sanity-checked measured-voltage stats (telemetry only).
    volt_min_mv: Option<u32>,
    volt_avg_mv: Option<u32>,
    volt_max_mv: Option<u32>,
    volt_sample_count: u32,
    /// Steady-state temperature start/end/mean (°C), if NVML reported it.
    start_temp_c: Option<f32>,
    end_temp_c: Option<f32>,
    avg_temp_c: Option<f32>,
    max_temp_c: Option<f32>,
    thermal_throttled: bool,
    /// Workload-side render coverage. Diagnostic only; absent when the render panicked before
    /// returning its report.
    render_frames: Option<u64>,
    render_fps: Option<f64>,
    qualification_coverage: Option<F2QualificationCoverage>,
    prehang_stall_detected: bool,
}

impl Measured {
    /// A no-data result (device init failed / no samples) carrying only the legacy
    /// voltage max. All richer stats are absent.
    fn degenerate(result: StabilityResult, volt_mv: u32) -> Self {
        Measured {
            result,
            cancelled: false,
            clock_mhz: 0,
            power_w: 0.0,
            max_power_w: 0.0,
            power_p99_w: None,
            power_std_w: 0.0,
            capped_frac: 0.0,
            volt_mv,
            sample_count: 0,
            duration_ms: 0,
            min_clock_mhz: 0,
            p5_clock_mhz: 0,
            p95_clock_mhz: 0,
            volt_min_mv: None,
            volt_avg_mv: None,
            volt_max_mv: None,
            volt_sample_count: 0,
            start_temp_c: None,
            end_temp_c: None,
            avg_temp_c: None,
            max_temp_c: None,
            thermal_throttled: false,
            render_frames: None,
            render_fps: None,
            qualification_coverage: None,
            prehang_stall_detected: false,
        }
    }
}

#[cfg(windows)]
fn prehang_stall_signal(saw_valid_sample: bool, elapsed_since_valid_ms: u64) -> bool {
    saw_valid_sample && elapsed_since_valid_ms >= PREHANG_STALL_MS
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

#[cfg(windows)]
fn f2_apply_key(point: &PowerSweepPoint) -> Option<(u32, u32)> {
    Some((
        point.target_clock_mhz.unwrap_or(point.clock_mhz),
        point.vf_table_voltage_mv?,
    ))
}

#[cfg(windows)]
fn f2_unique_profile_points(
    profiles: &[Option<PowerSweepPoint>],
) -> Vec<PowerSweepPoint> {
    let mut seen = std::collections::HashSet::new();
    profiles
        .iter()
        .flatten()
        .copied()
        .filter(|point| f2_apply_key(point).is_some_and(|key| seen.insert(key)))
        .collect()
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct F2RegimeSupport {
    observed_p95_mhz: u32,
    support_target_mhz: u32,
    required_apply_mv: u32,
}

#[cfg(windows)]
fn f2_boundary_point_is_qualified(
    point: &PowerSweepPoint,
    required_confirmations: u32,
    confidence_threshold: f64,
) -> bool {
    point.validation_count.unwrap_or(0) >= required_confirmations
        && point.confidence.unwrap_or(0.0) >= confidence_threshold
}

/// Resolve the electrical regime exercised by a calibrated point. Any sustained upper-clock regime
/// above the configured target maps to the nearest measured target at/above p95; there is no
/// one-bin deployability tolerance.
/// Required voltage is the conservative maximum Apply anchor across that upward target span.
#[cfg(windows)]
fn f2_regime_support(
    point: &PowerSweepPoint,
    frontier: &[(PowerSweepPoint, f64)],
) -> Result<F2RegimeSupport, &'static str> {
    let target_mhz = point
        .target_clock_mhz
        .ok_or("candidate has no configured target")?;
    let observed_p95_mhz = point
        .p95_clock_mhz
        .filter(|clock| *clock > 0)
        .ok_or("candidate has no measured p95")?;
    let support_target_mhz = if observed_p95_mhz > target_mhz {
        frontier
            .iter()
            .filter_map(|(candidate, _)| candidate.target_clock_mhz)
            .filter(|candidate_target| *candidate_target >= observed_p95_mhz)
            .min()
            .ok_or("no measured target supports the observed p95 regime")?
    } else {
        target_mhz
    };
    // The requirement is computed from PRE-lift (base) apply anchors: the lifted extra on a
    // higher target covers that target's OWN overshoot regime, which this point's hardware
    // (clock-capped at its sustained p95) never reaches — using post-lift values would ratchet
    // the whole frontier up to the top regime's voltage.
    let required_apply_mv = frontier
        .iter()
        .filter_map(|(candidate, _)| {
            let candidate_target = candidate.target_clock_mhz?;
            let apply_mv = candidate.base_apply_mv.or(candidate.vf_table_voltage_mv)?;
            (candidate_target >= target_mhz && candidate_target <= support_target_mhz)
                .then_some(apply_mv)
        })
        .max()
        .ok_or("observed p95 regime has no measured Apply anchor")?;
    Ok(F2RegimeSupport {
        observed_p95_mhz,
        support_target_mhz,
        required_apply_mv,
    })
}

#[cfg(windows)]
fn f2_regime_candidate_refusal(
    point: &PowerSweepPoint,
    frontier: &[(PowerSweepPoint, f64)],
    require_boundary_qualification: bool,
    required_confirmations: u32,
    confidence_threshold: f64,
) -> Option<String> {
    if require_boundary_qualification
        && !f2_boundary_point_is_qualified(
            point,
            required_confirmations,
            confidence_threshold,
        )
    {
        return Some("its own frontier boundary lacks current v8 qualification".into());
    }
    let support = match f2_regime_support(point, frontier) {
        Ok(support) => support,
        Err(reason) => return Some(reason.into()),
    };
    let support_point = frontier.iter().find_map(|(candidate, _)| {
        (candidate.target_clock_mhz == Some(support.support_target_mhz)).then_some(candidate)
    });
    if require_boundary_qualification
        && !support_point.is_some_and(|candidate| {
            f2_boundary_point_is_qualified(
                candidate,
                required_confirmations,
                confidence_threshold,
            )
        })
    {
        return Some(format!(
            "{} MHz p95 maps to {} MHz, whose frontier is failed or inconclusive",
            support.observed_p95_mhz, support.support_target_mhz
        ));
    }
    let Some(apply_mv) = point.vf_table_voltage_mv else {
        return Some("candidate has no exact Apply anchor".into());
    };
    (apply_mv < support.required_apply_mv).then(|| {
        format!(
            "{} MHz target sustains p95 {} MHz but Apply {} mV is below the {} mV required by the {} MHz regime",
            point.target_clock_mhz.unwrap_or(point.clock_mhz),
            support.observed_p95_mhz,
            apply_mv,
            support.required_apply_mv,
            support.support_target_mhz
        )
    })
}

#[cfg(windows)]
fn f2_regime_dependent_apply_keys(
    failed_key: (u32, u32),
    frontier: &[(PowerSweepPoint, f64)],
) -> Vec<(u32, u32)> {
    frontier
        .iter()
        .filter_map(|(point, _)| {
            let key = f2_apply_key(point)?;
            let support = f2_regime_support(point, frontier).ok()?;
            (key != failed_key
                && support.support_target_mhz == failed_key.0
                && key.1 <= failed_key.1)
                .then_some(key)
        })
        .collect()
}

/// 95th-percentile (upper) sustained clock of a sample set. This is the high counterpart to p5 and
/// describes the boost regime reached repeatedly rather than a single maximum sample.
fn p95_clock_mhz(clocks: &[u32]) -> Option<u32> {
    if clocks.is_empty() {
        return None;
    }
    let mut s = clocks.to_vec();
    s.sort_unstable();
    let idx = (((s.len() - 1) as f64) * 0.95).ceil() as usize;
    Some(s[idx.min(s.len() - 1)])
}

/// Sustained high-power percentile from every retained post-ramp dwell sample.
///
/// Uses the nearest-rank definition. For fewer than [`POWER_PEAK_MIN_SAMPLES`] samples,
/// p99 cannot discard a complete top 1%, so the measured raw maximum is returned. Empty or
/// wholly non-finite input returns `None`; callers must fail closed rather than invent power.
fn sustained_power_percentile(samples: &[f32]) -> Option<f32> {
    let mut sorted: Vec<f32> = samples
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f32::total_cmp);
    if sorted.len() < POWER_PEAK_MIN_SAMPLES {
        return sorted.last().copied();
    }
    let percentile = POWER_PEAK_PERCENTILE as usize;
    let rank = percentile
        .saturating_mul(sorted.len())
        .saturating_add(99)
        / 100;
    sorted.get(rank.saturating_sub(1).min(sorted.len() - 1)).copied()
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
#[allow(dead_code)] // dwell-quality combiner — retained + unit-tested
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderStressPurpose {
    PowerCharacterization,
    VfQualification(VfQualifierPattern, RenderGoldens),
}

#[cfg(windows)]
type PhaseSample = (u32, f32, bool, Option<f32>, u8, bool);

#[cfg(windows)]
fn avg_power_for_phases(samples: &[PhaseSample], phases: &[VfQualifierPhase]) -> Option<f32> {
    let mut total = 0.0f32;
    let mut count = 0u32;
    for sample in samples {
        let Some(phase) = VfQualifierPhase::from_code(sample.4) else { continue };
        if phases.contains(&phase) {
            total += sample.1;
            count = count.saturating_add(1);
        }
    }
    (count > 0).then(|| total / count as f32)
}

#[cfg(windows)]
fn pct_u32(mut values: Vec<u32>, pct: f64) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let idx = (((values.len() - 1) as f64) * pct).round() as usize;
    values.get(idx).copied()
}

#[cfg(windows)]
fn pct_f32(mut values: Vec<f32>, pct: f64) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((values.len() - 1) as f64) * pct).round() as usize;
    values.get(idx).copied()
}

#[cfg(windows)]
fn avg_f32(values: &[f32]) -> Option<f32> {
    (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
}

#[cfg(windows)]
fn f2_pattern_from_stress(
    pattern: VfQualifierPattern,
) -> (F2QualificationStrength, Option<F2QualificationPattern>, &'static str) {
    match pattern {
        VfQualifierPattern::Fsgl1 => (F2QualificationStrength::Fsgl1, None, "fsgl1"),
        VfQualifierPattern::Fsgl2A => (
            F2QualificationStrength::Fsgl2,
            Some(F2QualificationPattern::A),
            "fsgl2-a",
        ),
        VfQualifierPattern::Fsgl2B => (
            F2QualificationStrength::Fsgl2,
            Some(F2QualificationPattern::B),
            "fsgl2-b",
        ),
        VfQualifierPattern::Fsgl3A => (
            F2QualificationStrength::Fsgl3,
            Some(F2QualificationPattern::A),
            "fsgl3-a",
        ),
        VfQualifierPattern::Fsgl3B => (
            F2QualificationStrength::Fsgl3,
            Some(F2QualificationPattern::B),
            "fsgl3-b",
        ),
        VfQualifierPattern::V8HighFps => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::HighFps),
            "v8-high-fps",
        ),
        VfQualifierPattern::V8Texture => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::Texture),
            "v8-texture",
        ),
        VfQualifierPattern::V8Transitions => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::Transitions),
            "v8-transitions",
        ),
        VfQualifierPattern::V8Memory => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::Memory),
            "v8-memory",
        ),
        VfQualifierPattern::Endurance => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::Endurance),
            "endurance",
        ),
        VfQualifierPattern::TransitionShock => (
            F2QualificationStrength::Fsgl4,
            Some(F2QualificationPattern::TransitionShock),
            "transition-shock",
        ),
    }
}

#[cfg(windows)]
fn qualification_coverage_from_run(
    result: StabilityResult,
    phase_reports: &[nidavellir_gpu_stress::VfPhaseReport],
    samples: &[PhaseSample],
    target_mhz: Option<u32>,
    pattern: VfQualifierPattern,
) -> F2QualificationCoverage {
    // Coverage denominator is pattern-specific: legacy FSGL plans exercise 8 phases, the
    // current v8 plans exercise 9 (FrameCadence). A fixed count would mark legacy runs
    // incomplete or let a v8 run skip its cadence phase.
    let expected_phases = nidavellir_gpu_stress::qualifier_expected_phases(pattern);
    let mut seen = [false; VfQualifierPhase::COUNT];
    let mut checksum_count = 0u32;
    let mut compute_check_count = 0u32;
    let mut failure_phase = None;
    for report in phase_reports {
        checksum_count = checksum_count.saturating_add(report.checksum_count);
        if report.phase == VfQualifierPhase::ComputeBurst {
            compute_check_count = compute_check_count.saturating_add(report.checksum_count);
        }
        if failure_phase.is_none() && !report.result.is_stable() {
            failure_phase = Some(report.phase.label().to_string());
        }
        if report.result.is_stable() {
            seen[report.phase.code() as usize] = true;
        }
    }
    let phases_completed = seen.iter().filter(|seen| **seen).count() as u32;
    let phase_sample_count = samples
        .iter()
        .filter(|sample| VfQualifierPhase::from_code(sample.4).is_some())
        .count() as u32;
    let target_residency_frac = target_mhz.and_then(|target| {
        let target_floor = target.saturating_sub(F2_QUALIFIER_TARGET_TOL_MHZ);
        let mut total = 0u32;
        let mut resident = 0u32;
        for sample in samples {
            if VfQualifierPhase::from_code(sample.4).is_none() {
                continue;
            }
            total = total.saturating_add(1);
            if sample.0 >= target_floor {
                resident = resident.saturating_add(1);
            }
        }
        (total > 0).then(|| resident as f32 / total as f32)
    });
    let heavy_power = avg_power_for_phases(
        samples,
        &[VfQualifierPhase::PowerOpening, VfQualifierPhase::HeavySpike, VfQualifierPhase::PowerClosing],
    );
    let light_power = avg_power_for_phases(samples, &[VfQualifierPhase::BoostEdge]);
    let heavy_light_power_delta_w = heavy_power.zip(light_power).map(|(heavy, light)| heavy - light);
    let boost_samples = samples
        .iter()
        .filter(|sample| VfQualifierPhase::from_code(sample.4) == Some(VfQualifierPhase::BoostEdge))
        .count();
    let boost_capped_fraction = (boost_samples > 0).then(|| {
        samples
            .iter()
            .filter(|sample| {
                VfQualifierPhase::from_code(sample.4) == Some(VfQualifierPhase::BoostEdge)
                    && sample.2
            })
            .count() as f32
            / boost_samples as f32
    });
    let (strength, qualifier_pattern, phase_pattern) = f2_pattern_from_stress(pattern);
    let phase_metrics = phase_reports
        .iter()
        .map(|report| {
            let phase_samples: Vec<_> = samples
                .iter()
                .copied()
                .filter(|sample| VfQualifierPhase::from_code(sample.4) == Some(report.phase))
                .collect();
            let clocks: Vec<u32> = phase_samples.iter().map(|sample| sample.0).collect();
            let powers: Vec<f32> = phase_samples.iter().map(|sample| sample.1).collect();
            let temperatures: Vec<f32> = phase_samples.iter().filter_map(|sample| sample.3).collect();
            let target_residency_pct = target_mhz.and_then(|target| {
                let target_floor = target.saturating_sub(F2_QUALIFIER_TARGET_TOL_MHZ);
                (!phase_samples.is_empty()).then(|| {
                    let resident = phase_samples
                        .iter()
                        .filter(|sample| sample.0 >= target_floor)
                        .count();
                    resident as f32 * 100.0 / phase_samples.len() as f32
                })
            });
            let power_capped_fraction = (!phase_samples.is_empty()).then(|| {
                phase_samples.iter().filter(|sample| sample.2).count() as f32
                    / phase_samples.len() as f32
            });
            let coverage_status = if !report.result.is_stable() {
                "fail"
            } else if phase_samples.is_empty() {
                "telemetry_missing"
            } else if report.checksum_count == 0 {
                "checksum_missing"
            } else {
                "pass"
            };
            F2QualificationPhaseMetric {
                phase_name: report.phase.label().to_string(),
                phase_pattern: phase_pattern.to_string(),
                duration_ms: report.elapsed_ms,
                frame_count: report.frames,
                checksum_count: report.checksum_count,
                compute_check_count: if report.phase == VfQualifierPhase::ComputeBurst {
                    report.checksum_count
                } else {
                    0
                },
                clock_avg: (!clocks.is_empty())
                    .then(|| clocks.iter().map(|clock| *clock as f32).sum::<f32>() / clocks.len() as f32),
                clock_p5: pct_u32(clocks.clone(), 0.05),
                clock_p50: pct_u32(clocks.clone(), 0.50),
                clock_p95: pct_u32(clocks, 0.95),
                target_residency_pct,
                power_avg: avg_f32(&powers),
                power_p95: pct_f32(powers, 0.95),
                power_capped_fraction,
                temperature_avg: avg_f32(&temperatures),
                temperature_max: pct_f32(temperatures, 1.0),
                coverage_status: coverage_status.to_string(),
            }
        })
        .collect::<Vec<_>>();

    let (verdict, reason) = if !result.is_stable() {
        (F2QualificationVerdict::Fail, Some("workload_failed".to_string()))
    } else if phases_completed < expected_phases {
        (F2QualificationVerdict::Inconclusive, Some("phase_not_completed".to_string()))
    } else if checksum_count < expected_phases {
        (F2QualificationVerdict::Inconclusive, Some("checksum_coverage_low".to_string()))
    } else if phase_sample_count == 0 {
        (F2QualificationVerdict::Inconclusive, Some("telemetry_missing".to_string()))
    } else if target_residency_frac.is_some_and(|frac| frac < 0.35) {
        (F2QualificationVerdict::Inconclusive, Some("target_residency_low".to_string()))
    } else if boost_capped_fraction.is_some_and(|frac| frac > 0.20) {
        (
            F2QualificationVerdict::Inconclusive,
            Some("boost_edge_power_bound".to_string()),
        )
    } else if heavy_light_power_delta_w.is_some_and(|delta| delta < 3.0) {
        (F2QualificationVerdict::Inconclusive, Some("phase_contrast_low".to_string()))
    } else {
        (F2QualificationVerdict::Pass, None)
    };

    F2QualificationCoverage {
        strength,
        pattern: qualifier_pattern,
        pass_index: 0,
        verdict,
        phases_completed,
        phases_expected: expected_phases,
        checksum_count,
        sample_count: phase_sample_count,
        compute_check_count,
        target_residency_frac,
        heavy_light_power_delta_w,
        failure_phase,
        retry_count: 0,
        reason,
        phase_metrics,
    }
}

#[cfg(windows)]
fn load_and_measure(ms: u64) -> Measured {
    load_and_measure_for(
        ms,
        RenderStressPurpose::PowerCharacterization,
        None,
        None,
    )
}

#[cfg(windows)]
fn f2_apply_anchor_with_margin(
    curve: &[(usize, u32, u32)],
    target_mhz: u32,
    boundary_mv: u32,
) -> u32 {
    let mut valid_anchors: Vec<u32> = curve
        .iter()
        .filter(|(_, mv, base_mhz)| {
            *mv >= boundary_mv && *base_mhz < target_mhz && is_sane_core_point(*mv, *base_mhz)
        })
        .map(|(_, mv, _)| *mv)
        .collect();
    valid_anchors.sort_unstable();
    valid_anchors.dedup();
    let requested_mv = boundary_mv.saturating_add(APPLY_MARGIN_MV);
    valid_anchors
        .iter()
        .copied()
        .find(|mv| *mv >= requested_mv)
        .or_else(|| valid_anchors.last().copied())
        .unwrap_or(boundary_mv)
}

#[cfg(windows)]
fn apply_f2_margin_policy(
    points: &mut [(PowerSweepPoint, f64)],
    curve: &[(usize, u32, u32)],
) -> Vec<String> {
    for (point, _) in points.iter_mut() {
        let Some(target_mhz) = point.target_clock_mhz else {
            continue;
        };
        let boundary_mv = point
            .boundary_voltage_mv
            .or(point.vf_table_voltage_mv)
            .unwrap_or(point.voltage_mv);
        let apply_mv = f2_apply_anchor_with_margin(curve, target_mhz, boundary_mv);
        point.boundary_voltage_mv = Some(boundary_mv);
        point.vf_table_voltage_mv = Some(apply_mv);
        point.base_apply_mv = Some(apply_mv);
        point.apply_margin_mv = Some(apply_mv.saturating_sub(boundary_mv));
    }
    // v13: the regime lift was removed. Every dwell now runs under an absolute NVML max-clock
    // ceiling at its target, so the sustained regime IS the target by construction (p95 == target;
    // an over-target p95 fails the dwell as Inconclusive). The strict p95 reconciliation
    // (`f2_regime_support`) stays untouched as a dormant fail-closed net: it can only fire if a
    // ceiling silently failed, and excluding such a candidate is correct.
    Vec::new()
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct F2ApplyPowerBackfill {
    target_mhz: u32,
    apply_mv: u32,
    reference_offset_mhz: i32,
}

#[cfg(windows)]
fn missing_f2_apply_power_backfills(
    points: &[(PowerSweepPoint, f64)],
    observations: &[F2Observation],
    gpu_key: &str,
) -> Vec<F2ApplyPowerBackfill> {
    points
        .iter()
        .filter_map(|(point, _)| {
            let target_mhz = point.target_clock_mhz.unwrap_or(point.clock_mhz);
            let apply_mv = point.vf_table_voltage_mv?;
            nidavellir_core::f2_observation::current_discovery_observation_at_anchor(
                observations,
                target_mhz,
                apply_mv,
                gpu_key,
            )
            .is_none()
            .then_some(F2ApplyPowerBackfill {
                target_mhz,
                apply_mv,
                reference_offset_mhz: point.offset_mhz,
            })
        })
        .collect()
}

#[cfg(windows)]
fn calibrate_f2_profile_power(
    points: &mut [(PowerSweepPoint, f64)],
    observations: &[F2Observation],
    gpu_key: &str,
) -> Result<(), String> {
    for (point, _) in points {
        let target_mhz = point.target_clock_mhz.unwrap_or(point.clock_mhz);
        let apply_mv = point
            .vf_table_voltage_mv
            .ok_or_else(|| format!("{target_mhz} MHz has no apply VF bin"))?;
        let measured = nidavellir_core::f2_observation::current_discovery_observation_at_anchor(
            observations,
            target_mhz,
            apply_mv,
            gpu_key,
        )
        .ok_or_else(|| {
            format!(
                "{target_mhz} MHz @ {apply_mv} mV has no current, reset-clean, thermally valid \
                 discovery-v4 confirmed sustained-p99 power measurement"
            )
        })?;
        let mean_power = measured.watts.unwrap_or(0) as f32;
        let peak_power = measured.max_watts.unwrap_or(0) as f32;
        let power_p99 = measured.power_p99_w.ok_or_else(|| {
            format!(
                "{target_mhz} MHz @ {apply_mv} mV has no measured sustained-p99 power"
            )
        })?;
        point.clock_mhz = measured.avg_clock_mhz.unwrap_or(point.clock_mhz);
        point.p5_clock_mhz = measured.sustained_clock_mhz.or(point.p5_clock_mhz);
        point.p95_clock_mhz = measured
            .sustained_upper_clock_mhz
            .or(point.p95_clock_mhz);
        let sustained_clock = point.p5_clock_mhz.unwrap_or(point.clock_mhz);
        point.power_w = mean_power;
        point.max_power_w = peak_power;
        point.power_p99_w = Some(power_p99);
        point.perf_per_watt = sustained_clock as f64 / power_p99 as f64;
        point.dwell_duration_ms = measured.dwell_duration_ms;
        point.dwell_sample_count = measured.sample_count;
        point.max_temp_c = measured.max_temp_c;
        point.thermal_throttled = measured.thermal_throttled;
    }
    Ok(())
}

#[cfg(windows)]
fn publish_f2_profile_power_from_apply_qualification(
    point: &mut PowerSweepPoint,
    observations: &[F2Observation],
    run_id: Option<&str>,
    gpu_key: &str,
) -> Result<bool, String> {
    let target_mhz = point.target_clock_mhz.unwrap_or(point.clock_mhz);
    let apply_mv = point
        .vf_table_voltage_mv
        .ok_or_else(|| format!("{target_mhz} MHz has no exact Apply anchor"))?;
    let discovery_p99 = point
        .power_p99_w
        .filter(|power| power.is_finite() && *power > 0.0)
        .ok_or_else(|| format!("{target_mhz} MHz @ {apply_mv} mV has no calibrated p99"))?;
    let qualification_p99 = match run_id {
        Some(run_id) => {
            nidavellir_core::f2_observation::current_apply_qualification_p99_at_anchor(
                observations,
                run_id,
                target_mhz,
                apply_mv,
                gpu_key,
            )
        }
        None => nidavellir_core::f2_observation::highest_apply_qualification_p99_at_anchor(
            observations,
            target_mhz,
            apply_mv,
            gpu_key,
        ),
    }
    .ok_or_else(|| {
        format!("{target_mhz} MHz @ {apply_mv} mV has no complete current v8 p99 measurement")
    })?;
    let published_p99 = discovery_p99.max(qualification_p99);
    let changed = published_p99 > discovery_p99;
    point.power_p99_w = Some(published_p99);
    point.perf_per_watt =
        point.p5_clock_mhz.unwrap_or(point.clock_mhz) as f64 / published_p99 as f64;
    Ok(changed)
}

#[cfg(windows)]
fn publish_f2_profile_set_power_from_apply_qualification(
    profiles: &mut [Option<PowerSweepPoint>],
    observations: &[F2Observation],
    run_id: Option<&str>,
    gpu_key: &str,
) -> Result<usize, String> {
    let mut changed = 0;
    for point in profiles.iter_mut().flatten() {
        changed += usize::from(publish_f2_profile_power_from_apply_qualification(
            point,
            observations,
            run_id,
            gpu_key,
        )?);
    }
    Ok(changed)
}

#[cfg(windows)]
fn capture_fsgl3_render_goldens() -> Result<RenderGoldens, String> {
    fn capture(label: &str, workload: VfWorkload) -> Result<(u32, u32), String> {
        let ctx = nidavellir_gpu_stress::GpuCtx::new()
            .map_err(|e| format!("{label}: GpuCtx init failed: {e}"))?;
        ctx.capture_one_golden(workload, V8_GOLDEN_SAMPLE_MS)
            .map_err(|e| format!("{label}: {e}"))
    }

    let stream = capture("texture-stream golden", VfWorkload::TextureStream)?;
    Ok(RenderGoldens {
        power: capture("power golden", VfWorkload::PowerRender)?.0,
        boost: capture("boost golden", VfWorkload::BoostEdge)?.0,
        texrop: capture("texture/ROP golden", VfWorkload::TextureRop)?.0,
        cadence: capture("frame-cadence golden", VfWorkload::FrameCadence)?.0,
        geometry: capture("geometry/depth golden", VfWorkload::GeometryDepth)?.0,
        stream: stream.0,
        stream_frame_reference_ms: stream.1,
    })
}

#[cfg(windows)]
fn load_and_measure_for(
    ms: u64,
    purpose: RenderStressPurpose,
    target_mhz: Option<u32>,
    cancel: Option<&AtomicBool>,
) -> Measured {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicU32, AtomicU8};
    // FRESH wgpu context per measurement. The FurMark-class render reliably runs
    // ONCE on a fresh device but TDRs (device lost, unrecoverable) if a SECOND
    // heavy render is issued on the SAME GpuCtx — so we create + drop a context per
    // dwell. The clock offset is applied via NVAPI on the hardware, independent of
    // the wgpu device, so a fresh context still measures the applied operating point.
    let ctx = match nidavellir_gpu_stress::GpuCtx::new() {
        Ok(c) => c,
        Err(_) => return Measured::degenerate(StabilityResult::Crash, 0),
    };
    let sampler_stop = Arc::new(AtomicBool::new(false));
    // Collect raw samples in the sampler thread for precise stats (mean/max/std + the
    // richer min/p5/temperature stats). Tuple: (clock_mhz, power_w, capped, temp_c, qualifier_phase).
    let samples: Arc<Mutex<Vec<PhaseSample>>> = Arc::new(Mutex::new(Vec::new()));
    let phase_state = Arc::new(AtomicU8::new(VfQualifierPhase::NONE_CODE));
    let prehang_stall = Arc::new(AtomicBool::new(false));
    let volt = Arc::new(AtomicU32::new(0));
    // Ramp-filtered + sanity-checked voltage samples → measured-voltage telemetry
    // (avg/min/max/count). The legacy `volt` AtomicU32 max is kept UNCHANGED so the
    // apply key (which snaps `volt_mv`) is unaffected.
    let volts: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let (s2, smp, vlt, vsmp, phase_for_sampler, prehang_for_sampler) = (
        sampler_stop.clone(),
        samples.clone(),
        volt.clone(),
        volts.clone(),
        phase_state.clone(),
        prehang_stall.clone(),
    );
    let t0 = std::time::Instant::now();
    let sampler = std::thread::spawn(move || {
        let mut tick: u32 = 0;
        let mut saw_valid_sample = false;
        let mut last_valid_sample = std::time::Instant::now();
        while !s2.load(Ordering::SeqCst) {
            let mut valid_sample = false;
            if let Some(r) = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml().into_iter().next() {
                if let (Some(c), Some(p)) = (r.core_clock_mhz, r.power_w) {
                    valid_sample = true;
                    saw_valid_sample = true;
                    last_valid_sample = std::time::Instant::now();
                    // Discovery discards ramp-up for steady-state power. Qualification keeps the
                    // opening/transition samples because the phase changes are the workload.
                    if matches!(purpose, RenderStressPurpose::VfQualification(_, _))
                        || t0.elapsed().as_millis() >= RAMP_DISCARD_MS
                    {
                        if let Ok(mut v) = smp.lock() {
                            v.push((
                                c,
                                p,
                                r.power_capped == Some(true),
                                r.temperature_c,
                                phase_for_sampler.load(Ordering::SeqCst),
                                r.thermal_throttled == Some(true),
                            ));
                        }
                    }
                }
            }
            if !valid_sample
                && prehang_stall_signal(
                    saw_valid_sample,
                    last_valid_sample.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                )
            {
                prehang_for_sampler.store(true, Ordering::SeqCst);
            }
            // Voltage via NVAPI is heavier (re-inits), so sample it sparsely.
            tick += 1;
            if tick.is_multiple_of(16) {
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
    let render = catch_unwind(AssertUnwindSafe(|| match purpose {
        RenderStressPurpose::PowerCharacterization => match cancel {
            Some(token) => ctx.run_render_stress_with_cancel(ms, token),
            None => ctx.run_render_stress(ms),
        },
        RenderStressPurpose::VfQualification(pattern, goldens) => {
            ctx.run_vf_qualifier_stress_with_phase_pattern_goldens_and_cancel(
                ms,
                phase_state.as_ref(),
                pattern,
                Some(goldens),
                cancel,
            )
        }
    }));
    let (res, phase_reports, render_frames, render_fps) = match render {
        Ok(r) => {
            if let Some(phase) = r.failure_phase {
                warn!("VF qualifier failed during phase {}", phase.label());
            }
            (
                r.result,
                r.phase_reports,
                Some(r.frames),
                (r.fps.is_finite() && r.fps >= 0.0).then_some(r.fps),
            )
        }
        Err(_) => (StabilityResult::Crash, Vec::new(), None, None),
    };
    let cancelled = cancel.is_some_and(|token| token.load(Ordering::SeqCst));
    sampler_stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();

    let volt_mv = volt.load(Ordering::SeqCst);
    let prehang_stall_detected = prehang_stall.load(Ordering::SeqCst);
    let duration_ms = t0.elapsed().as_millis() as u64;
    let v = samples.lock().map(|g| g.clone()).unwrap_or_default();
    let qualification_coverage = match purpose {
        RenderStressPurpose::VfQualification(pattern, _) => {
            Some(qualification_coverage_from_run(res, &phase_reports, &v, target_mhz, pattern))
        }
        RenderStressPurpose::PowerCharacterization => None,
    };
    let volt_samples = volts.lock().map(|g| g.clone()).unwrap_or_default();
    let (volt_min_mv, volt_avg_mv, volt_max_mv, volt_sample_count) = match voltage_stats(&volt_samples)
    {
        Some((mn, avg, mx, c)) => (Some(mn), Some(avg), Some(mx), c),
        None => (None, None, None, 0),
    };
    if v.is_empty() {
        return Measured {
            cancelled,
            volt_min_mv,
            volt_avg_mv,
            volt_max_mv,
            volt_sample_count,
            duration_ms,
            render_frames,
            render_fps,
            qualification_coverage,
            prehang_stall_detected,
            ..Measured::degenerate(res, volt_mv)
        };
    }
    let n = v.len() as f32;
    let clock = (v.iter().map(|s| s.0 as u64).sum::<u64>() / v.len() as u64) as u32;
    let mean_p = v.iter().map(|s| s.1).sum::<f32>() / n;
    let max_p = v.iter().map(|s| s.1).fold(0.0f32, f32::max);
    let powers: Vec<f32> = v.iter().map(|s| s.1).collect();
    let power_p99 = sustained_power_percentile(&powers);
    let var = v.iter().map(|s| (s.1 - mean_p).powi(2)).sum::<f32>() / n;
    let std_p = var.sqrt();
    let capped = v.iter().filter(|s| s.2).count() as f32 / n;
    let clocks: Vec<u32> = v.iter().map(|s| s.0).collect();
    let min_clock = clocks.iter().copied().min().unwrap_or(0);
    let p5_clock = p5_clock_mhz(&clocks).unwrap_or(0);
    let p95_clock = p95_clock_mhz(&clocks).unwrap_or(0);
    let temps: Vec<f32> = v.iter().filter_map(|s| s.3).collect();
    let (start_temp_c, end_temp_c, avg_temp_c, max_temp_c) = if temps.is_empty() {
        (None, None, None, None)
    } else {
        let avg = temps.iter().sum::<f32>() / temps.len() as f32;
        let max = temps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (Some(temps[0]), Some(temps[temps.len() - 1]), Some(avg), Some(max))
    };
    let thermal_throttled = v.iter().any(|s| s.5);
    Measured {
        result: res,
        cancelled,
        clock_mhz: clock,
        power_w: mean_p,
        max_power_w: max_p,
        power_p99_w: power_p99,
        power_std_w: std_p,
        capped_frac: capped,
        volt_mv,
        sample_count: v.len() as u32,
        duration_ms,
        min_clock_mhz: min_clock,
        p5_clock_mhz: p5_clock,
        p95_clock_mhz: p95_clock,
        volt_min_mv,
        volt_avg_mv,
        volt_max_mv,
        volt_sample_count,
        start_temp_c,
        end_temp_c,
        avg_temp_c,
        max_temp_c,
        thermal_throttled,
        render_frames,
        render_fps,
        qualification_coverage,
        prehang_stall_detected,
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
#[allow(dead_code)] // V2 confidence-gate selector — retained + unit-tested for the knowledge path
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
            .min_by(|a, b| {
            a.power_capped_frac.partial_cmp(&b.power_capped_frac).unwrap_or(Ord::Equal)
        })
    } else {
        off_cap
            .iter()
            .copied()
            .max_by(|a, b| {
            a.perf_per_watt.partial_cmp(&b.perf_per_watt).unwrap_or(Ord::Equal)
        })
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
    /// Default daily-use policy: Brokkr's >= 95% clock, Deep Calm >= 90% clock, gate .85.
    /// Brokkr's floor relaxed 0.98 -> 0.95 so the knee can sit a little deeper (up to 5% clock
    /// traded for much larger efficiency gains) without colliding into Deep Calm's 90% floor.
    fn balanced() -> Self {
        Self { brokkrs_min_clock_frac: 0.95, deep_calm_min_clock_frac: 0.90, confidence_threshold: 0.85,
        }
    }
    fn conservative() -> Self {
        Self { brokkrs_min_clock_frac: 0.99, deep_calm_min_clock_frac: 0.92, confidence_threshold: 0.95,
        }
    }
    fn aggressive() -> Self {
        Self { brokkrs_min_clock_frac: 0.97, deep_calm_min_clock_frac: 0.85, confidence_threshold: 0.70,
        }
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
fn classify_regime(cap_fraction: f32, power_w: f32, power_limit_w: f32, temp_c: Option<f32>,
) -> Regime {
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

/// Read-only BRIDGE: feed an F2 LEARNED FRONTIER to the EXISTING profile classifier and return a
/// compact summary of which Godforge / Brokkr's Best / Deep Calm points it WOULD pick. Builds the
/// canonical `(PowerSweepPoint, confidence)` input via
/// [`nidavellir_core::f2_observation::frontier_to_points`] and runs the SAME [`synthesize_forge_profiles`]
/// with the balanced policy — NO new scoring. It NEVER selects, applies, persists, or promotes a
/// profile; it only previews what the learned frontier classifies to.
#[cfg(windows)]
pub(crate) fn classify_f2_frontier_summary(
    entries: &[nidavellir_core::f2_observation::F2FrontierEntry],
) -> Vec<String> {
    let points = nidavellir_core::f2_observation::frontier_to_points(entries);
    let profiles = synthesize_forge_profiles(&points, &ForgePolicy::balanced());
    let fmt = |label: &str, p: &Option<PowerSweepPoint>| -> String {
        match p {
            Some(pt) => format!(
                "  {label:<13}: {} MHz @ {} mV (sustained {:?} MHz, {:.0} W)",
                pt.clock_mhz,
                pt.vf_table_voltage_mv.unwrap_or(pt.voltage_mv),
                pt.p5_clock_mhz,
                pt.power_w
            ),
            None => format!("  {label:<13}: none (no qualifying frontier point)"),
        }
    };
    let mut out = vec![format!(
        "classifier bridge  : {} frontier point(s) -> synthesize_forge_profiles (ForgePolicy::balanced; read-only, NOT applied)",
        points.len()
    )];
    out.push(fmt("Godforge", &profiles.godforge));
    out.push(fmt("Brokkr's Best", &profiles.brokkrs));
    out.push(fmt("Deep Calm", &profiles.deep_calm));
    out.push(
        "  (classifier PREVIEW only — no profile is selected, persisted, applied, or promoted)"
            .to_string(),
    );
    out
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

/// v13.1 off-cap headroom: a published undervolt point's measured PEAK power must stay this fraction
/// below the power cap. An undervolt that reaches the cap is forced by the driver to droop voltage to
/// respect the power budget, and a droop below the point's Vmin crashes — this is what TDR'd Godforge
/// 1920@918 in-game at ~200 W while its dwell read only ~190 W. The fraction covers that measured
/// peak vs real-game gap PLUS the µs transients NVML polling cannot see.
#[cfg(windows)]
const POWER_HEADROOM_FRAC: f32 = 0.06;

/// The maximum measured PEAK power a point may draw and still count as off-cap.
#[cfg(windows)]
fn off_cap_ceiling_w(power_limit_w: f32) -> f32 {
    power_limit_w * (1.0 - POWER_HEADROOM_FRAC)
}

/// True iff a point's estimated draw keeps `POWER_HEADROOM_FRAC` below the cap. The estimate is the
/// MAX of two measurements, because neither alone is a clean upper bound on the applied point's power:
/// `max_power_w` is a true PEAK but measured at the lower BOUNDARY voltage (so it underestimates the
/// applied draw), while `power_p99_w` is only a p99 but measured at the exact APPLY voltage (the same
/// value scoring uses). Whichever is higher is the safe basis; the 6% headroom then covers the
/// p99→peak and NVML-invisible µs-transient gap. `power_limit_w <= 0` (unknown cap: bridge / legacy /
/// test) fails OPEN so the gate is a no-op. A point with NO usable power measurement cannot prove
/// headroom, so it fails CLOSED when the cap is known. Pure + testable.
#[cfg(windows)]
fn is_off_cap_safe(p: &PowerSweepPoint, power_limit_w: f32) -> bool {
    if power_limit_w <= 0.0 {
        return true;
    }
    let estimated_w = p.max_power_w.max(p.power_p99_w.unwrap_or(0.0));
    estimated_w.is_finite() && estimated_w > 0.0 && estimated_w <= off_cap_ceiling_w(power_limit_w)
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

/// Back-compat 2-arg entry (cap unknown → the off-cap gate is a no-op). The live F2 forge calls
/// [`synthesize_forge_profiles_capped`] with the measured power cap so at-cap points are excluded.
#[cfg(windows)]
#[allow(dead_code)]
fn synthesize_forge_profiles(frontier: &[(PowerSweepPoint, f64)], policy: &ForgePolicy,
) -> ForgeProfiles {
    synthesize_forge_profiles_capped(frontier, policy, 0.0)
}

/// Synthesize the three forge profiles from a (multi-clock) power frontier — each entry
/// a measured operating point plus its accumulated stability confidence (Wilson LB):
/// Every published point must ALSO be off-cap (v13.1): its measured PEAK power must keep
/// `POWER_HEADROOM_FRAC` below `power_limit_w` (the cap). An at-cap undervolt droops voltage below
/// its Vmin under the power budget and crashes. `power_limit_w <= 0` skips the gate (bridge/test).
///
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
fn synthesize_forge_profiles_capped(frontier: &[(PowerSweepPoint, f64)], policy: &ForgePolicy,
    power_limit_w: f32,
) -> ForgeProfiles {
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

    // ── Off-cap power invariant (v13.1) ──────────────────────────────────────────────────────────
    // Exclude any point whose estimated draw (peak, or apply-bin p99 — whichever is higher) reaches
    // within POWER_HEADROOM_FRAC of the cap: an undervolt held at the cap is forced to droop voltage
    // below its Vmin and crashes (Godforge
    // 1920@918 TDR'd in-game at the 200 W cap). Applies to ALL three profiles. If EVERY point is
    // at-cap the gate fails CLOSED (publishes nothing → Apply blocked) — never ships a TDR-prone
    // profile. No-op when the cap is unknown (`power_limit_w <= 0`).
    let pool: Vec<(PowerSweepPoint, f64)> = if power_limit_w > 0.0 {
        let off_cap: Vec<(PowerSweepPoint, f64)> =
            pool.iter().copied().filter(|(p, _)| is_off_cap_safe(p, power_limit_w)).collect();
        let excluded = pool.len() - off_cap.len();
        if off_cap.is_empty() {
            // Hard off-cap invariant: EVERY qualified point reaches the cap → no safe undervolt
            // exists at any qualified clock. Fail CLOSED (publish nothing → Apply stays blocked)
            // rather than ship a TDR-prone at-cap profile: an undervolt held at the cap is forced to
            // droop voltage below its Vmin and crashes (the Godforge 1920@918 failure).
            log.push(format!(
                "FORGE: off-cap gate — ALL {} qualified point(s) reach within {:.0}% of the {:.0} W \
                 cap; no off-cap profile can be published — Apply stays blocked (fail-closed)",
                pool.len(), POWER_HEADROOM_FRAC * 100.0, power_limit_w
            ));
            return ForgeProfiles {
                godforge: None, brokkrs: None, deep_calm: None,
                power_bound_excluded, power_bound_collapse, log,
            };
        }
        if excluded > 0 {
            log.push(format!(
                "FORGE: off-cap gate excluded {excluded} at-cap point(s) (peak > {:.0} W = {:.0}% \
                 of {:.0} W cap) from all profiles",
                off_cap_ceiling_w(power_limit_w), (1.0 - POWER_HEADROOM_FRAC) * 100.0, power_limit_w
            ));
        }
        off_cap
    } else {
        pool
    };

    // Sustained clock = p5 when available (dip-aware), else average (legacy fallback).
    let sustained = |p: &PowerSweepPoint| p.p5_clock_mhz.unwrap_or(p.clock_mhz);
    // F2 profiles are applied above their learned boundary. Their selection budget is therefore the
    // sustained p99 measured at the exact apply-margin bin. Legacy/F1 points retain mean-power scoring.
    let profile_power = |p: &PowerSweepPoint| {
        if p.boundary_voltage_mv.is_some() {
            p.power_p99_w.unwrap_or(0.0)
        } else {
            p.power_w
        }
    };
    let efficiency = |p: &PowerSweepPoint| {
        let power = profile_power(p);
        if power > 0.0 {
            sustained(p) as f64 / power as f64
        } else {
            0.0
        }
    };

    // Godforge = highest sustainable clock (ties → the lowest power that holds it).
    let godforge = pool
        .iter()
        .copied()
        .max_by(|a, b| {
            sustained(&a.0)
                .cmp(&sustained(&b.0))
                .then(profile_power(&b.0).partial_cmp(&profile_power(&a.0)).unwrap_or(Ord::Equal))
        })
        .unwrap();
    let gc = sustained(&godforge.0) as f64;
    let gp = profile_power(&godforge.0) as f64;

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
        .max_by(|a, b| {
            efficiency(&a.0).partial_cmp(&efficiency(&b.0)).unwrap_or(Ord::Equal)
        })
        .unwrap_or(godforge);

    // Brokkr's = best R within the Brokkr's clock floor; must be a real trade (clock
    // below Godforge AND less power). Falls back to Godforge if no such point exists.
    let br_floor = gc * policy.brokkrs_min_clock_frac;
    let r_of = |p: &PowerSweepPoint| -> f64 {
        let clk_lost = (gc - sustained(p) as f64) / gc;
        let pwr_saved = (gp - profile_power(p) as f64) / gp;
        if clk_lost > 0.0 { pwr_saved / clk_lost } else { 0.0 }
    };
    let brokkrs = pool
        .iter()
        .copied()
        .filter(|(p, _)| {
            let s = sustained(p) as f64;
            s >= br_floor && s < gc && (profile_power(p) as f64) < gp
        })
        .max_by(|a, b| r_of(&a.0).partial_cmp(&r_of(&b.0)).unwrap_or(Ord::Equal))
        .unwrap_or(godforge);

    log.push(format!(
        "FORGE: Godforge {}MHz/{:.0}W · Brokkr's {}MHz/{:.0}W (R={:.2}, floor {:.0}%) · \
         Deep Calm {}MHz/{:.0}W ({:.2} MHz/W, floor {:.0}%)",
        sustained(&godforge.0), profile_power(&godforge.0),
        sustained(&brokkrs.0), profile_power(&brokkrs.0), r_of(&brokkrs.0), policy.brokkrs_min_clock_frac * 100.0,
        sustained(&deep_calm.0), profile_power(&deep_calm.0), efficiency(&deep_calm.0), policy.deep_calm_min_clock_frac * 100.0
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
    /// Phase-B knee-seeking captured a BOUNDED below-knee tail and stopped cleanly: after the knee
    /// crossing (first `pcf < POWER_BOUND_FRAC` point) it kept descending until it had
    /// `PHASE_B_MIN_USEFUL_POINTS` useful off-cap points OR spent `PHASE_B_POST_KNEE_TAIL_BINS`
    /// post-knee bins. A CLEAN, intentional stop (no failure) — distinct from `LeftPowerRegime`
    /// (bind-seeking's first-off-cap stop), the depth `PerTargetCap`, and the `CleanFloor`. Replaces
    /// the old Phase-B first-off-cap stop, which truncated a steep knee to ONE useful point (confirmed
    /// hardware run 2026-06-16: pcf 1.0 → 0.437 in one bin → single point → still collapse).
    KneeTailComplete,
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
        Self { cap_frac: BIND_CAP_FRAC,
        }
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

    BindDecision { eligible, bound, reason, avg_clock_mhz, p5_clock_mhz, power_capped_frac,
    }
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
            "knee thresholds    : plateau = median power-bound clock; knee = pcf crosses below {:.2}, \
             then capture a bounded below-knee tail (≥ {} useful points or ≤ {} post-knee bins)",
            POWER_BOUND_FRAC, PHASE_B_MIN_USEFUL_POINTS, PHASE_B_POST_KNEE_TAIL_BINS
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
        run_target_descents(candidate_clocks, descent, carry, max_per_target, bind_seeking, &probe,
    );
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
            descend_target(target, decision.start_mv, descent, max_per_target, bind_seeking, probe,
        );
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
                descend_target(target, carry.safe_start_cap_mv, descent, max_per_target, bind_seeking, probe,
            );
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

/// Phase-B below-knee TAIL target: keep descending past the knee crossing until at least this many
/// useful off-cap points are captured. DECOUPLED from (and richer than) the synthesis collapse
/// threshold `MIN_USEFUL_FRONTIER_POINTS` (= 2): the 2-point tail validated the mechanism but on a knee
/// that hugs the power cap both points sat at ~199 W, so Godforge/Brokkr's/Deep Calm coincided
/// (confirmed-run 2026-06-20, PASS but thin). A 4-point tail reaches deeper bins where the ceiling
/// actually pulls clock/power down, giving the three profiles room to separate. Still bounded.
#[cfg(windows)]
const PHASE_B_MIN_USEFUL_POINTS: usize = 4;

/// Hard bound on the Phase-B below-knee tail: at most this many verified + dwell-stable bins are probed
/// AT or AFTER the knee crossing before the tail stops cleanly, even if the useful-point target is not
/// met (e.g. a jittery plateau that bounces back on-cap). Keeps the tail short and bounded (one bin of
/// jitter headroom over the 4-point target); the failure / instability / budget / floor stops always
/// take precedence.
#[cfg(windows)]
const PHASE_B_POST_KNEE_TAIL_BINS: u32 = 5;

/// Run ONE focused target's DEEP voltage descent, recording every verified + dwell-stable bin so the
/// knee can be detected from the pcf trajectory. Mirrors `descend_target`'s stop precedence exactly
/// (crash → abort → budget drain → verifier failure → dwell instability), and descends THROUGH the
/// knee (the first `pcf < POWER_BOUND_FRAC` point) so the below-knee efficiency tail is captured.
/// **Tail policy (steep-knee fix):** once the knee is crossed it keeps descending until it has
/// `PHASE_B_MIN_USEFUL_POINTS` useful off-cap points OR has spent `PHASE_B_POST_KNEE_TAIL_BINS`
/// post-knee bins, then stops CLEANLY as `KneeTailComplete` — instead of stopping at the FIRST off-cap
/// point (which truncated a steep knee to one useful point → still collapse). Failure / instability /
/// `budget` / floor stops are checked FIRST and always win, so the tail never descends through a
/// verifier failure, instability, drain, or below the hardware floor. The global `--max-probes`
/// (enforced by the probe closure via `budget_drained`) remains the master cap. Pure; the closure is
/// the only seam to hardware. Never writes the VF curve / runs stress itself.
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
    let mut useful_offcap = 0usize; // off-cap (pcf < POWER_BOUND_FRAC) points captured so far
    let mut knee_bin: Option<u32> = None; // bin where the knee was first crossed
    let mut tail_bins = 0u32; // verified + stable bins probed AT or AFTER the knee crossing
    for &v in descent.bins_desc.iter().filter(|&&b| b <= start_bin) {
        if probes_used >= budget {
            stop_reason = BracketStop::PerTargetCap;
            break;
        }
        let s = probe(target, v);
        probes_used += 1;
        // Drain / hard-failure / verify / instability ALWAYS win first — same precedence as
        // `descend_target`. The tail never continues through any of these.
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
                let pt = probe_to_point(target, v, &s);
                // "Useful" = off the power cap, matching the synthesis exclusion (`is_power_bound_point`,
                // keyed on POWER_BOUND_FRAC) so Phase B and synthesis agree on what counts.
                let off_cap = !is_power_bound_point(&pt);
                points.push((pt, s.confidence));
                if off_cap {
                    useful_offcap += 1;
                    if knee_bin.is_none() {
                        knee_bin = Some(v); // first off-cap point = the knee crossing
                    }
                }
                // Below-knee TAIL: once the knee is crossed keep capturing a BOUNDED tail — stop once
                // there are enough useful points to differentiate, OR a small post-knee bin window is
                // spent (a jittery plateau that never reaches the target). Reached only after the
                // failure/drain/verify/instability arms above, so safety always takes precedence.
                if knee_bin.is_some() {
                    tail_bins += 1;
                    if useful_offcap >= PHASE_B_MIN_USEFUL_POINTS
                        || tail_bins >= PHASE_B_POST_KNEE_TAIL_BINS
                    {
                        stop_reason = BracketStop::KneeTailComplete;
                        break;
                    }
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
         stable_points={} useful_offcap={useful_offcap} knee_bin={} stop={stop_reason:?}",
        points.len(),
        knee_bin.map(|b| b.to_string()).unwrap_or_else(|| "none".to_string())
    ));
    PhaseBTrajectory { points, stop_reason, probes_used, log,
    }
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
        run_target_descents(candidate_clocks, descent, carry, max_per_target, bind_seeking, &probe,
    );
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
/// Live power-sweep BUTTON defaults (no CLI args). Bounded/quick multi-clock coverage: the global
/// probe cap hard-bounds total dwell time, the per-target depth cap turns on the coverage-bounded
/// scheduler (reach several clocks before deepening one) so the three profiles can differentiate.
/// Both are hardware-relative PROBE counts (not fixed MHz); the per-probe verifier/dwell/Safe-Loop
/// guards inside the shared core remain the safety net. Conservative — knee-seeking/warm-start stay off.
#[cfg(windows)]
const BUTTON_MAX_PROBES: u32 = 24;
#[cfg(windows)]
const BUTTON_MAX_PROBES_PER_TARGET: u32 = 3;
/// FAST button mode: trimmed discovery (~half of `BUTTON_MAX_PROBES`, one bin shallower per target)
/// for a quicker supervised run. Reaches fewer clocks; each surviving pick STILL gets one full
/// fail-closed ceiling soak. Cross-run confidence is intentionally left to IDLE / later manual runs.
#[cfg(windows)]
const FAST_MAX_PROBES: u32 = 12;
#[cfg(windows)]
const FAST_MAX_PROBES_PER_TARGET: u32 = 2;
/// LONG button mode: broader (more clocks) + deeper (one extra bin/target) discovery so the three
/// profiles can differentiate, PLUS repeated per-pick ceiling soaks (`LONG_VALIDATION_PASSES`) so a
/// deep point earns its confidence in ONE session instead of over later runs. `LONG_MAX_PROBES`
/// stays a hard global cap; every probe/soak is the same fail-closed verifier/dwell/Safe-Loop motor.
#[cfg(windows)]
const LONG_MAX_PROBES: u32 = 40;
#[cfg(windows)]
const LONG_MAX_PROBES_PER_TARGET: u32 = 4;
/// LONG mode: re-soak each pick at its DISCOVERED ceiling this many times before accepting it. Any
/// non-Stable pass DROPS the pick (fail-closed) — extra passes can only reject, never widen exposure.
#[cfg(windows)]
const LONG_VALIDATION_PASSES: u32 = 3;
/// Defensive hard cap on per-pick ceiling-soak passes (mode values stay well below this).
#[cfg(windows)]
const POWER_SWEEP_MAX_VALIDATION_PASSES: u32 = 5;

/// Complete F2 frontier ends at the last real clock bin at or above 90% of the discovered Cmax so
/// Deep Calm's 90% policy floor is backed by measured data instead of a truncated 95% domain.
#[cfg(windows)]
const F2_CMAX_FLOOR_PERCENT: u64 = 90;

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

pub(crate) fn f2_stock_clock_ceiling(
    live_curve: &[(usize, u32, u32)],
) -> Result<u32, String> {
    derive_core_seed(live_curve).map(|seed| seed.stock_boost_max_mhz)
}

/// Restore the GPU to stock: zero the core offset, clear the modern VF-curve offsets, and
/// release any NVML clock cap. Idempotent; called on every exit path of the supervised run.
/// `pub(crate)` so the isolated F2 confirmed path (`gpu_undervolt`) reuses the SAME reset as
/// build-frontier (single source of truth); behavior is unchanged for F1.
#[cfg(windows)]
pub(crate) fn reset_to_stock() {
    let _ = nidavellir_gpu_nvapi::set_core_offset_mhz(0);
    let _ = nidavellir_gpu_nvapi::reset_vf_curve();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
}

/// A single load-dwell outcome, simplified for reuse by the isolated F2 confirmed path. Maps the
/// internal [`Measured`] verdict to plain booleans + the headline stats. Domain-neutral so
/// `gpu_power_sweep` stays F1-focused.
#[cfg(windows)]
pub(crate) struct SingleDwell {
    pub cancelled: bool,
    pub crashed: bool,
    pub silent_error: bool,
    pub stable: bool,
    pub avg_clock_mhz: u32,
    pub p5_clock_mhz: u32,
    pub p95_clock_mhz: u32,
    pub power_w: f32,
    pub max_power_w: f32,
    pub power_p99_w: Option<f32>,
    pub power_capped_frac: f32,
    pub max_temp_c: Option<f32>,
    pub thermal_throttled: bool,
    pub volt_min_mv: Option<u32>,
    pub volt_avg_mv: Option<u32>,
    pub volt_max_mv: Option<u32>,
    pub volt_sample_count: u32,
    pub render_frames: Option<u64>,
    pub render_fps: Option<f64>,
    pub duration_ms: u64,
    pub sample_count: u32,
    pub qualification_coverage: Option<F2QualificationCoverage>,
    pub prehang_stall_detected: bool,
}

/// Run ONE game-power load dwell via the validated [`load_and_measure`] path (fresh wgpu context,
/// FurMark-class render, steady-state measurement, TDR/device-lost detection) and return the
/// simplified outcome. Reused by the F2 confirmed single step so it shares the hardware-tested
/// measurement path rather than duplicating it. No VF write / no apply happens here.
#[cfg(windows)]
pub(crate) fn single_load_dwell_with_cancel(
    dwell_ms: u64,
    cancel: Option<&AtomicBool>,
) -> SingleDwell {
    let m = load_and_measure_for(
        dwell_ms,
        RenderStressPurpose::PowerCharacterization,
        None,
        cancel,
    );
    single_dwell_from_measured(m)
}

/// Run one transient VF-qualification dwell. It deliberately does not replace the steady power
/// render used by discovery; callers must also avoid interpreting aggregate light/medium-phase
/// telemetry as a sustained-clock boundary.
#[cfg(windows)]
pub(crate) fn single_qualifier_dwell_with_cancel(
    dwell_ms: u64,
    target_mhz: u32,
    pattern: VfQualifierPattern,
    goldens: RenderGoldens,
    cancel: Option<&AtomicBool>,
) -> SingleDwell {
    let m = load_and_measure_for(
        dwell_ms,
        RenderStressPurpose::VfQualification(pattern, goldens),
        Some(target_mhz),
        cancel,
    );
    single_dwell_from_measured(m)
}

#[cfg(windows)]
fn single_dwell_from_measured(m: Measured) -> SingleDwell {
    SingleDwell {
        cancelled: m.cancelled,
        crashed: matches!(m.result, StabilityResult::Crash),
        silent_error: matches!(m.result, StabilityResult::SilentError),
        stable: matches!(m.result, StabilityResult::Stable),
        avg_clock_mhz: m.clock_mhz,
        p5_clock_mhz: m.p5_clock_mhz,
        p95_clock_mhz: m.p95_clock_mhz,
        power_w: m.power_w,
        max_power_w: m.max_power_w,
        power_p99_w: m.power_p99_w,
        power_capped_frac: m.capped_frac,
        max_temp_c: m.max_temp_c,
        thermal_throttled: m.thermal_throttled,
        volt_min_mv: m.volt_min_mv,
        volt_avg_mv: m.volt_avg_mv,
        volt_max_mv: m.volt_max_mv,
        volt_sample_count: m.volt_sample_count,
        render_frames: m.render_frames,
        render_fps: m.render_fps,
        duration_ms: m.duration_ms,
        sample_count: m.sample_count,
        qualification_coverage: m.qualification_coverage,
        prehang_stall_detected: m.prehang_stall_detected,
    }
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

/// Confirmed multi-clock frontier measurement result. The `frontier`/`profiles` are exactly the
/// `FrontierBuildResult` the supervised run produces (so reporting/persistence is unchanged);
/// `est_wall_s` is the pre-run worst-case dwell-time estimate, `detected_clock_mhz` is the derived
/// stock reference clock (single-clock validation fallback), `aborted` mirrors the post-crash abort
/// flag, and `logs` are the ordered scheduler + synthesis decision lines.
#[cfg(windows)]
struct MultiClockForgeResult {
    frontier: Vec<PowerSweepPoint>,
    profiles: ForgeProfiles,
    est_wall_s: u64,
    detected_clock_mhz: u32,
    aborted: bool,
    logs: Vec<String>,
}

/// Run the CONFIRMED multi-clock frontier measurement and return its result, or `None` on any
/// fail-closed abort (unseeded curve, invalid limits, out-of-range target, empty descent). This is
/// the proven supervised core shared by `run_build_frontier` (console) and the live power-sweep
/// button: `derive_core_seed` → regime (clamped to PowerLimited) → hardware-derived floor →
/// `candidate_clocks` → limits → bin-based descent → `plan_frontier` → the real probe closure →
/// `build_frontier`(_two_phase) → `synthesize_forge_profiles`. EVERY fail-closed guard is preserved.
/// It ALWAYS restores stock and clears the boot flag on return (success, partial, abort). It NEVER
/// applies or persists a profile; the caller decides what to do with the returned profiles.
#[cfg(windows)]
fn measure_multiclock_forge(
    store: &SafeLoopStore,
    stop: &Arc<AtomicBool>,
    limits: &FrontierLimits,
) -> Option<MultiClockForgeResult> {
    use nidavellir_gpu_nvapi as gpu;
    if !gpu::vf_curve_supported() {
        warn!("multiclock-forge: modern VF curve API unsupported on this GPU/driver — aborting.");
        return None;
    }
    let live = gpu::read_vf_curve_modern();
    if live.is_empty() {
        warn!("multiclock-forge: VF curve readback returned no points — aborting.");
        return None;
    }
    let static_base = gpu::read_vf_base_curve_modern();
    // SANITY-DOMAIN GUARD: derive the stock reference ONLY from sane graphics-core VF points; fail
    // closed if no sane core cluster exists (same guard as the console build-frontier).
    let seed = match derive_core_seed(&live) {
        Ok(s) => s,
        Err(e) => {
            warn!("multiclock-forge: {e}");
            return None;
        }
    };

    // One NON-LOAD telemetry snapshot for regime context; CLAMP idle Unconstrained to PowerLimited
    // so a first supervised run never explores ABOVE stock.
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

    let hw_floor_mv = seed.cluster_v_min_mv;
    if let Err(e) = validate_limits(limits, hw_floor_mv) {
        warn!("multiclock-forge: invalid limits — {e}");
        return None;
    }
    let raw_targets = candidate_clocks(
        seed.stock_sustained_mhz,
        seed.stock_boost_max_mhz,
        regime,
        FRONTIER_CLOCK_STEP_MHZ,
        FRONTIER_FLOOR_FRAC,
    );
    let (targets, safe_start_mv) =
        apply_frontier_limits(raw_targets, seed.safe_start_mv, hw_floor_mv, limits);
    if targets.iter().any(|&t| t > CORE_FREQ_HARD_MAX_MHZ) {
        warn!("multiclock-forge: candidate target > {CORE_FREQ_HARD_MAX_MHZ} MHz — failing closed");
        return None;
    }
    let descent = derive_descent(&seed.cluster_bins_mv, safe_start_mv, FRONTIER_VOLT_STEP_MV);
    if descent.bins_desc.is_empty() {
        warn!("multiclock-forge: no real VF bin ≤ {safe_start_mv} mV — failing closed (no hardware floor)");
        return None;
    }
    let plan = plan_frontier(targets.clone(), &descent, DWELL_MS, limits.max_probes_per_target,
    );
    let capped_dwells = limits
        .max_probes
        .map_or(plan.est_dwell_count, |mp| plan.est_dwell_count.min(mp));
    let est_wall_s = capped_dwells as u64 * (DWELL_MS + PROBE_OVERHEAD_MS) / 1000;

    warn!("multiclock-forge: CONFIRMED — supervised hardware run begins (game-power dwells; can TDR/reboot).");
    let policy = ForgePolicy::balanced();
    let abort = AtomicBool::new(false);
    let probe_count = std::sync::atomic::AtomicU32::new(0);
    let probe = |target: u32, vbin: u32| {
        // --max-probes hard stop: short-circuit (no hardware) once the budget is spent. Also stop
        // when the caller signalled stop. Flagged `budget_drained` so the scheduler treats it as a
        // drain (never a verify failure / B2 fallback trigger).
        if stop.load(Ordering::SeqCst) {
            let mut p = unverified_probe();
            p.budget_drained = true;
            return p;
        }
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
    let carry = BracketCarryConfig::from_descent(
        &descent,
        limits.warm_start_brackets,
        FRONTIER_WARM_START_MARGIN_STEPS,
    );
    let phase_b_budget = limits
        .power_bound_knee_seeking
        .then(|| limits.phase_b_probes.unwrap_or(FRONTIER_PHASE_B_PROBES));
    let result = if let Some(budget) = phase_b_budget {
        info!(
            "multiclock-forge: power-bound knee-seeking ENABLED (opt-in) — Phase-B deep-descent budget \
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

    let aborted = abort.load(Ordering::SeqCst);
    let logs: Vec<String> = ordered_frontier_logs(&result.log, &result.profiles.log)
        .into_iter()
        .cloned()
        .collect();
    Some(MultiClockForgeResult {
        frontier: result.frontier,
        profiles: result.profiles,
        est_wall_s,
        detected_clock_mhz: seed.stock_sustained_mhz,
        aborted,
        logs,
    })
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
    let plan = plan_frontier(targets.clone(), &descent, DWELL_MS, limits.max_probes_per_target,
    );
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
        soft_max_voltage_warning(seed.safe_start_mv, descent.safe_start_mv, CORE_VF_SOFT_MAX_MV,
    )
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

    // CONFIRMED: run the shared supervised multi-clock measurement core. It re-derives the seed,
    // descent, targets, and limits (re-applying EVERY fail-closed guard the dry-run preview printed),
    // runs the real probe/build, and ALWAYS restores stock. A fail-closed abort returns `None`.
    let Some(result) = measure_multiclock_forge(store, &Arc::new(AtomicBool::new(false)), &limits)
    else {
        println!("=== build-frontier ABORTED (fail-closed) ===");
        warn!("build-frontier: supervised measurement failed closed — GPU restored to stock; nothing applied or persisted.");
        return;
    };

    if result.aborted {
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
    for l in &result.logs {
        info!("build-frontier: {l}");
    }
    info!("build-frontier: done — GPU restored to stock; no profile applied or persisted.");
}

/// Non-Windows stub — the frontier build is Windows-only (NVAPI/NVML).
#[cfg(not(windows))]
pub fn run_build_frontier(_store: &SafeLoopStore, _confirm: bool, _limits: FrontierLimits) {
    tracing::warn!("build-frontier is Windows-only");
}

/// Long-soak validation of one multi-clock pick AT ITS DISCOVERED VF-TABLE CEILING — the exact
/// operating point the Apply path will write (`apply_core` → `choose_ceiling_mv`). It mirrors the
/// real-probe ceiling write+verify (`real_probe_step`) but with the ARDUOUS soak duration so the
/// long soak validates what Apply persists, not a stock offset. FAIL-CLOSED: any non-Stable verdict
/// (unstable/silent error, device-lost/TDR crash, verify failure, snap/apply failure) DROPS the pick
/// (`None`) — no offset-style back-off. The clock is assumed already locked by the caller. EVERY
/// path resets to stock and clears the boot flag before returning.
///
/// Requires `pick.vf_table_voltage_mv = Some(vbin)`; callers with `None` keep the legacy
/// offset-based `arduous_validate` path. The bin is a discovered safe descent bin, so the crash
/// floor is already respected; `apply_vf_ceiling_monotone` itself fails closed if the static base is
/// unavailable (audit B1), and a snap below any live bin returns `None` here.
#[cfg(windows)]
fn validate_pick_at_ceiling(
    store: &SafeLoopStore,
    clk: u32,
    pick: PowerSweepPoint,
    stop: &Arc<AtomicBool>,
    label: &str,
    progress: &Arc<Mutex<PowerSweepProgress>>,
    prog: &mut PowerSweepProgress,
) -> Option<PowerSweepPoint> {
    use nidavellir_gpu_nvapi as gpu;
    let Some(vbin) = pick.vf_table_voltage_mv else {
        // Caller must not route a None-ceiling pick here.
        return Some(pick);
    };
    if stop.load(Ordering::SeqCst) {
        return Some(pick);
    }
    // Snap the discovered bin to a real VF-table bin at/above it — same as apply/verify/real-probe.
    let live = gpu::read_vf_curve_modern();
    let Some((ceiling_idx, ceiling_mv)) = gpu::nearest_vf_bin_at_or_above(&live, vbin) else {
        prog.log.push(format!(
            "✗ {label}: sem bin VF real ≥ {vbin} mV — descartado (fail-closed)."
        ));
        set(progress, prog.clone());
        return None;
    };
    let static_base = gpu::read_vf_base_curve_modern();
    prog.log.push(format!(
        "Validação árdua {label}: teto {ceiling_mv} mV @ {clk} MHz (~35s, carga de jogo)…"
    ));
    set(progress, prog.clone());

    // ARM the Safe Loop for this pick's INTENT before any VF write (a crash anytime after → the
    // ceiling is NOT reapplied on boot). Same axes shape as the real probe.
    let intent = TuningPoint::from_axes([
        ("gpu_freq_mhz", clk as i64),
        ("gpu_vf_bin_mv", ceiling_mv as i64),
    ]);
    let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_power_validate_ceiling"));

    // Apply the monotone-down, static-base-anchored VF ceiling (REUSED; never modified). Fails
    // closed if the static base is unavailable — reset + clear + drop.
    if let Err(e) = gpu::apply_vf_ceiling_monotone(ceiling_mv, clk) {
        warn!("{label}: apply_vf_ceiling_monotone({ceiling_mv} mV, {clk} MHz) failed closed: {e}");
        reset_to_stock();
        let _ = store.clear_boot_flag();
        prog.log.push(format!("✗ {label}: aplicação do teto falhou — descartado (fail-closed)."));
        set(progress, prog.clone());
        return None;
    }
    // Read-only verify the JUST-applied transient ceiling (shared path). Static base enables the
    // NoDownCapNeeded benign-zero rescue; stock_top is not available here so the narrow
    // stock-equivalent path is conservatively off (passing `None`).
    let after = gpu::read_vf_curve_modern();
    let eval = crate::gpu_verify::classify_live_ceiling(
        &after, ceiling_idx, ceiling_mv, clk, FRONTIER_VERIFY_TOL_MHZ, None, Some(&static_base),
    );
    let verified = eval.state == nidavellir_core::ipc::CurveVerification::VerifiedCurve
        || eval.stock_equivalent
        || eval.no_down_cap_rescue;
    if !verified {
        warn!("{label}: ceiling verify failed (state={:?}) — dropping pick (fail-closed).", eval.state);
        reset_to_stock();
        let _ = store.clear_boot_flag();
        prog.log.push(format!("✗ {label}: teto não verificou — descartado (fail-closed)."));
        set(progress, prog.clone());
        return None;
    }

    // Long game-power soak at the applied ceiling (the ARDUOUS duration).
    let res = load_and_measure(35_000).result;
    // ALWAYS reset to stock + clear the flag, on EVERY path, before deciding.
    reset_to_stock();
    let _ = store.clear_boot_flag();

    if matches!(res, StabilityResult::Stable) {
        prog.log.push(format!("✓ {label} validado no teto: {ceiling_mv} mV @ {clk} MHz."));
        set(progress, prog.clone());
        Some(pick)
    } else {
        // UNSTABLE / SilentError / device-lost (Crash/TDR) → DROP (conservative, no back-off).
        prog.log.push(format!(
            "✗ {label}: instável no teto {ceiling_mv} mV @ {clk} MHz — descartado (fail-closed)."
        ));
        set(progress, prog.clone());
        None
    }
}

/// Run the fail-closed ceiling soak (`validate_pick_at_ceiling`) `passes` times for ONE pick. The
/// caller has already locked the pick's clock. Each pass independently arms → applies → verifies →
/// soaks → resets at the discovered ceiling; ANY non-Stable pass DROPS the pick (`None`). Extra
/// passes (LONG mode) only let a deep point ACCUMULATE in-session confirmations — they can never
/// widen exposure or rescue a failed pass. `passes == 1` is exactly the single-soak Fast/Standard
/// behavior. A mid-run stop keeps the last good pick (no further soaks).
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn validate_pick_ceiling_passes(
    store: &SafeLoopStore,
    clk: u32,
    pick: PowerSweepPoint,
    stop: &Arc<AtomicBool>,
    label: &str,
    progress: &Arc<Mutex<PowerSweepProgress>>,
    prog: &mut PowerSweepProgress,
    passes: u32,
) -> Option<PowerSweepPoint> {
    let mut current = pick;
    for pass in 1..=passes {
        if stop.load(Ordering::SeqCst) {
            return Some(current);
        }
        if passes > 1 {
            prog.log.push(format!(
                "Validação árdua {label}: passagem {pass}/{passes} (confiança em sessão)…"
            ));
            set(progress, prog.clone());
        }
        match validate_pick_at_ceiling(store, clk, current, stop, label, progress, prog) {
            Some(p) => current = p,
            None => return None, // fail-closed: any failed pass drops the pick
        }
    }
    Some(current)
}

/// F1 MULTI-CLOCK flatten-down forge — kept INTACT but no longer routed to the live button (the button
/// now runs the F2 anchored-undervolt forge, which can differentiate power-bound cards F1 cannot).
/// Phase 3 decides this path's fate (retire vs repurpose for cards where F1 CAN differentiate). Retained
/// so its proven logic + tests stay available; `#[allow(dead_code)]` because the only caller (the button
/// spawn) now targets the F2 forge.
#[cfg(windows)]
#[allow(dead_code)]
fn run_power_sweep(
    progress: Arc<Mutex<PowerSweepProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
    mode: PowerSweepMode,
) {
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;

    info!("Power sweep starting (voltage → max-stable-clock → power) — mode {}", mode.label());
    let mut prog = idle();
    prog.running = true;
    prog.phase = "power".into();
    prog.power_limit_w = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
        .and_then(|r| r.power_limit_w)
        .unwrap_or(0.0);
    prog.log.push(format!(
        "Power sweep ({}) — cap {:.0} W. Mapeando tensão → clock estável → potência…",
        mode.label(),
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
    // Identity for forge-state persistence (single source of truth across restarts).
    let gpu_key = nidavellir_gpu_nvapi::read_curve()
        .map(|c| c.name)
        .unwrap_or_else(|_| "unknown-gpu".into());

    // BUTTON multi-clock limits: bounded supervised measurement with NO CLI args and NO fixed MHz —
    // everything stays hardware-relative (candidate clocks + real VF bins are derived from the live
    // curve inside `measure_multiclock_forge`). The per-target depth cap turns on the coverage-bounded
    // scheduler (reach several clocks before deepening one) so the three profiles can differentiate;
    // the global probe cap bounds total dwell time. The probe/depth caps come from the selected MODE
    // (Fast = trimmed, Standard = the proven default, Long = broader+deeper); `validation_passes` sets
    // how many times each pick is re-soaked at its ceiling. Knee-seeking and warm-start stay OFF
    // (default) in all modes to keep the live run conservative and predictable.
    let (max_probes, max_probes_per_target, validation_passes) = mode.tuning();
    let validation_passes = validation_passes.clamp(1, POWER_SWEEP_MAX_VALIDATION_PASSES);
    let limits = FrontierLimits {
        max_targets: None,
        max_probes: Some(max_probes),
        max_probes_per_target: Some(max_probes_per_target),
        safe_start_cap_mv: None,
        warm_start_brackets: false,
        bind_seeking: false,
        power_bound_knee_seeking: false,
        phase_b_probes: None,
    };

    // Surface the worst-case duration estimate BEFORE the long measurement (UI shows it). The estimate
    // is derived from the same hardware-relative plan the measurement runs.
    prog.note = Some("Forja multi-clock — estimando duração…".into());
    set(&progress, prog.clone());

    // Run the PROVEN multi-clock measurement core (same supervised path as build-frontier): derive
    // the seed, candidate clocks, real VF-bin descent; probe under game-power dwells with the per-probe
    // verifier + Safe Loop + reset guards; synthesize the three differentiated profiles. It ALWAYS
    // restores stock and clears the boot flag, and applies/persists NOTHING. A fail-closed abort
    // (unseeded curve, invalid limits, out-of-range target, empty descent) returns `None`.
    let Some(result) = measure_multiclock_forge(&store, &stop, &limits) else {
        // FAIL CLOSED: never persist a failed/empty run. The helper already reset to stock.
        let _ = gpu::reset_all();
        let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
        prog.running = false;
        prog.phase = "done".into();
        prog.note = Some(
            "Forja multi-clock abortada com segurança (curva/limites inválidos) — GPU restaurada ao stock, nada aplicado."
                .into(),
        );
        set(&progress, prog);
        info!("Power sweep aborted fail-closed (multi-clock measurement)");
        return;
    };

    let detected = result.detected_clock_mhz;
    prog.stock_clock_mhz = detected;
    let val_note = if validation_passes > 1 {
        format!(", validação {validation_passes}× por perfil")
    } else {
        String::new()
    };
    prog.note = Some(format!(
        "Forja multi-clock ({}) — ~{} s estimados (cap {cap:.0} W, clock base ~{detected} MHz{val_note}).",
        mode.label(),
        result.est_wall_s
    ));
    prog.log.push(format!(
        "Frontier multi-clock: {} ponto(s), ~{} s estimados, clock base ~{detected} MHz (cap {cap:.0} W).",
        result.frontier.len(), result.est_wall_s
    ));
    prog.log.extend(result.logs);

    // Map the three multi-clock profiles (godforge / Brokkr's Best / Deep Calm) and frontier points.
    // `deep_calm` is now populated by the multi-clock synthesis (additive — the existing apply handler
    // already reads it).
    prog.power_bound_collapse = result.profiles.power_bound_collapse;
    prog.godforge = result.profiles.godforge;
    prog.brokkrs = result.profiles.brokkrs;
    prog.deep_calm = result.profiles.deep_calm;
    prog.points = result.frontier;

    // APPLY-AXIS BACKFILL: multi-clock picks come from `probe_to_point`, which records the discovered
    // undervolt ceiling in `vf_table_voltage_mv` but leaves `voltage_mv = 0` (default). The Apply path
    // (`apply_core` → `choose_ceiling_mv`) keys on `voltage_mv`, and `choose_ceiling_mv(curve, 0)` snaps
    // to the LOWEST bin = deepest undervolt — the WRONG ceiling. Backfill `voltage_mv` from the
    // discovered bin so Apply writes the DISCOVERED ceiling and `prog.points`/persisted state agree.
    // `vf_table_voltage_mv` is left untouched.
    let backfill_voltage = |p: &mut PowerSweepPoint| {
        if p.voltage_mv == 0 {
            p.voltage_mv = p.vf_table_voltage_mv.unwrap_or(p.voltage_mv);
        }
    };
    if let Some(p) = prog.godforge.as_mut() {
        backfill_voltage(p);
    }
    if let Some(p) = prog.brokkrs.as_mut() {
        backfill_voltage(p);
    }
    if let Some(p) = prog.deep_calm.as_mut() {
        backfill_voltage(p);
    }
    set(&progress, prog.clone());

    // --- Arduous validation of each pick (long soak) ----------------------
    // Each pick is validated AT ITS OWN clock: lock the pick's target clock, then soak under
    // game-power for the arduous duration. A multi-clock pick carries the DISCOVERED VF-table ceiling
    // (`vf_table_voltage_mv = Some(vbin)`) — the exact operating point Apply will write — so it is
    // soaked AT THAT CEILING (`validate_pick_at_ceiling`, fail-closed: drop on any failure). A pick
    // with no ceiling (`None`, single-clock / legacy) keeps the offset-based `arduous_validate` path,
    // whose back-off candidate set is filtered to that pick's `target_clock_mhz`.
    let pts = prog.points.clone();
    prog.phase = "validate".into();
    set(&progress, prog.clone());
    let picks: [(&str, Option<PowerSweepPoint>); 3] = [
        ("Godforge", prog.godforge),
        ("Brokkr's", prog.brokkrs),
        ("Deep Calm", prog.deep_calm),
    ];
    for (label, pick) in picks {
        let Some(p) = pick else { continue };
        let clk = p.target_clock_mhz.unwrap_or(detected);
        let _ = nidavellir_core::nvml_gpu::lock_core_clock_max_mhz(clk);
        let validated = if p.vf_table_voltage_mv.is_some() {
            // Multi-clock pick: long soak AT THE DISCOVERED CEILING (mirrors the real probe write).
            // LONG mode repeats the soak (`validation_passes`) so a deep point earns in-session
            // confidence; Fast/Standard run a single pass. Any failed pass drops the pick (fail-closed).
            validate_pick_ceiling_passes(
                &store, clk, p, &stop, label, &progress, &mut prog, validation_passes,
            )
        } else {
            // Legacy / single-clock fallback: offset-based long soak + back-off within this clock.
            let same_clock: Vec<PowerSweepPoint> = match p.target_clock_mhz {
                Some(t) => pts.iter().copied().filter(|q| q.target_clock_mhz == Some(t)).collect(),
                None => pts.clone(),
            };
            arduous_validate(&mut ctx, &store, clk, p, &same_clock, &stop, label, &progress, &mut prog,
            )
        };
        match label {
            "Godforge" => prog.godforge = validated,
            "Brokkr's" => prog.brokkrs = validated,
            _ => prog.deep_calm = validated,
        }
    }
    prog.recommended = prog.brokkrs;

    // ALWAYS restore stock after validation (success, partial, or abort). No profile is applied.
    let _ = nidavellir_gpu_nvapi::unlock_core_voltage();
    let _ = gpu::reset_all();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    let _ = store.clear_boot_flag();

    if prog.godforge.is_some() {
        let fmt = |o: Option<PowerSweepPoint>| match o {
            Some(p) => format!(
                "{} MHz @ {} mV ({:.0} W)",
                p.clock_mhz,
                p.vf_table_voltage_mv.unwrap_or(p.voltage_mv),
                p.power_w
            ),
            None => "—".into(),
        };
        prog.note = Some(format!(
            "Multi-clock · cap {cap:.0} W · Godforge {} · Brokkr's {} · Deep Calm {} — confirme em jogo.",
            fmt(prog.godforge), fmt(prog.brokkrs), fmt(prog.deep_calm)
        ));
    } else {
        prog.note = Some("Nenhum perfil estável encontrado.".into());
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

/// Every distinct frequency present in the sane static VF table, highest first. These are the GPU's
/// real clock bins; no synthetic 30 MHz ladder and no mode-specific truncation.
#[cfg(windows)]
fn f2_real_clock_targets(curve: &[(usize, u32, u32)], clock_ceiling_mhz: u32) -> Vec<u32> {
    let mut clocks: Vec<u32> = curve
        .iter()
        .filter(|&&(_, mv, mhz)| {
            is_sane_core_point(mv, mhz) && mhz <= clock_ceiling_mhz
        })
        .map(|&(_, _, mhz)| mhz)
        .collect();
    clocks.sort_unstable_by(|a, b| b.cmp(a));
    clocks.dedup();
    clocks
}

#[cfg(windows)]
fn f2_clock_within_cmax_floor(target_mhz: u32, cmax_mhz: u32) -> bool {
    target_mhz as u64 * 100 >= cmax_mhz as u64 * F2_CMAX_FLOOR_PERCENT
}

#[cfg(windows)]
const F2_ESTIMATE_MAX_PROFILE_PAIRS: usize = 3;

#[cfg(windows)]
fn f2_frontier_bounds(targets: &[u32], cmax_mhz: u32) -> Option<(u32, u32)> {
    let mut floor = None;
    let mut count = 0u32;
    for &target in targets {
        if target <= cmax_mhz && f2_clock_within_cmax_floor(target, cmax_mhz) {
            floor = Some(target);
            count = count.saturating_add(1);
        }
    }
    floor.map(|floor_mhz| (floor_mhz, count))
}

#[cfg(windows)]
fn f2_target_upper_estimate_ms(
    candidate_count: usize,
    policy: F2ForgeModePolicy,
) -> u64 {
    let discovery_ms = policy.discovery_dwell_ms.saturating_add(PROBE_OVERHEAD_MS);
    let qualification_ms = u64::try_from(policy.qualification_passes)
        .unwrap_or(u64::MAX)
        .saturating_mul(
            policy
                .qualification_dwell_ms
                .saturating_add(PROBE_OVERHEAD_MS),
        );
    let final_gate_ms = u64::try_from(policy.final_gate_passes)
        .unwrap_or(u64::MAX)
        .saturating_mul(
            policy
                .final_gate_dwell_ms
                .saturating_add(PROBE_OVERHEAD_MS),
        );
    u64::try_from(candidate_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(
            discovery_ms
                .saturating_add(qualification_ms)
                .saturating_add(final_gate_ms),
        )
}

#[cfg(windows)]
fn f2_calibration_upper_estimate_ms(
    missing_count: usize,
    policy: F2ForgeModePolicy,
) -> u64 {
    u64::try_from(missing_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(
            u64::try_from(crate::gpu_undervolt::POWER_P99_MAX_ATTEMPTS)
                .unwrap_or(u64::MAX),
        )
        .saturating_mul(
            policy
                .discovery_dwell_ms
                .saturating_add(PROBE_OVERHEAD_MS),
        )
}

/// Per-pair exact-Apply dwell durations IN EXECUTION ORDER: the required 5-min patterns first,
/// then the v15 TransitionShock (~8 min), then the v14 Endurance soak (~20 min). The single source
/// for every exact-Apply ETA (upper estimate + the in-loop remaining countdown), so the displayed
/// time can never desync from what the gate actually runs.
#[cfg(windows)]
const F2_APPLY_PAIR_DWELL_LADDER_MS: [u64;
    nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS.len() + 2] = [
    F2_APPLY_QUALIFICATION_DWELL_MS,
    F2_APPLY_QUALIFICATION_DWELL_MS,
    F2_APPLY_QUALIFICATION_DWELL_MS,
    crate::gpu_undervolt::F2_TRANSITION_SHOCK_DWELL_MS,
    crate::gpu_undervolt::F2_ENDURANCE_QUALIFICATION_DWELL_MS,
];

/// Total wall-clock upper bound for ONE exact-Apply pair (every dwell + per-dwell overhead).
#[cfg(windows)]
fn f2_apply_pair_upper_ms() -> u64 {
    F2_APPLY_PAIR_DWELL_LADDER_MS
        .iter()
        .fold(0u64, |total, dwell| {
            total.saturating_add(dwell.saturating_add(PROBE_OVERHEAD_MS))
        })
}

#[cfg(windows)]
fn f2_apply_upper_estimate_ms(pair_count: usize, policy: F2ForgeModePolicy) -> u64 {
    // Fast (no qualification) skips exact-Apply entirely. Otherwise every pair runs the COMPLETE
    // gate ladder — the 3 required patterns + TransitionShock + Endurance — so the ETA reflects
    // the real deployment gate, not just the 5-min patterns.
    if f2_required_qualification_passes(policy) == 0 {
        return 0;
    }
    u64::try_from(pair_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(f2_apply_pair_upper_ms())
}

#[cfg(windows)]
fn f2_publish_upper_estimate(
    prog: &mut PowerSweepProgress,
    started: &std::time::Instant,
    remaining_upper_ms: u64,
) {
    prog.elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    prog.estimated_total_upper_ms =
        Some(prog.elapsed_ms.saturating_add(remaining_upper_ms));
}

#[cfg(windows)]
const F2_FRONTIER_PREDICTION_CONTRADICTION_MV: u32 = 25;

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct F2FrontierPrediction {
    boundary_mv: u32,
    start_mv: u32,
    used_historical_boundary: bool,
}

#[cfg(windows)]
fn f2_isotonic_trend_prediction(
    recent_boundaries: &[(u32, u32)],
    target_mhz: u32,
) -> Option<u32> {
    let recent = &recent_boundaries[recent_boundaries.len().saturating_sub(4)..];
    if recent.len() < 2 {
        return None;
    }

    // Pooled-adjacent-violators projection for the physical expectation that boundary voltage does
    // not increase as target clock decreases. The projection suggests a start; it is never evidence.
    let mut blocks: Vec<(f64, usize)> = Vec::with_capacity(recent.len());
    for &(_, voltage_mv) in recent {
        blocks.push((f64::from(voltage_mv), 1));
        while blocks.len() >= 2 {
            let right = blocks[blocks.len() - 1];
            let left = blocks[blocks.len() - 2];
            if left.0 / left.1 as f64 >= right.0 / right.1 as f64 {
                break;
            }
            blocks.pop();
            blocks.pop();
            blocks.push((left.0 + right.0, left.1 + right.1));
        }
    }
    let mut projected = Vec::with_capacity(recent.len());
    for (sum, count) in blocks {
        projected.extend(std::iter::repeat_n(sum / count as f64, count));
    }

    let mut slopes_mv_per_mhz = Vec::new();
    for index in 1..recent.len() {
        let clock_drop = recent[index - 1].0.saturating_sub(recent[index].0);
        if clock_drop == 0 {
            continue;
        }
        slopes_mv_per_mhz.push(
            (projected[index - 1] - projected[index]).max(0.0) / f64::from(clock_drop),
        );
    }
    if slopes_mv_per_mhz.is_empty() {
        return None;
    }
    slopes_mv_per_mhz.sort_by(f64::total_cmp);
    let slope = slopes_mv_per_mhz[slopes_mv_per_mhz.len() / 2];
    let last_clock = recent.last()?.0;
    let last_voltage = *projected.last()?;
    let clock_drop = last_clock.saturating_sub(target_mhz);
    Some(
        (last_voltage - slope * f64::from(clock_drop))
            .max(0.0)
            .round() as u32,
    )
}

#[cfg(windows)]
fn f2_predict_frontier_start(
    curve: &[(usize, u32, u32)],
    limits: &nidavellir_gpu_nvapi::PositiveOffsetLimits,
    target_mhz: u32,
    historical_boundary_mv: Option<u32>,
    recent_boundaries: &[(u32, u32)],
) -> Option<F2FrontierPrediction> {
    let previous_boundary_mv = recent_boundaries.last().map(|(_, mv)| *mv);
    let trend_mv = f2_isotonic_trend_prediction(recent_boundaries, target_mhz);
    let mut suggestions: Vec<u32> = [historical_boundary_mv, previous_boundary_mv, trend_mv]
        .into_iter()
        .flatten()
        .collect();
    if suggestions.is_empty() {
        return None;
    }
    let min = *suggestions.iter().min()?;
    let max = *suggestions.iter().max()?;
    if max.abs_diff(min) > F2_FRONTIER_PREDICTION_CONTRADICTION_MV {
        return None;
    }
    suggestions.sort_unstable();
    let boundary_mv = historical_boundary_mv
        .or(trend_mv)
        .unwrap_or(suggestions[suggestions.len() / 2]);
    let descent = crate::gpu_undervolt::plan_anchored_undervolt_descent(
        curve,
        target_mhz,
        None,
        limits,
        usize::MAX,
    );
    let start_mv = descent
        .candidates
        .iter()
        .map(|candidate| candidate.anchor.voltage_mv)
        .filter(|voltage_mv| *voltage_mv > boundary_mv)
        .min()?;
    Some(F2FrontierPrediction {
        boundary_mv,
        start_mv,
        used_historical_boundary: historical_boundary_mv.is_some(),
    })
}

/// LIVE F2 ANCHORED-UNDERVOLT forge — the new primary method behind the live forge button (replaces the
/// F1 flatten-down `run_power_sweep` for the button; F1 stays intact for cards it can differentiate).
///
/// F1 flatten-down cannot lower power on a power-bound card (lowering a frequency ceiling does nothing
/// once the card is at its power limit). F2 holds the clock at a LOWER VOLTAGE and drops power directly
/// (HW-proven on the RTX 3060 Ti: 1800 MHz @ 875 mV = 157 W vs the 200 W power-bound point, −43 W).
///
/// This is Phase 1: MEASURE + SYNTHESIZE + PERSIST via the proven F2 motor. APPLY stays GATED (the
/// Apply IPC writes an F1 ceiling, wrong for an F2 undervolt point — Phase 2 wires the real F2 apply).
///
/// It REUSES, never reinvents:
/// - hardware-relative candidate CLOCKS via [`derive_core_seed`] + [`candidate_clocks`] (the SAME
///   derivation the F1 forge uses — no fixed MHz);
/// - the F2 INPUT derivation + complete per-clock CONFIRMED discovery via
///   [`crate::gpu_undervolt::f2_forge_inputs`] +
///   [`crate::gpu_undervolt::run_confirmed_f2_clock_discovery`]
///   (each candidate = anchored write→verify→dwell→reset→clear, recorded as an observation, with the
///   Safe Loop arm/clear + crash-floor guards intact; Standard/Long then independently qualify the
///   discovered boundary with longer reset/reapply passes);
/// - the F2→profiles synthesis bridge: [`F2ObservationStore::learned_frontier`] →
///   [`frontier_to_points`] → [`synthesize_forge_profiles`].
///
/// Safety: STOPS the whole forge on a per-clock safety failure ([`crate::gpu_f2_sweep::ladder_should_continue`])
/// or a confirmed-gate refusal; resets to stock + clears the boot flag on EVERY exit path (success,
/// fail-closed, safety-stop); persists `forge_state.json` ONLY when a usable Godforge profile exists;
/// applies NOTHING. Every mode traverses the same clock domain; `mode` changes only evidence depth.
#[cfg(windows)]
fn measure_multiclock_undervolt_forge(
    progress: &Arc<Mutex<PowerSweepProgress>>,
    stop: &Arc<AtomicBool>,
    store: &SafeLoopStore,
    mode: PowerSweepMode,
) {
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;

    use nidavellir_core::f2_observation::{
        last_discovery_good_for_target, new_run_id, F2ObservationStore,
    };
    use nidavellir_gpu_nvapi as gpu;

    let started = Instant::now();
    info!("F2 undervolt forge starting (anchored min-stable-voltage per clock) — mode {}", mode.label());
    let previous = progress.lock().map(|g| g.clone()).unwrap_or_default();
    let mut prog = idle();
    // Keep the last completed profiles available while a new frontier is learned. A partial run may
    // replace its live/checkpoint view, but must never erase a previously usable recommendation.
    prog.points = previous.points;
    prog.recommended = previous.recommended;
    prog.godforge = previous.godforge;
    prog.brokkrs = previous.brokkrs;
    prog.deep_calm = previous.deep_calm;
    prog.power_bound_collapse = previous.power_bound_collapse;
    // A new hardware run may discover evidence that invalidates an older boundary. Keep the prior
    // recommendations visible, but fail closed until this run finishes a complete qualification.
    prog.profiles_qualified = false;
    prog.running = true;
    prog.phase = "power".into();
    prog.is_undervolt = true; // Apply IPC routes these points through the anchored F2 writer.
    prog.mode = Some(mode.id().to_string());
    prog.frontier_complete = false;
    prog.learning_saved = true;
    prog.power_limit_w = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
        .and_then(|r| r.power_limit_w)
        .unwrap_or(0.0);
    let cap = prog.power_limit_w;
    prog.log.push(format!(
        "Forja F2 (undervolt anchorado, {}) — cap {cap:.0} W. Descendo a tensão mínima estável por clock…",
        mode.label()
    ));
    set(progress, prog.clone());

    // Stable physical identity scopes observations and persisted profiles to this exact GPU.
    let gpu_key = current_gpu_key();

    // Final reset helper — runs on EVERY exit path (success / fail-closed / safety-stop). The motor
    // already resets per candidate; this is the belt-and-suspenders final restore.
    let final_reset = |store: &SafeLoopStore, clear_boot_flag: bool| -> Result<(), String> {
        let clock_error = nidavellir_core::nvml_gpu::reset_core_clock_lock().err();
        let gpu_error = gpu::reset_all().err();
        if clock_error.is_some() || gpu_error.is_some() {
            return Err(format!(
                "final reset incomplete: clock-cap={}; VF/global={}",
                clock_error.as_deref().unwrap_or("ok"),
                gpu_error.as_deref().unwrap_or("ok")
            ));
        }
        if clear_boot_flag {
            store.clear_boot_flag()
                .map_err(|e| format!("boot-flag clear failed after confirmed reset: {e}"))?;
        }
        Ok(())
    };

    let safe_record = store.load_record();
    if safe_record.safe_mode {
        prog.running = false;
        prog.phase = "incomplete".into();
        prog.note = Some(
            "Forja F2 recusada pelo Safe Loop — Safe Mode ativo; nenhum hardware foi alterado."
                .into(),
        );
        set(progress, prog);
        return;
    }
    if store.is_boot_flag_armed() {
        prog.running = false;
        prog.phase = "interrupted".into();
        prog.note = Some(
            "Forja F2 aguardando recuperação da execução interrompida; nenhum hardware foi alterado."
                .into(),
        );
        set(progress, prog);
        return;
    }
    // Discovery must observe the stock live clock domain, not a previously applied/capped profile.
    if let Err(e) = final_reset(store, true) {
        prog.running = false;
        prog.phase = "incomplete".into();
        prog.note = Some(format!(
            "Forja F2 abortada antes da descoberta — não foi possível confirmar stock: {e}"
        ));
        set(progress, prog);
        return;
    }

    // ── Derive hardware-relative candidate clocks (REUSE the F1 forge derivation; no fixed MHz) ──
    if !gpu::vf_curve_supported() {
        let _ = final_reset(store, true);
        prog.running = false;
        prog.phase = "incomplete".into();
        prog.note = Some(
            "Forja F2 abortada com segurança — API de curva V/F moderna não suportada neste GPU/driver. GPU no stock, nada aplicado."
                .into(),
        );
        set(progress, prog);
        warn!("f2-forge: modern VF curve API unsupported — fail closed");
        return;
    }
    let live = gpu::read_vf_curve_modern();
    let seed = match derive_core_seed(&live) {
        Ok(s) => s,
        Err(e) => {
            let _ = final_reset(store, true);
            prog.running = false;
            prog.phase = "incomplete".into();
            prog.note = Some(format!(
                "Forja F2 abortada com segurança (curva V/F sem cluster de núcleo sano): {e} — GPU no stock, nada aplicado."
            ));
            set(progress, prog);
            warn!("f2-forge: {e}");
            return;
        }
    };

    // F2 motor inputs (static VF base curve + hardware-derived physical offset envelope).
    // Fail-closed if no sane static base points.
    let Some(f2_inputs) =
        crate::gpu_undervolt::f2_forge_inputs(seed.stock_boost_max_mhz)
    else {
        let _ = final_reset(store, true);
        prog.running = false;
        prog.phase = "incomplete".into();
        prog.note = Some(
            "Forja F2 abortada com segurança — sem pontos de curva V/F base sãos. GPU no stock, nada aplicado."
                .into(),
        );
        set(progress, prog);
        return;
    };

    let mode_policy = mode.f2_policy();
    let qualification_needs_goldens =
        mode_policy.qualification_passes > 0 || mode_policy.final_gate_passes > 0;
    let render_goldens = if qualification_needs_goldens {
        prog.log.push("Qualificação v8: capturando golden stock para power/boost/texture-ROP/frame-cadence/geometry…".into());
        set(progress, prog.clone());
        match capture_fsgl3_render_goldens() {
            Ok(goldens) => {
                prog.log.push("Qualificação v8: golden stock capturado; os quatro padrões podem começar.".into());
                set(progress, prog.clone());
                Some(goldens)
            }
            Err(e) => {
                let _ = final_reset(store, true);
                prog.running = false;
                prog.phase = "incomplete".into();
                prog.note = Some(format!(
                    "Forja F2 abortada antes da qualificação v8 — stock não produziu golden determinístico: {e}. GPU no stock, nada aplicado."
                ));
                set(progress, prog);
                warn!("f2-forge: v8 golden capture failed: {e}");
                return;
            }
        }
    } else {
        None
    };
    let targets = f2_real_clock_targets(&live, seed.stock_boost_max_mhz);
    if targets.is_empty() {
        let _ = final_reset(store, true);
        prog.running = false;
        prog.phase = "incomplete".into();
        prog.note = Some(
            "Forja F2 abortada com segurança — nenhum clock candidato derivável. GPU no stock, nada aplicado."
                .into(),
        );
        set(progress, prog);
        warn!("f2-forge: no candidate clocks derived — fail closed");
        return;
    }
    prog.stock_clock_mhz = seed.stock_sustained_mhz;
    let estimated_targets: Vec<u32> = targets
        .iter()
        .copied()
        .take_while(|target| f2_clock_within_cmax_floor(*target, targets[0]))
        .collect();
    let target_upper_work_ms: HashMap<u32, u64> = targets
        .iter()
        .map(|&target| {
            let descent = crate::gpu_undervolt::plan_anchored_undervolt_descent(
                &f2_inputs.sane_base_curve,
                target,
                None,
                &f2_inputs.limits,
                usize::MAX,
            );
            (
                target,
                f2_target_upper_estimate_ms(descent.candidates.len(), mode_policy),
            )
        })
        .collect();
    let mut estimated_steps_by_target: HashMap<u32, usize> = estimated_targets
        .iter()
        .map(|&target| {
            let descent = crate::gpu_undervolt::plan_anchored_undervolt_descent(
                &f2_inputs.sane_base_curve,
                target,
                None,
                &f2_inputs.limits,
                usize::MAX,
            );
            let validation = mode_policy
                .qualification_passes
                .saturating_add(mode_policy.final_gate_passes);
            (target, descent.candidates.len().saturating_add(validation))
        })
        .collect();
    prog.total_steps_estimate = estimated_steps_by_target
        .values()
        .copied()
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX);
    let estimated_work_ms = estimated_steps_by_target.values().fold(0u64, |total, steps| {
        let qualification_steps = mode_policy
            .qualification_passes
            .saturating_add(mode_policy.final_gate_passes)
            .min(*steps);
        let discovery_steps = steps.saturating_sub(qualification_steps);
        total
            .saturating_add(
                u64::try_from(discovery_steps)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(
                        mode_policy
                            .discovery_dwell_ms
                            .saturating_add(PROBE_OVERHEAD_MS),
                    ),
            )
            .saturating_add(
                u64::try_from(qualification_steps)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(
                        mode_policy
                            .qualification_dwell_ms
                            .saturating_add(PROBE_OVERHEAD_MS),
                    ),
            )
    });
    let planned_average_step_ms = if prog.total_steps_estimate > 0 {
        estimated_work_ms / u64::from(prog.total_steps_estimate)
    } else {
        mode_policy
            .discovery_dwell_ms
            .saturating_add(PROBE_OVERHEAD_MS)
    };
    prog.estimated_remaining_ms = Some(estimated_work_ms);
    prog.log.push(format!(
        "Frontier F2: {} clock(s) reais disponíveis, começando em {} MHz; modo {} = descoberta {} s, qualificação v8 {}×{} s, gate final extra {}×{} s; ~{} dwells na estimativa inicial.",
        targets.len(), targets[0],
        mode.label(),
        mode_policy.discovery_dwell_ms / 1000,
        mode_policy.qualification_passes,
        mode_policy.qualification_dwell_ms / 1000,
        mode_policy.final_gate_passes,
        mode_policy.final_gate_dwell_ms / 1000,
        prog.total_steps_estimate
    ));
    set(progress, prog.clone());

    warn!("f2-forge: CONFIRMED — supervised hardware run begins (anchored undervolt per clock; can TDR/reboot).");

    // ── Complete real-clock discovery. Cmax is the first target that actually sustains under load.
    let obs_store = F2ObservationStore::system();
    let run_id = new_run_id("f2-forge");
    let power_limit = (cap > 0.0).then_some(cap);
    let mut cmax: Option<u32> = None;
    let mut forge_complete = false;
    let mut forge_aborted = false;
    let mut retain_boot_flag = false;
    let mut next_clock_start_mv: Option<u32> = None;
    let mut conservative_start_mv: Option<u32> = None;
    let mut recent_boundaries: Vec<(u32, u32)> = Vec::new();
    let mut adjusted_targets = HashSet::new();
    for (i, &target) in targets.iter().enumerate() {
        if let Some(max) = cmax {
            if !f2_clock_within_cmax_floor(target, max) {
                forge_complete = true;
                prog.log.push(format!(
                    "Fronteira completa: próximo bin real {target} MHz está abaixo de 90% do Cmax {max} MHz."
                ));
                break;
            }
        }
        if stop.load(Ordering::SeqCst) {
            prog.log.push("Parada solicitada — encerrando a forja F2.".into());
            break;
        }
        let historical_boundary_mv = last_discovery_good_for_target(
            &obs_store.query_by_target_for_gpu(target, &gpu_key),
            target,
        )
        .map(|observation| observation.anchor_mv);
        let fallback_start_mv = next_clock_start_mv;
        if let Some(prediction) = f2_predict_frontier_start(
            &f2_inputs.sane_base_curve,
            &f2_inputs.limits,
            target,
            historical_boundary_mv,
            &recent_boundaries,
        ) {
            next_clock_start_mv = Some(prediction.start_mv);
            prog.log.push(format!(
                "{target} MHz: fronteira prevista em {} mV por {}; início um bin físico acima, {} mV. Previsão orienta a busca e não conta como evidência.",
                prediction.boundary_mv,
                if prediction.used_historical_boundary {
                    "fronteira v4 anterior + tendência recente"
                } else {
                    "tendência isotônica recente"
                },
                prediction.start_mv
            ));
        } else if historical_boundary_mv.is_some() || !recent_boundaries.is_empty() {
            next_clock_start_mv = fallback_start_mv;
            prog.log.push(format!(
                "{target} MHz: previsão ausente/contraditória (> {F2_FRONTIER_PREDICTION_CONTRADICTION_MV} mV); usando início sequencial conservador {:?} mV.",
                fallback_start_mv
            ));
        }
        prog.phase = "descend".into();
        prog.current_clock_mhz = Some(target);
        prog.current_voltage_mv = next_clock_start_mv;
        prog.log.push(format!("Clock {}/{}: {target} MHz — descendo tensão (motor F2 confirmado)…", i + 1, targets.len()));
        set(progress, prog.clone());

        let mut on_progress =
            |event: crate::gpu_undervolt::F2ClockDiscoveryProgress| {
                if event.anchor_mv.is_none() && adjusted_targets.insert(event.target_mhz) {
                    let actual = event
                        .planned_steps
                        .saturating_add(mode_policy.qualification_passes)
                        .saturating_add(mode_policy.final_gate_passes);
                    let previous_estimate = estimated_steps_by_target
                        .insert(event.target_mhz, actual)
                        .unwrap_or(0);
                    let revised = usize::try_from(prog.total_steps_estimate)
                        .unwrap_or(usize::MAX)
                        .saturating_sub(previous_estimate)
                        .saturating_add(actual);
                    prog.total_steps_estimate = revised.try_into().unwrap_or(u32::MAX);
                    if event.unpruned_steps > event.planned_steps {
                        prog.log.push(format!(
                            "{} MHz: {} dwell(s) redundantes pulados pelo reaproveitamento da fronteira anterior.",
                            event.target_mhz,
                            event.unpruned_steps - event.planned_steps
                        ));
                    }
                }

                if event.outcome.is_some() {
                    prog.completed_steps = prog.completed_steps.saturating_add(1);
                    prog.learned_points = prog.learned_points.saturating_add(1);
                }
                prog.current_clock_mhz = Some(event.target_mhz);
                prog.current_voltage_mv = event.anchor_mv;
                prog.last_outcome = event.outcome.clone();
                prog.elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                let remaining = prog
                    .total_steps_estimate
                    .saturating_sub(prog.completed_steps);
                let per_step_ms = if prog.completed_steps > 0 {
                    (prog.elapsed_ms / u64::from(prog.completed_steps))
                        .max(planned_average_step_ms)
                } else {
                    planned_average_step_ms
                };
                prog.estimated_remaining_ms =
                    Some(u64::from(remaining).saturating_mul(per_step_ms));
                prog.log.push(event.line);
                set(progress, prog.clone());
                if event.outcome.is_some() {
                    save_forge_state(&gpu_key, &prog);
                }
            };
        let attempted_start_mv = next_clock_start_mv;
        let mut summary = crate::gpu_undervolt::run_confirmed_f2_clock_discovery(
            store,
            &obs_store,
            &run_id,
            &gpu_key,
            &f2_inputs.sane_base_curve,
            &f2_inputs.limits,
            target,
            next_clock_start_mv,
            power_limit,
            mode_policy.discovery_dwell_ms,
            mode_policy.qualification_dwell_ms,
            mode_policy.qualification_passes,
            mode_policy.final_gate_dwell_ms,
            mode_policy.final_gate_passes,
            render_goldens,
            stop,
            &mut on_progress,
        );
        let mut target_executed_steps = summary.executed_steps;
        // The start bin used by the most recent attempt — the base the upward recovery climbs from.
        let mut last_attempted_start_mv = attempted_start_mv;
        let mut fallback_message = None;
        if summary.warm_start_rejected {
            if let Some(warm) = attempted_start_mv {
                let fallback = conservative_start_mv;
                if fallback.is_none_or(|fallback_mv| fallback_mv > warm) {
                    fallback_message = Some(format!(
                        "{target} MHz: warm-start em {warm} mV não sustentou; fallback conservador {}.",
                        fallback
                            .map(|mv| format!("em {mv} mV"))
                            .unwrap_or_else(|| "desde o topo físico".into())
                    ));
                    let fallback_summary = crate::gpu_undervolt::run_confirmed_f2_clock_discovery(
                        store,
                        &obs_store,
                        &run_id,
                        &gpu_key,
                        &f2_inputs.sane_base_curve,
                        &f2_inputs.limits,
                        target,
                        fallback,
                        power_limit,
                        mode_policy.discovery_dwell_ms,
                        mode_policy.qualification_dwell_ms,
                        mode_policy.qualification_passes,
                        mode_policy.final_gate_dwell_ms,
                        mode_policy.final_gate_passes,
                        render_goldens,
                        stop,
                        &mut on_progress,
                    );
                    target_executed_steps = target_executed_steps
                        .saturating_add(fallback_summary.executed_steps);
                    last_attempted_start_mv = fallback;
                    summary = fallback_summary;
                }
            }
        }
        // Upward recovery (plan item 1.8): a QUALIFICATION rejection at the starting bin means
        // the predicted entry likely overshot this clock's real boundary — the existing in-clock
        // recovery cannot help because no shallower candidate exists in the plan. Re-run the
        // clock one physical bin higher (bounded) instead of discarding it. A ClockDrop-style
        // stop is NOT retried: that is the clock being unsustainable, not a prediction error.
        let mut start_recovery_climbs = 0usize;
        let mut climb_messages: Vec<String> = Vec::new();
        while start_recovery_climbs < F2_START_RECOVERY_MAX_CLIMBS
            && summary.warm_start_rejected
            && !summary.aborted
            && summary.stop_reason.starts_with("QualificationRejected")
            && !stop.load(std::sync::atomic::Ordering::SeqCst)
        {
            let Some(base_mv) = last_attempted_start_mv else { break };
            let Some(next_start) = f2_next_bin_above(&f2_inputs.sane_base_curve, base_mv) else {
                break;
            };
            start_recovery_climbs += 1;
            climb_messages.push(format!(
                "{target} MHz: rejeição de qualificação no bin inicial ({base_mv} mV); recuperação para cima — re-tentado em {next_start} mV (subida {start_recovery_climbs}/{F2_START_RECOVERY_MAX_CLIMBS})."
            ));
            let climb_summary = crate::gpu_undervolt::run_confirmed_f2_clock_discovery(
                store,
                &obs_store,
                &run_id,
                &gpu_key,
                &f2_inputs.sane_base_curve,
                &f2_inputs.limits,
                target,
                Some(next_start),
                power_limit,
                mode_policy.discovery_dwell_ms,
                mode_policy.qualification_dwell_ms,
                mode_policy.qualification_passes,
                mode_policy.final_gate_dwell_ms,
                mode_policy.final_gate_passes,
                render_goldens,
                stop,
                &mut on_progress,
            );
            target_executed_steps =
                target_executed_steps.saturating_add(climb_summary.executed_steps);
            last_attempted_start_mv = Some(next_start);
            summary = climb_summary;
        }
        if let Some(message) = fallback_message {
            prog.log.push(message);
        }
        prog.log.extend(climb_messages);
        prog.log.extend(
            summary
                .logs
                .iter()
                .filter(|line| line.contains("retomando"))
                .cloned(),
        );
        prog.log.push(format!(
            "Clock {target} MHz → sustentável {}, tensão mínima {:?} mV, primeira falha {:?} mV, motivo {}.",
            summary.sustainable, summary.last_good_mv, summary.first_bad_mv, summary.stop_reason
        ));
        let prior_clock_estimate = estimated_steps_by_target
            .insert(target, target_executed_steps)
            .unwrap_or(0);
        let revised_total = usize::try_from(prog.total_steps_estimate)
            .unwrap_or(usize::MAX)
            .saturating_sub(prior_clock_estimate)
            .saturating_add(target_executed_steps);
        prog.total_steps_estimate = revised_total
            .max(usize::try_from(prog.completed_steps).unwrap_or(usize::MAX))
            .try_into()
            .unwrap_or(u32::MAX);
        next_clock_start_mv = summary.next_clock_start_mv;
        conservative_start_mv = summary.conservative_start_mv;
        if let Some(mv) = next_clock_start_mv {
            prog.log.push(format!(
                "Próximo clock começará em {mv} mV: um bin físico acima do mínimo anterior; fallback {:?} mV.",
                conservative_start_mv
            ));
        }
        set(progress, prog.clone());
        if summary.aborted {
            forge_aborted = true;
            retain_boot_flag |= summary.retain_boot_flag;
            warn!("f2-forge: unsafe/failed end at {target} MHz — stopping forge");
            break;
        }
        if !summary.completed {
            break;
        }
        if summary.sustainable && cmax.is_none() {
            cmax = Some(target);
            prog.log.push(format!(
                "Cmax descoberto: {target} MHz sustentado. Continuando por todos os bins reais até 90%."
            ));
        }
        if let Some(max) = cmax {
            if let Some((floor_mhz, clock_count)) = f2_frontier_bounds(&targets, max) {
                prog.cmax_clock_mhz = Some(max);
                prog.frontier_floor_clock_mhz = Some(floor_mhz);
                prog.frontier_clock_count = Some(clock_count);
                let remaining_frontier_upper_ms = targets
                    .iter()
                    .skip(i + 1)
                    .take_while(|next| f2_clock_within_cmax_floor(**next, max))
                    .fold(0u64, |total, next| {
                        total.saturating_add(
                            target_upper_work_ms.get(next).copied().unwrap_or(0),
                        )
                    });
                let calibration_upper_ms = f2_calibration_upper_estimate_ms(
                    usize::try_from(clock_count).unwrap_or(usize::MAX),
                    mode_policy,
                );
                let apply_upper_ms =
                    f2_apply_upper_estimate_ms(F2_ESTIMATE_MAX_PROFILE_PAIRS, mode_policy);
                f2_publish_upper_estimate(
                    &mut prog,
                    &started,
                    remaining_frontier_upper_ms
                        .saturating_add(calibration_upper_ms)
                        .saturating_add(apply_upper_ms),
                );
                set(progress, prog.clone());
            }
        }
        if summary.sustainable {
            if let Some(boundary_mv) = summary.last_good_mv {
                recent_boundaries.push((target, boundary_mv));
            }
        }
        if i + 1 == targets.len() && cmax.is_some() {
            forge_complete = true;
        }
    }

    // Synthesize only after the complete Cmax→90% frontier exists for this exact physical GPU.
    prog.phase = "synthesize".into();
    set(progress, prog.clone());
    if let (true, Some(max)) = (forge_complete && !forge_aborted, cmax) {
        let frontier = obs_store.learned_frontier_for_gpu(&gpu_key)
            .into_iter()
            .filter(|entry| {
                entry.target_mhz <= max && f2_clock_within_cmax_floor(entry.target_mhz, max)
            })
            .collect::<Vec<_>>();
        let mut pts = nidavellir_core::f2_observation::frontier_to_points(&frontier);
        // The lift runs BEFORE the p99 backfill below, so lifted Apply pairs get their own
        // calibrated power measurement like any other pair.
        prog.log
            .extend(apply_f2_margin_policy(&mut pts, &f2_inputs.sane_base_curve));
        let initial_observations = obs_store.load_all();
        let missing_power =
            missing_f2_apply_power_backfills(&pts, &initial_observations, &gpu_key);
        let missing_power_count = missing_power.len();
        let reserved_apply_upper_ms =
            f2_apply_upper_estimate_ms(F2_ESTIMATE_MAX_PROFILE_PAIRS, mode_policy);
        let mut backfill_ok = true;
        if !missing_power.is_empty() {
            prog.phase = "calibrate".into();
            prog.total_steps_estimate = prog
                .total_steps_estimate
                .saturating_add(missing_power.len().try_into().unwrap_or(u32::MAX));
            prog.log.push(format!(
                "Calibração p99: {} bin(s) exato(s) de Apply sem medição v4; preenchendo somente essas lacunas com PowerRender.",
                missing_power.len()
            ));
            let calibration_upper_ms =
                f2_calibration_upper_estimate_ms(missing_power_count, mode_policy);
            prog.estimated_remaining_ms = Some(
                calibration_upper_ms.saturating_add(reserved_apply_upper_ms),
            );
            f2_publish_upper_estimate(
                &mut prog,
                &started,
                calibration_upper_ms.saturating_add(reserved_apply_upper_ms),
            );
            set(progress, prog.clone());
        }
        for (missing_index, missing) in missing_power.into_iter().enumerate() {
            if stop.load(Ordering::SeqCst) {
                backfill_ok = false;
                prog.log.push(
                    "Calibração p99 cancelada; nenhum perfil novo será sintetizado.".into(),
                );
                break;
            }
            let future_missing_count = missing_power_count.saturating_sub(missing_index + 1);
            let mut calibration_attempts_completed = 0usize;
            let mut on_calibration_progress =
                |event: crate::gpu_undervolt::F2ClockDiscoveryProgress| {
                    if event.outcome.is_some() {
                        calibration_attempts_completed =
                            calibration_attempts_completed.saturating_add(1);
                        prog.completed_steps = prog.completed_steps.saturating_add(1);
                        prog.learned_points = prog.learned_points.saturating_add(1);
                        prog.total_steps_estimate =
                            prog.total_steps_estimate.max(prog.completed_steps);
                    }
                    prog.current_clock_mhz = Some(event.target_mhz);
                    prog.current_voltage_mv = event.anchor_mv;
                    prog.last_outcome = event.outcome.clone();
                    let current_attempts_remaining =
                        crate::gpu_undervolt::POWER_P99_MAX_ATTEMPTS
                            .saturating_sub(calibration_attempts_completed);
                    let remaining_attempts = future_missing_count
                        .saturating_mul(crate::gpu_undervolt::POWER_P99_MAX_ATTEMPTS)
                        .saturating_add(current_attempts_remaining);
                    let calibration_upper_ms = u64::try_from(remaining_attempts)
                        .unwrap_or(u64::MAX)
                        .saturating_mul(
                            mode_policy
                                .discovery_dwell_ms
                                .saturating_add(PROBE_OVERHEAD_MS),
                        );
                    let remaining_upper_ms =
                        calibration_upper_ms.saturating_add(reserved_apply_upper_ms);
                    prog.estimated_remaining_ms = Some(remaining_upper_ms);
                    f2_publish_upper_estimate(&mut prog, &started, remaining_upper_ms);
                    prog.log.push(event.line);
                    set(progress, prog.clone());
                    if event.outcome.is_some() {
                        save_forge_state(&gpu_key, &prog);
                    }
                };
            let summary = crate::gpu_undervolt::run_confirmed_f2_power_calibration(
                store,
                &obs_store,
                &run_id,
                &gpu_key,
                &f2_inputs.sane_base_curve,
                &f2_inputs.limits,
                missing.target_mhz,
                missing.apply_mv,
                missing.reference_offset_mhz,
                power_limit,
                mode_policy.discovery_dwell_ms,
                stop,
                &mut on_calibration_progress,
            );
            prog.log.extend(summary.logs);
            prog.log.push(format!(
                "Calibração p99 {} MHz @ {} mV → {} ({} tentativa(s)).",
                missing.target_mhz,
                missing.apply_mv,
                summary.stop_reason,
                summary.executed_steps
            ));
            if !summary.confirmed {
                backfill_ok = false;
                forge_aborted |= summary.aborted;
                retain_boot_flag |= summary.retain_boot_flag;
                prog.log.push(
                    "FORGE: backfill p99 não confirmado; nenhum perfil novo será criado.".into(),
                );
                break;
            }
            let calibration_upper_ms =
                f2_calibration_upper_estimate_ms(future_missing_count, mode_policy);
            let remaining_upper_ms =
                calibration_upper_ms.saturating_add(reserved_apply_upper_ms);
            prog.estimated_remaining_ms = Some(remaining_upper_ms);
            f2_publish_upper_estimate(&mut prog, &started, remaining_upper_ms);
            set(progress, prog.clone());
        }
        prog.phase = "synthesize".into();
        set(progress, prog.clone());
        let observations = obs_store.load_all();
        let power_calibrated = if backfill_ok {
            match calibrate_f2_profile_power(&mut pts, &observations, &gpu_key) {
                Ok(()) => true,
                Err(e) => {
                    prog.godforge = None;
                    prog.brokkrs = None;
                    prog.deep_calm = None;
                    prog.recommended = None;
                    prog.profiles_qualified = false;
                    prog.log.push(format!(
                        "FORGE: calibração de potência recusada — {e}; nenhum perfil novo foi criado."
                    ));
                    false
                }
            }
        } else {
            prog.godforge = None;
            prog.brokkrs = None;
            prog.deep_calm = None;
            prog.recommended = None;
            prog.profiles_qualified = false;
            false
        };
        for (point, _) in &pts {
            if let (Some(boundary), Some(apply), Some(margin)) = (
                point.boundary_voltage_mv,
                point.vf_table_voltage_mv,
                point.apply_margin_mv,
            ) {
                prog.log.push(format!(
                    "{} MHz: fronteira {boundary} mV → Apply {apply} mV (margem física +{margin} mV).",
                    point.target_clock_mhz.unwrap_or(point.clock_mhz)
                ));
            }
        }
        let mut classified = pts;
        if power_calibrated {
            let exact_apply_required = f2_required_qualification_passes(mode_policy) > 0;
            let required_confirmations =
                f2_required_qualification_passes(mode_policy) as u32;
            let confidence_threshold = ForgePolicy::balanced().confidence_threshold;
            let mut excluded_apply_pairs = std::collections::HashSet::new();
            for (point, _) in &classified {
                if let Some(reason) = f2_regime_candidate_refusal(
                    point,
                    &classified,
                    exact_apply_required,
                    required_confirmations,
                    confidence_threshold,
                ) {
                    if let Some(key) = f2_apply_key(point) {
                        excluded_apply_pairs.insert(key);
                        prog.log.push(format!(
                            "Reconciliação de regime: {} MHz target @ {} mV VF excluído — {reason}.",
                            key.0, key.1
                        ));
                    } else {
                        prog.log.push(format!(
                            "Reconciliação de regime: candidato sem identidade Apply excluído — {reason}."
                        ));
                    }
                }
            }
            let mut final_profiles = None;
            loop {
                let eligible = classified
                    .iter()
                    .copied()
                    .filter(|(point, _)| {
                        f2_apply_key(point)
                            .is_some_and(|key| !excluded_apply_pairs.contains(&key))
                    })
                    .collect::<Vec<_>>();
                if eligible.is_empty() {
                    prog.log.push(
                        "FORGE: nenhum candidato permaneceu após a qualificação no Apply exato."
                            .into(),
                    );
                    break;
                }
                let profiles =
                    synthesize_forge_profiles_capped(&eligible, &ForgePolicy::balanced(), prog.power_limit_w);
                if !exact_apply_required {
                    final_profiles = Some(profiles);
                    break;
                }
                let selected = f2_unique_profile_points(&[
                    profiles.godforge,
                    profiles.brokkrs,
                    profiles.deep_calm,
                ]);
                if selected.len() < 3
                    && [profiles.godforge, profiles.brokkrs, profiles.deep_calm]
                        .iter()
                        .any(Option::is_none)
                {
                    prog.log.push(
                        "FORGE: síntese não produziu os três perfis; Apply permanece bloqueado."
                            .into(),
                    );
                    break;
                }
                let mut changed = false;
                let mut terminal = false;
                let mut remaining_apply_pairs = selected
                    .iter()
                    .filter(|selected_point| {
                        let Some(key) = f2_apply_key(selected_point) else {
                            return true;
                        };
                        !classified.iter().any(|(point, _)| {
                            f2_apply_key(point) == Some(key)
                                && point.apply_qualified
                                && point.apply_qualification_version
                                    == Some(
                                        nidavellir_core::f2_observation::
                                            F2_QUALIFICATION_CONTRACT_VERSION,
                                    )
                        })
                    })
                    .count();
                let apply_upper_ms =
                    f2_apply_upper_estimate_ms(remaining_apply_pairs, mode_policy);
                prog.estimated_remaining_ms = Some(apply_upper_ms);
                f2_publish_upper_estimate(&mut prog, &started, apply_upper_ms);
                set(progress, prog.clone());
                for selected_point in selected {
                    let Some(key) = f2_apply_key(&selected_point) else {
                        terminal = true;
                        prog.log.push(
                            "FORGE: perfil selecionado sem par target/Apply exato; recusado.".into(),
                        );
                        break;
                    };
                    let already_qualified = classified.iter().any(|(point, _)| {
                        f2_apply_key(point) == Some(key)
                            && point.apply_qualified
                            && point.apply_qualification_version
                                == Some(
                                    nidavellir_core::f2_observation::
                                        F2_QUALIFICATION_CONTRACT_VERSION,
                                )
                    })
                    // v14: "already qualified" must include THIS run's continuous endurance soak at
                    // the exact pair. Endurance evidence is run-scoped, so a point whose apply-
                    // qualification was restored from a prior run (same contract version) is NOT
                    // considered done — it re-runs the gate (which now includes the endurance soak)
                    // instead of publishing unproven. Fail closed.
                        && nidavellir_core::f2_observation::point_has_current_endurance_qualification(
                            &obs_store.load_all(),
                            &run_id,
                            key.0,
                            key.1,
                            &gpu_key,
                        );
                    if already_qualified {
                        continue;
                    }

                    prog.phase = "apply-qualify".into();
                    prog.total_steps_estimate = prog.total_steps_estimate.saturating_add(
                        u32::try_from(F2_APPLY_PAIR_DWELL_LADDER_MS.len()).unwrap_or(u32::MAX),
                    );
                    prog.log.push(format!(
                        "Qualificação Apply exato: {} MHz target @ {} mV VF — v8 Texture + Transitions + Memory (5 min cada) + TransitionShock ({} min) + Endurance ({} min).",
                        key.0, key.1,
                        crate::gpu_undervolt::F2_TRANSITION_SHOCK_DWELL_MS / 60_000,
                        crate::gpu_undervolt::F2_ENDURANCE_QUALIFICATION_DWELL_MS / 60_000
                    ));
                    let future_apply_pairs = remaining_apply_pairs.saturating_sub(1);
                    let mut completed_apply_dwells = 0usize;
                    set(progress, prog.clone());
                    let mut on_apply_qualification_progress =
                        |event: crate::gpu_undervolt::F2ClockDiscoveryProgress| {
                            if event.outcome.is_some() {
                                completed_apply_dwells =
                                    completed_apply_dwells.saturating_add(1);
                                prog.completed_steps = prog.completed_steps.saturating_add(1);
                                prog.learned_points = prog.learned_points.saturating_add(1);
                                prog.total_steps_estimate =
                                    prog.total_steps_estimate.max(prog.completed_steps);
                            }
                            prog.current_clock_mhz = Some(event.target_mhz);
                            prog.current_voltage_mv = event.anchor_mv;
                            prog.last_outcome = event.outcome.clone();
                            // Precise remaining time: the ladder is heterogeneous (5/5/5/8/20 min),
                            // so count the CURRENT pair's remaining dwells by position and future
                            // pairs by the full per-pair total — never a flat per-dwell average.
                            let current_pair_remaining_ms = F2_APPLY_PAIR_DWELL_LADDER_MS
                                .iter()
                                .skip(completed_apply_dwells)
                                .fold(0u64, |total, dwell| {
                                    total.saturating_add(
                                        dwell.saturating_add(PROBE_OVERHEAD_MS),
                                    )
                                });
                            let remaining_upper_ms = u64::try_from(future_apply_pairs)
                                .unwrap_or(u64::MAX)
                                .saturating_mul(f2_apply_pair_upper_ms())
                                .saturating_add(current_pair_remaining_ms);
                            prog.estimated_remaining_ms = Some(remaining_upper_ms);
                            f2_publish_upper_estimate(
                                &mut prog,
                                &started,
                                remaining_upper_ms,
                            );
                            prog.log.push(event.line);
                            set(progress, prog.clone());
                            if event.outcome.is_some() {
                                save_forge_state(&gpu_key, &prog);
                            }
                        };
                    let summary =
                        crate::gpu_undervolt::run_confirmed_f2_apply_qualification(
                            store,
                            &obs_store,
                            &run_id,
                            &gpu_key,
                            &f2_inputs.sane_base_curve,
                            &f2_inputs.limits,
                            key.0,
                            key.1,
                            selected_point.offset_mhz,
                            F2_APPLY_QUALIFICATION_DWELL_MS,
                            render_goldens,
                            stop,
                            &mut on_apply_qualification_progress,
                        );
                    prog.log.extend(summary.logs);
                    prog.log.push(format!(
                        "Qualificação Apply exato {} MHz @ {} mV → {} ({} dwell(s)).",
                        key.0, key.1, summary.stop_reason, summary.executed_steps
                    ));
                    remaining_apply_pairs = remaining_apply_pairs.saturating_sub(1);
                    let remaining_upper_ms =
                        f2_apply_upper_estimate_ms(remaining_apply_pairs, mode_policy);
                    prog.estimated_remaining_ms = Some(remaining_upper_ms);
                    f2_publish_upper_estimate(
                        &mut prog,
                        &started,
                        remaining_upper_ms,
                    );
                    set(progress, prog.clone());
                    retain_boot_flag |= summary.retain_boot_flag;
                    if summary.qualified {
                        let observations = obs_store.load_all();
                        let Some(apply_p95) =
                            nidavellir_core::f2_observation::
                                current_apply_qualification_p95_clock_at_anchor(
                                    &observations,
                                    &run_id,
                                    key.0,
                                    key.1,
                                    &gpu_key,
                                )
                        else {
                            excluded_apply_pairs.insert(key);
                            prog.log.push(format!(
                                "FORGE: candidato {} MHz target @ {} mV VF recusado — conjunto v8 completo sem p95 sustentado mensurável.",
                                key.0, key.1
                            ));
                            changed = true;
                            break;
                        };
                        for (point, _) in &mut classified {
                            if f2_apply_key(point) == Some(key) {
                                point.p95_clock_mhz =
                                    Some(point.p95_clock_mhz.unwrap_or(0).max(apply_p95));
                                point.apply_qualified = true;
                                point.apply_qualification_version = Some(
                                    nidavellir_core::f2_observation::
                                        F2_QUALIFICATION_CONTRACT_VERSION,
                                );
                            }
                        }
                        if let Some((point, _)) =
                            classified.iter().find(|(point, _)| f2_apply_key(point) == Some(key))
                        {
                            if let Some(reason) = f2_regime_candidate_refusal(
                                point,
                                &classified,
                                true,
                                required_confirmations,
                                confidence_threshold,
                            ) {
                                let dependent_keys =
                                    f2_regime_dependent_apply_keys(key, &classified);
                                for dependent in dependent_keys {
                                    excluded_apply_pairs.insert(dependent);
                                }
                                excluded_apply_pairs.insert(key);
                                prog.log.push(format!(
                                    "FORGE: p95 do conjunto v8 elevou o regime de {} MHz @ {} mV; candidato recusado — {reason}. Ressintetizando.",
                                    key.0, key.1
                                ));
                                changed = true;
                                break;
                            }
                        }
                        changed = true;
                        continue;
                    }
                    if summary.aborted || summary.cancelled {
                        forge_aborted |= summary.aborted;
                        terminal = true;
                        break;
                    }
                    let dependent_keys =
                        f2_regime_dependent_apply_keys(key, &classified);
                    let inherited_count = dependent_keys
                        .iter()
                        .filter(|dependent| excluded_apply_pairs.insert(**dependent))
                        .count();
                    excluded_apply_pairs.insert(key);
                    prog.log.push(format!(
                        "FORGE: candidato {} MHz target @ {} mV VF excluído; {} ponto(s) que herdam o mesmo regime p5 também bloqueado(s). Ressintetizando.",
                        key.0, key.1, inherited_count
                    ));
                    changed = true;
                    break;
                }
                if terminal {
                    break;
                }
                if changed {
                    continue;
                }
                final_profiles = Some(profiles);
                break;
            }
            if let Some(mut profiles) = final_profiles {
                let observations = obs_store.load_all();
                let mut selected_profiles = [
                    profiles.godforge,
                    profiles.brokkrs,
                    profiles.deep_calm,
                ];
                match publish_f2_profile_set_power_from_apply_qualification(
                    &mut selected_profiles,
                    &observations,
                    Some(&run_id),
                    &gpu_key,
                ) {
                    Ok(updated) => {
                        profiles.godforge = selected_profiles[0];
                        profiles.brokkrs = selected_profiles[1];
                        profiles.deep_calm = selected_profiles[2];
                        prog.log.extend(profiles.log.clone());
                        if let (Some(godforge), Some(brokkrs), Some(deep_calm)) = (
                            profiles.godforge,
                            profiles.brokkrs,
                            profiles.deep_calm,
                        ) {
                            prog.log.push(format!(
                                "FORGE: p99 publicado = máximo calibrado/conjunto v8 no Apply aprovado ({updated} perfil(is) elevado(s)) — Godforge {}@{} mV {:.0} W · Brokkr's {}@{} mV {:.0} W · Deep Calm {}@{} mV {:.0} W.",
                                godforge.target_clock_mhz.unwrap_or(godforge.clock_mhz),
                                godforge.vf_table_voltage_mv.unwrap_or(godforge.voltage_mv),
                                godforge.power_p99_w.unwrap_or(0.0),
                                brokkrs.target_clock_mhz.unwrap_or(brokkrs.clock_mhz),
                                brokkrs.vf_table_voltage_mv.unwrap_or(brokkrs.voltage_mv),
                                brokkrs.power_p99_w.unwrap_or(0.0),
                                deep_calm.target_clock_mhz.unwrap_or(deep_calm.clock_mhz),
                                deep_calm.vf_table_voltage_mv.unwrap_or(deep_calm.voltage_mv),
                                deep_calm.power_p99_w.unwrap_or(0.0),
                            ));
                        }
                        prog.power_bound_collapse = profiles.power_bound_collapse;
                        prog.godforge = profiles.godforge;
                        prog.brokkrs = profiles.brokkrs;
                        prog.deep_calm = profiles.deep_calm;
                        prog.recommended = prog.brokkrs;
                    }
                    Err(e) => {
                        prog.log.push(format!(
                            "FORGE: p99 final do conjunto v8 aprovado indisponível — {e}; perfis recusados."
                        ));
                    }
                }
            }
            prog.profiles_qualified = f2_profiles_meet_qualification(
                mode_policy,
                &[prog.godforge, prog.brokkrs, prog.deep_calm],
                ForgePolicy::balanced().confidence_threshold,
            );
            if !prog.profiles_qualified {
                prog.log.push(format!(
                    "FORGE: perfis provisórios — modo {} exige fronteira qualificada, confiança ≥ {:.2} e o conjunto v8 no par exato de Apply; Apply permanece bloqueado.",
                    mode.label(),
                    ForgePolicy::balanced().confidence_threshold
                ));
            }
        }
        prog.points = classified.into_iter().map(|(p, _)| p).collect();
    } else {
        prog.log.push(
            "Fronteira parcial encerrada: observações preservadas; perfis definitivos anteriores mantidos."
                .into(),
        );
    }

    // ALWAYS restore stock. Clear the boot flag only after the reset is confirmed.
    if let Err(e) = final_reset(store, !retain_boot_flag) {
        forge_complete = false;
        prog.godforge = None;
        prog.brokkrs = None;
        prog.deep_calm = None;
        prog.recommended = None;
        prog.profiles_qualified = false;
        prog.log.push(format!("Reset final NÃO confirmado: {e}"));
    } else if retain_boot_flag {
        prog.log.push(
            "Recuperação permanece armada após DeviceLost/reset não confirmado; a inicialização deverá contabilizar a execução interrompida."
                .into(),
        );
    }

    if forge_complete && prog.godforge.is_some() {
        let fmt = |o: Option<PowerSweepPoint>| match o {
            Some(p) => format!(
                "{} MHz target · p5 {} / p95 {} MHz · Apply VF {} mV · p99 {:.0} W",
                p.target_clock_mhz.unwrap_or(p.clock_mhz),
                p.p5_clock_mhz.unwrap_or(p.clock_mhz),
                p.p95_clock_mhz.unwrap_or(p.clock_mhz),
                p.vf_table_voltage_mv.unwrap_or(p.voltage_mv),
                p.power_p99_w.unwrap_or(p.max_power_w)
            ),
            None => "—".into(),
        };
        let readiness = if prog.profiles_qualified {
            "perfis qualificados, prontos para aplicação explícita"
        } else {
            "prévia descoberta; execute Standard ou Long para qualificar antes de aplicar"
        };
        prog.note = Some(format!(
            "Forja F2 · cap {cap:.0} W · Godforge {} · Brokkr's {} · Deep Calm {} — {readiness}.",
            fmt(prog.godforge), fmt(prog.brokkrs), fmt(prog.deep_calm)
        ));
    } else {
        prog.note = Some(format!(
            "Forja F2 parcial: {} novo(s) dwell(s) preservado(s) no Forge Knowledge; nenhum perfil definitivo novo foi criado.",
            prog.learned_points
        ));
    }
    prog.frontier_complete = forge_complete && prog.godforge.is_some();
    prog.elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    prog.estimated_remaining_ms = None;
    prog.estimated_total_upper_ms = Some(prog.elapsed_ms);
    prog.current_clock_mhz = None;
    prog.current_voltage_mv = None;
    prog.running = false;
    prog.phase = f2_terminal_phase(
        forge_complete,
        forge_aborted,
        retain_boot_flag,
        prog.profiles_qualified,
        prog.godforge.is_some(),
    )
    .into();
    // Every candidate is already durable in f2_observations.jsonl. Persist the UI checkpoint too so
    // a partial/failed run remains inspectable after a service restart. Previous completed profiles
    // stay attached until a newer complete frontier replaces them. NO auto-apply.
    save_forge_state(&gpu_key, &prog);
    set(progress, prog);
    info!("F2 undervolt forge finished");
}

#[cfg(windows)]
fn f2_terminal_phase(
    forge_complete: bool,
    forge_aborted: bool,
    retain_boot_flag: bool,
    profiles_qualified: bool,
    has_profile: bool,
) -> &'static str {
    if retain_boot_flag {
        "interrupted"
    } else if forge_complete && !forge_aborted && profiles_qualified && has_profile {
        "finished"
    } else if forge_complete && !forge_aborted && has_profile {
        "provisional"
    } else {
        "incomplete"
    }
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

    #[test]
    fn recover_after_reset_clears_visible_forge_state() {
        let handle = PowerSweepHandle::default();
        {
            let mut prog = handle.progress.lock().expect("progress lock");
            prog.running = true;
            prog.phase = "interrupted".into();
            prog.points = vec![PowerSweepPoint::default()];
            prog.godforge = Some(PowerSweepPoint::default());
            prog.profiles_qualified = true;
        }

        handle.recover_after_reset("reset complete");
        let restored = handle.progress();

        assert!(!restored.running);
        assert_eq!(restored.phase, "idle");
        assert!(restored.points.is_empty());
        assert!(restored.godforge.is_none());
        assert!(!restored.profiles_qualified);
        assert_eq!(restored.note.as_deref(), Some("reset complete"));
        assert_eq!(restored.log, vec!["reset complete".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn power_sweep_mode_tuning_preserves_standard_and_bounds_fast_long() {
        // Legacy F1 tuning remains covered because the old engine is intentionally retained, but the
        // live F2 route uses `f2_policy()` below and does not consume these breadth/depth budgets.
        // INVARIANT: Standard stays byte-identical to the pre-modes (proven, HW-validated) button —
        // same probe/depth caps and a single ceiling soak. The plain `StartPowerSweep` maps here.
        assert_eq!(PowerSweepMode::default(), PowerSweepMode::Standard);
        assert_eq!(
            PowerSweepMode::Standard.tuning(),
            (BUTTON_MAX_PROBES, BUTTON_MAX_PROBES_PER_TARGET, 1)
        );
        // Fast: trimmed discovery (fewer probes, no deeper than Standard), still ONE fail-closed soak.
        let (fp, fpt, fpass) = PowerSweepMode::Fast.tuning();
        assert!(fp < BUTTON_MAX_PROBES, "fast must reduce the global probe budget");
        assert!(fpt <= BUTTON_MAX_PROBES_PER_TARGET, "fast must not deepen per-target descent");
        assert_eq!(fpass, 1, "fast keeps a single ceiling soak per pick");
        // Long: broader + deeper discovery and >1 confidence passes, within the defensive hard cap.
        let (lp, lpt, lpass) = PowerSweepMode::Long.tuning();
        assert!(lp > BUTTON_MAX_PROBES, "long must widen the global probe budget");
        assert!(lpt >= BUTTON_MAX_PROBES_PER_TARGET, "long must not shrink per-target descent");
        assert!(
            (2..=POWER_SWEEP_MAX_VALIDATION_PASSES).contains(&lpass),
            "long passes must exceed 1 yet stay within the defensive cap"
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_terminal_phase_reserves_finished_for_ready_profiles() {
        assert_eq!(
            f2_terminal_phase(true, false, false, true, true),
            "finished"
        );
        assert_eq!(
            f2_terminal_phase(true, false, false, false, true),
            "provisional"
        );
        assert_eq!(
            f2_terminal_phase(false, true, true, false, false),
            "interrupted"
        );
        assert_eq!(
            f2_terminal_phase(false, true, false, false, false),
            "incomplete"
        );
        assert_eq!(PowerSweepMode::Fast.id(), "fast");
        assert_eq!(PowerSweepMode::Standard.id(), "standard");
        assert_eq!(PowerSweepMode::Long.id(), "long");
    }

    #[cfg(windows)]
    #[test]
    fn f2_apply_margin_snaps_up_to_real_bin_and_clamps_to_valid_anchor() {
        let curve = vec![
            (0, 875, 1650),
            (1, 881, 1665),
            (2, 887, 1680),
            (3, 893, 1695),
            (4, 900, 1800),
        ];
        assert_eq!(f2_apply_anchor_with_margin(&curve, 1800, 875), 887);
        assert_eq!(f2_apply_anchor_with_margin(&curve, 1800, 887), 893);

        let point = PowerSweepPoint {
            voltage_mv: 875,
            vf_table_voltage_mv: Some(875),
            boundary_voltage_mv: Some(875),
            target_clock_mhz: Some(1800),
            ..Default::default()
        };
        let mut points = vec![(point, 0.95)];
        apply_f2_margin_policy(&mut points, &curve);
        assert_eq!(points[0].0.boundary_voltage_mv, Some(875));
        assert_eq!(points[0].0.vf_table_voltage_mv, Some(887));
        assert_eq!(points[0].0.apply_margin_mv, Some(12));
        assert_eq!(points[0].0.base_apply_mv, Some(887));
    }

    #[cfg(windows)]
    #[test]
    fn f2_margin_policy_v13_never_lifts_and_reconciliation_refuses_failed_ceiling() {
        // v13: every dwell runs under an absolute NVML clock ceiling, so p95 == target by
        // construction and the regime lift no longer exists. A p95 above target can only mean the
        // ceiling silently failed — the dormant reconciliation net must then REFUSE the candidate
        // (fail closed), never lift it.
        let curve = vec![
            (0, 875, 1600),
            (1, 881, 1610),
            (2, 887, 1620),
            (3, 893, 1630),
            (4, 900, 1640),
            (5, 906, 1645),
        ];
        let mk = |target: u32, boundary: u32, p95: Option<u32>| PowerSweepPoint {
            voltage_mv: boundary,
            vf_table_voltage_mv: Some(boundary),
            boundary_voltage_mv: Some(boundary),
            target_clock_mhz: Some(target),
            p95_clock_mhz: p95,
            ..Default::default()
        };
        let mut points = vec![
            (mk(1650, 875, Some(1650)), 0.9), // ceiling held (p95 == target)
            (mk(1665, 881, Some(1680)), 0.9), // ceiling FAILED (p95 above target)
            (mk(1680, 893, Some(1680)), 0.9), // ceiling held
        ];
        let messages = apply_f2_margin_policy(&mut points, &curve);
        assert!(messages.is_empty(), "v13 removed the regime lift");
        // Applies stay at boundary + margin snapped to a real bin — no lift mutation.
        assert_eq!(points[0].0.vf_table_voltage_mv, Some(887));
        assert_eq!(points[0].0.base_apply_mv, Some(887));
        assert_eq!(points[1].0.vf_table_voltage_mv, Some(893));
        assert_eq!(points[2].0.vf_table_voltage_mv, Some(906));
        // Held-ceiling points pass the dormant net; the failed-ceiling point is refused.
        assert_eq!(f2_regime_candidate_refusal(&points[0].0, &points, false, 0, 0.0), None);
        assert_eq!(f2_regime_candidate_refusal(&points[2].0, &points, false, 0, 0.0), None);
        assert!(
            f2_regime_candidate_refusal(&points[1].0, &points, false, 0, 0.0).is_some(),
            "p95 above target (failed ceiling) must be refused, not lifted"
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_profile_power_calibration_uses_apply_bin_p99_and_preserves_avg_peak() {
        use nidavellir_core::f2_observation::{
            F2EvidenceKind, F2ObsDwell, F2ObsMode, F2ObsOutcome, F2ObsVerifier,
            F2QualificationCoverage, F2QualificationPattern, F2QualificationStrength,
            F2QualificationVerdict, F2_DISCOVERY_CONTRACT_VERSION,
            F2_QUALIFICATION_CONTRACT_VERSION,
        };

        let measured = F2Observation {
            run_id: "power-v4".into(),
            timestamp: "2026-07-01T00:00:00Z".into(),
            gpu_key: Some("GPU-1".into()),
            evidence_kind: F2EvidenceKind::Discovery,
            discovery_contract_version: Some(F2_DISCOVERY_CONTRACT_VERSION),
            qualification_contract_version: None,
            qualification_coverage: None,
            mode: F2ObsMode::LadderSweep,
            target_mhz: 1920,
            requested_start_mv: None,
            anchor_mv: 943,
            base_mhz: 1905,
            offset_mhz: 15,
            positive_offset_cap_mhz: 90,
            higher_bins_capped: 10,
            max_flatten_mhz: 120,
            lower_bins_elastic: 40,
            verifier_result: F2ObsVerifier::RaiseVerified,
            dwell_result: F2ObsDwell::ClockDrop,
            avg_clock_mhz: Some(1882),
            sustained_clock_mhz: Some(1875),
            sustained_upper_clock_mhz: Some(1890),
            watts: Some(188),
            max_watts: Some(200),
            power_p99_w: Some(198.0),
            power_p99_confirmed: true,
            power_p99_attempts: 1,
            measured_voltage_min_mv: Some(942),
            measured_voltage_avg_mv: Some(943),
            measured_voltage_max_mv: Some(944),
            measured_voltage_sample_count: 16,
            render_frames: Some(600),
            render_fps: Some(60.0),
            power_capped_frac: Some(1.0),
            max_temp_c: Some(72.0),
            thermal_throttled: false,
            dwell_duration_ms: Some(10_000),
            sample_count: Some(130),
            silent_error: false,
            device_lost: false,
            unstable: false,
            clock_drop: true,
            tdr_or_crash: false,
            reset_to_stock_attempted: true,
            reset_to_stock_ok: true,
            boot_flag_cleared: true,
            blacklisted: false,
            outcome: F2ObsOutcome::PowerBoundClockDrop,
            confidence: None,
            notes: None,
        };
        let point = PowerSweepPoint {
            clock_mhz: 1920,
            p5_clock_mhz: Some(1920),
            power_w: 170.0,
            max_power_w: 180.0,
            target_clock_mhz: Some(1920),
            offset_mhz: 15,
            boundary_voltage_mv: Some(931),
            vf_table_voltage_mv: Some(943),
            apply_margin_mv: Some(12),
            ..Default::default()
        };
        let mut points = vec![(point, 0.95)];
        assert!(missing_f2_apply_power_backfills(
            &points,
            std::slice::from_ref(&measured),
            "GPU-1"
        )
        .is_empty());
        let mut missing_point = point;
        missing_point.vf_table_voltage_mv = Some(950);
        assert_eq!(
            missing_f2_apply_power_backfills(
                &[(missing_point, 0.95)],
                std::slice::from_ref(&measured),
                "GPU-1"
            ),
            vec![F2ApplyPowerBackfill {
                target_mhz: 1920,
                apply_mv: 950,
                reference_offset_mhz: 15,
            }]
        );
        let mut thermally_invalid = measured.clone();
        thermally_invalid.thermal_throttled = true;
        assert_eq!(
            missing_f2_apply_power_backfills(
                &points,
                std::slice::from_ref(&thermally_invalid),
                "GPU-1"
            )
            .len(),
            1
        );

        calibrate_f2_profile_power(&mut points, std::slice::from_ref(&measured), "GPU-1").unwrap();
        let calibrated = points[0].0;
        assert_eq!(calibrated.clock_mhz, 1882);
        assert_eq!(calibrated.p5_clock_mhz, Some(1875));
        assert_eq!(calibrated.p95_clock_mhz, Some(1890));
        assert_eq!(calibrated.power_w, 188.0);
        assert_eq!(calibrated.max_power_w, 200.0);
        assert_eq!(calibrated.power_p99_w, Some(198.0));
        assert_eq!(calibrated.max_temp_c, Some(72.0));
        assert!((calibrated.perf_per_watt - 1875.0 / 198.0).abs() < f64::EPSILON);

        let apply_pass = |pattern, power_p99_w| {
            let mut observation = measured.clone();
            observation.run_id = "forge-v7".into();
            observation.evidence_kind = F2EvidenceKind::ApplyQualification;
            observation.discovery_contract_version = None;
            observation.qualification_contract_version =
                Some(F2_QUALIFICATION_CONTRACT_VERSION);
            observation.mode = F2ObsMode::ApplyQualification;
            observation.dwell_result = F2ObsDwell::Stable;
            observation.outcome = F2ObsOutcome::Validated;
            observation.power_p99_w = Some(power_p99_w);
            observation.qualification_coverage = Some(F2QualificationCoverage {
                strength: F2QualificationStrength::Fsgl4,
                pattern: Some(pattern),
                pass_index: match pattern {
                    F2QualificationPattern::A => 1,
                    F2QualificationPattern::B => 2,
                    F2QualificationPattern::HighFps => 1,
                    F2QualificationPattern::Texture => 2,
                    F2QualificationPattern::Transitions => 3,
                    F2QualificationPattern::Memory => 4,
                    F2QualificationPattern::Endurance => 5,
                    F2QualificationPattern::TransitionShock => 6,
                },
                verdict: F2QualificationVerdict::Pass,
                phases_completed: 8,
                phases_expected: 8,
                checksum_count: 8,
                sample_count: 100,
                compute_check_count: 1,
                target_residency_frac: Some(1.0),
                heavy_light_power_delta_w: Some(20.0),
                failure_phase: None,
                retry_count: 0,
                reason: None,
                phase_metrics: Vec::new(),
            });
            observation
        };
        let apply_observations = [
            apply_pass(F2QualificationPattern::HighFps, 201.25),
            apply_pass(F2QualificationPattern::Texture, 203.5),
            apply_pass(F2QualificationPattern::Transitions, 204.0),
            apply_pass(F2QualificationPattern::Memory, 202.75),
        ];
        let mut published = calibrated;
        assert!(publish_f2_profile_power_from_apply_qualification(
            &mut published,
            &apply_observations,
            Some("forge-v7"),
            "GPU-1"
        )
        .unwrap());
        assert_eq!(published.power_p99_w, Some(204.0));
        assert!((published.perf_per_watt - 1875.0 / 204.0).abs() < f64::EPSILON);

        let mut missing_p99 = measured;
        missing_p99.power_p99_w = None;
        let err = calibrate_f2_profile_power(&mut points, &[missing_p99], "GPU-1").unwrap_err();
        assert!(err.contains("discovery-v4 confirmed sustained-p99"));
    }

    #[cfg(windows)]
    #[test]
    fn next_bin_above_returns_smallest_higher_voltage_or_none() {
        let sane = vec![(0, 843, 1815), (1, 850, 1830), (2, 856, 1845), (3, 862, 1860)];
        assert_eq!(f2_next_bin_above(&sane, 843), Some(850));
        assert_eq!(f2_next_bin_above(&sane, 851), Some(856));
        assert_eq!(f2_next_bin_above(&sane, 862), None);
    }

    #[cfg(windows)]
    #[test]
    fn f2_modes_share_discovery_and_scale_qualification() {
        let fast = PowerSweepMode::Fast.f2_policy();
        let standard = PowerSweepMode::Standard.f2_policy();
        let long = PowerSweepMode::Long.f2_policy();
        assert_eq!(
            (
                fast.discovery_dwell_ms,
                standard.discovery_dwell_ms,
                long.discovery_dwell_ms
            ),
            (10_000, 10_000, 10_000)
        );
        assert_eq!(
            (
                fast.qualification_dwell_ms,
                fast.qualification_passes,
                fast.final_gate_dwell_ms,
                fast.final_gate_passes
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(
            (
                standard.qualification_dwell_ms,
                standard.qualification_passes,
                standard.final_gate_dwell_ms,
                standard.final_gate_passes
            ),
            (60_000, F2_DESCENT_DETECTOR_PASSES, 0, 0)
        );
        assert_eq!(
            (
                long.qualification_dwell_ms,
                long.qualification_passes,
                long.final_gate_dwell_ms,
                long.final_gate_passes
            ),
            (60_000, F2_DESCENT_DETECTOR_PASSES, 0, 0)
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_apply_qualification_requires_mode_evidence_on_all_profiles() {
        let qualified = PowerSweepPoint {
            confidence: Some(0.99),
            validation_count: Some(4),
            apply_qualified: true,
            apply_qualification_version: Some(
                nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION,
            ),
            ..Default::default()
        };
        let mut profiles = [Some(qualified); 3];
        assert!(!f2_profiles_meet_qualification(
            PowerSweepMode::Fast.f2_policy(),
            &profiles,
            0.85
        ));
        assert!(f2_profiles_meet_qualification(
            PowerSweepMode::Standard.f2_policy(),
            &profiles,
            0.85
        ));
        profiles[0] = Some(PowerSweepPoint {
            apply_qualification_version: Some(
                nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION - 1,
            ),
            ..qualified
        });
        assert!(!f2_profiles_meet_qualification(
            PowerSweepMode::Standard.f2_policy(),
            &profiles,
            0.85
        ));
        profiles[0] = Some(qualified);
        // v13 single-detector: the boundary confirmation count is 1 (Texture), so validation_count 0
        // (no boundary qualification at all) still fails; 1 now suffices for that gate.
        profiles[0] = Some(PowerSweepPoint {
            confidence: Some(0.99),
            validation_count: Some(0),
            apply_qualified: true,
            apply_qualification_version: Some(
                nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION,
            ),
            ..Default::default()
        });
        assert!(!f2_profiles_meet_qualification(
            PowerSweepMode::Standard.f2_policy(),
            &profiles,
            0.85
        ));
        profiles[0] = Some(qualified);
        profiles[2] = Some(PowerSweepPoint {
            confidence: Some(0.84),
            validation_count: Some(3),
            apply_qualified: true,
            apply_qualification_version: Some(
                nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION,
            ),
            ..Default::default()
        });
        assert!(!f2_profiles_meet_qualification(
            PowerSweepMode::Standard.f2_policy(),
            &profiles,
            0.85
        ));
    }

    #[cfg(windows)]
    #[test]
    fn exact_apply_gate_deduplicates_shared_profile_points_by_target_and_bin() {
        let shared = PowerSweepPoint {
            target_clock_mhz: Some(1860),
            vf_table_voltage_mv: Some(893),
            ..Default::default()
        };
        let distinct = PowerSweepPoint {
            target_clock_mhz: Some(1890),
            vf_table_voltage_mv: Some(912),
            ..Default::default()
        };
        assert_eq!(
            f2_unique_profile_points(&[Some(shared), Some(shared), Some(distinct)]).len(),
            2
        );
    }

    #[cfg(windows)]
    fn regime_point(
        target_mhz: u32,
        apply_mv: u32,
        p5_clock_mhz: u32,
        validation_count: u32,
    ) -> (PowerSweepPoint, f64) {
        (
            PowerSweepPoint {
                clock_mhz: p5_clock_mhz,
                target_clock_mhz: Some(target_mhz),
                vf_table_voltage_mv: Some(apply_mv),
                p5_clock_mhz: Some(p5_clock_mhz),
                p95_clock_mhz: Some(p5_clock_mhz),
                power_w: 180.0,
                max_power_w: 185.0,
                power_p99_w: Some(182.0),
                stable: true,
                perf_per_watt: p5_clock_mhz as f64 / 182.0,
                confidence: Some(0.99),
                validation_count: Some(validation_count),
                ..Default::default()
            },
            0.99,
        )
    }

    #[cfg(windows)]
    #[test]
    fn profile_regime_rejects_1860_at_893_when_p95_sustains_1890() {
        let frontier = vec![
            regime_point(1860, 893, 1890, 3),
            regime_point(1875, 900, 1875, 3),
            regime_point(1890, 918, 1890, 3),
        ];
        let refusal =
            f2_regime_candidate_refusal(&frontier[0].0, &frontier, true, 3, 0.85)
                .expect("lower target must inherit the 1890 MHz regime");
        assert!(refusal.contains("p95 1890 MHz"));
        assert!(refusal.contains("below the 918 mV"));
        assert!(
            f2_regime_candidate_refusal(&frontier[2].0, &frontier, true, 3, 0.85)
                .is_none(),
            "the canonical 1890 MHz / 918 mV point remains eligible"
        );
    }

    #[cfg(windows)]
    #[test]
    fn profile_regime_rejects_one_bin_alias_unless_voltage_covers_the_regime() {
        let one_bin = regime_point(1860, 893, 1875, 3);
        let one_bin_support = regime_point(1875, 900, 1875, 3);
        let raised = regime_point(1860, 925, 1890, 3);
        let support = regime_point(1890, 918, 1890, 3);
        let one_bin_frontier = vec![one_bin, one_bin_support];
        assert!(
            f2_regime_candidate_refusal(
                &one_bin_frontier[0].0,
                &one_bin_frontier,
                true,
                3,
                0.85
            )
            .is_some(),
            "v8 has zero sustained-regime bin tolerance"
        );
        let raised_frontier = vec![raised, support];
        assert!(
            f2_regime_candidate_refusal(
                &raised_frontier[0].0,
                &raised_frontier,
                true,
                3,
                0.85
            )
            .is_none()
        );
    }

    #[cfg(windows)]
    #[test]
    fn profile_regime_inherits_inconclusive_support_and_exact_apply_failure() {
        let lower = regime_point(1860, 893, 1890, 3);
        let inconclusive_support = regime_point(1890, 918, 1890, 2);
        let frontier = vec![lower, inconclusive_support];
        let refusal =
            f2_regime_candidate_refusal(&frontier[0].0, &frontier, true, 3, 0.85)
                .expect("inconclusive 1890 support must block the lower alias");
        assert!(refusal.contains("failed or inconclusive"));

        let dependent = f2_regime_dependent_apply_keys((1890, 918), &frontier);
        assert!(dependent.contains(&(1860, 893)));
        assert!(!dependent.contains(&(1890, 918)));
    }

    #[cfg(windows)]
    #[test]
    fn profile_synthesis_uses_canonical_regime_after_alias_is_removed() {
        let mut efficient = regime_point(1845, 887, 1845, 3);
        efficient.0.power_p99_w = Some(168.0);
        efficient.0.perf_per_watt = 1845.0 / 168.0;
        let alias = regime_point(1860, 893, 1890, 3);
        let canonical = regime_point(1890, 918, 1890, 3);
        let frontier = vec![efficient, alias, canonical];
        let eligible = frontier
            .iter()
            .copied()
            .filter(|(point, _)| {
                f2_regime_candidate_refusal(point, &frontier, true, 3, 0.85)
                    .is_none()
            })
            .collect::<Vec<_>>();
        assert!(!eligible
            .iter()
            .any(|(point, _)| point.target_clock_mhz == Some(1860)));
        let profiles = synthesize_forge_profiles(&eligible, &ForgePolicy::balanced());
        assert_eq!(
            profiles.godforge.unwrap().target_clock_mhz,
            Some(1890),
            "performance must resolve to the canonical 1890/918 support"
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_qualification_coverage_classifies_pass_fail_and_inconclusive() {
        let phases = [
            VfQualifierPhase::PowerOpening,
            VfQualifierPhase::BoostEdge,
            VfQualifierPhase::HeavySpike,
            VfQualifierPhase::TextureRop,
            VfQualifierPhase::ComputeBurst,
            VfQualifierPhase::IdlePulse,
            VfQualifierPhase::MixedGame,
            VfQualifierPhase::PowerClosing,
            VfQualifierPhase::FrameCadence,
            VfQualifierPhase::VramPressure,
            VfQualifierPhase::GeometryDepth,
            VfQualifierPhase::TextureStream,
        ];
        let reports: Vec<_> = phases
            .iter()
            .map(|&phase| nidavellir_gpu_stress::VfPhaseReport {
                phase,
                result: StabilityResult::Stable,
                frames: 10,
                checksum_count: 1,
                elapsed_ms: 1_000,
            })
            .collect();
        let samples: Vec<_> = phases
            .iter()
            .flat_map(|&phase| {
                let power = match phase {
                    VfQualifierPhase::BoostEdge => 100.0,
                    VfQualifierPhase::PowerOpening
                    | VfQualifierPhase::HeavySpike
                    | VfQualifierPhase::PowerClosing => 180.0,
                    _ => 145.0,
                };
                (0..4).map(move |_| (1800, power, false, None, phase.code(), false))
            })
            .collect();

        let pass = qualification_coverage_from_run(
            StabilityResult::Stable,
            &reports,
            &samples,
            Some(1800),
            VfQualifierPattern::Fsgl3A,
        );
        assert_eq!(pass.verdict, F2QualificationVerdict::Pass);
        assert_eq!(pass.strength, F2QualificationStrength::Fsgl3);
        assert_eq!(pass.pattern, Some(F2QualificationPattern::A));
        assert_eq!(pass.phases_completed, 12);
        assert_eq!(pass.phases_expected, 8);
        assert_eq!(pass.compute_check_count, 1);
        assert_eq!(pass.phase_metrics.len(), 12);
        assert_eq!(pass.phase_metrics[0].phase_pattern, "fsgl3-a");

        let texture_pass = qualification_coverage_from_run(
            StabilityResult::Stable,
            &reports,
            &samples,
            Some(1800),
            VfQualifierPattern::V8Texture,
        );
        assert_eq!(texture_pass.verdict, F2QualificationVerdict::Pass);
        assert_eq!(texture_pass.strength, F2QualificationStrength::Fsgl4);
        assert_eq!(
            texture_pass.pattern,
            Some(F2QualificationPattern::Texture)
        );
        assert_eq!(texture_pass.phases_completed, 12);
        assert_eq!(texture_pass.phases_expected, 11);

        // A v8 run that never completed FrameCadence is Inconclusive, not Pass.
        let missing_cadence = qualification_coverage_from_run(
            StabilityResult::Stable,
            &reports[..8],
            &samples,
            Some(1800),
            VfQualifierPattern::V8Texture,
        );
        assert_eq!(missing_cadence.verdict, F2QualificationVerdict::Inconclusive);

        let failed = qualification_coverage_from_run(
            StabilityResult::SilentError,
            &reports,
            &samples,
            Some(1800),
            VfQualifierPattern::Fsgl3B,
        );
        assert_eq!(failed.verdict, F2QualificationVerdict::Fail);
        assert_eq!(failed.strength, F2QualificationStrength::Fsgl3);

        let missing_phase = qualification_coverage_from_run(
            StabilityResult::Stable,
            &reports[..7],
            &samples,
            Some(1800),
            VfQualifierPattern::Fsgl1,
        );
        assert_eq!(missing_phase.verdict, F2QualificationVerdict::Inconclusive);
        assert_eq!(missing_phase.strength, F2QualificationStrength::Fsgl1);
    }

    #[cfg(windows)]
    #[test]
    fn f2_uses_every_real_clock_bin_and_stops_below_90_percent_of_cmax() {
        let curve = vec![
            (0, 900, 1905),
            (1, 925, 1950),
            (2, 950, 1935),
            (3, 975, 1905),
            (4, 1000, 1890),
            (5, 1025, 1845),
        ];
        assert_eq!(
            f2_real_clock_targets(&curve, 1950),
            vec![1950, 1935, 1905, 1890, 1845]
        );
        let stock_curve: Vec<(usize, u32, u32)> = (0..8)
            .map(|i| (i, 900 + i as u32 * 6, 1845 + i as u32 * 15))
            .collect();
        assert_eq!(f2_stock_clock_ceiling(&stock_curve).unwrap(), 1950);
        assert!(f2_clock_within_cmax_floor(1755, 1950));
        assert!(!f2_clock_within_cmax_floor(1740, 1950));
    }

    #[cfg(windows)]
    #[test]
    fn f2_time_ceiling_uses_real_frontier_and_phase_costs() {
        let targets = vec![
            1950, 1935, 1920, 1905, 1890, 1875, 1860, 1845, 1830, 1815, 1800,
            1785, 1770, 1755, 1740,
        ];
        assert_eq!(f2_frontier_bounds(&targets, 1935), Some((1755, 13)));

        let standard = PowerSweepMode::Standard.f2_policy();
        // v13 single-detector: descent = 15s discovery + 1×65s (Texture only). Exact-Apply runs
        // the full gate ladder per pair: 3×305s patterns + 485s TransitionShock + 1205s Endurance.
        assert_eq!(f2_target_upper_estimate_ms(1, standard), 80_000);
        assert_eq!(f2_calibration_upper_estimate_ms(1, standard), 45_000);
        assert_eq!(f2_apply_upper_estimate_ms(1, standard), 2_605_000);
        assert_eq!(
            f2_apply_upper_estimate_ms(F2_ESTIMATE_MAX_PROFILE_PAIRS, standard),
            7_815_000
        );

        let fast = PowerSweepMode::Fast.f2_policy();
        assert_eq!(f2_target_upper_estimate_ms(1, fast), 15_000);
        assert_eq!(f2_apply_upper_estimate_ms(3, fast), 0);

        // v13 decoupling regression: the descent runs a SINGLE detector, but exact-Apply must run
        // the FULL required set. The apply ETA must key off the complete gate ladder (which embeds
        // REQUIRED_QUALIFICATION_PATTERNS.len()), NOT the (smaller) per-mode qualification_passes —
        // the desync that once published 0 profiles.
        assert_ne!(
            standard.qualification_passes,
            nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS.len()
        );
        // v15: the ladder = 3 required patterns + TransitionShock + Endurance, each + overhead —
        // the ETA and the gate can never desync because both read the SAME ladder const.
        assert_eq!(
            F2_APPLY_PAIR_DWELL_LADDER_MS.len(),
            nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS.len() + 2
        );
        assert_eq!(
            f2_apply_upper_estimate_ms(1, standard),
            nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS.len() as u64
                * (F2_APPLY_QUALIFICATION_DWELL_MS + PROBE_OVERHEAD_MS)
                + (crate::gpu_undervolt::F2_TRANSITION_SHOCK_DWELL_MS + PROBE_OVERHEAD_MS)
                + (crate::gpu_undervolt::F2_ENDURANCE_QUALIFICATION_DWELL_MS + PROBE_OVERHEAD_MS)
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_frontier_prediction_prefers_compatible_v4_history_and_starts_one_bin_above() {
        let curve: Vec<(usize, u32, u32)> =
            [850, 856, 862, 868, 875, 881, 887, 893, 900, 906]
                .into_iter()
                .enumerate()
                .map(|(index, voltage_mv)| (index, voltage_mv, 1800 + index as u32 * 5))
                .collect();
        let limits =
            nidavellir_gpu_nvapi::PositiveOffsetLimits::hardware_frontier(850, 1935, 1800);
        let recent = vec![(1920, 925), (1905, 918), (1890, 906), (1875, 906)];
        assert_eq!(
            f2_isotonic_trend_prediction(&recent, 1860),
            Some(899)
        );
        let prediction =
            f2_predict_frontier_start(&curve, &limits, 1860, Some(881), &recent).unwrap();
        assert_eq!(
            prediction,
            F2FrontierPrediction {
                boundary_mv: 881,
                start_mv: 887,
                used_historical_boundary: true,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_frontier_prediction_falls_back_when_sources_contradict() {
        let curve: Vec<(usize, u32, u32)> =
            [850, 856, 862, 868, 875, 881, 887, 893, 900, 906]
                .into_iter()
                .enumerate()
                .map(|(index, voltage_mv)| (index, voltage_mv, 1800 + index as u32 * 5))
                .collect();
        let limits =
            nidavellir_gpu_nvapi::PositiveOffsetLimits::hardware_frontier(850, 1935, 1800);
        let recent = vec![(1905, 918), (1890, 906), (1875, 906)];
        assert!(
            f2_predict_frontier_start(&curve, &limits, 1860, Some(850), &recent).is_none(),
            "a >25 mV disagreement must preserve the sequential fallback"
        );
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
    fn f2_synthesis_scores_sustained_p99_at_apply_bin() {
        let mut godforge = fp(1950, 150.0);
        godforge.max_power_w = 200.0;
        godforge.power_p99_w = Some(198.0);
        godforge.boundary_voltage_mv = Some(950);
        godforge.p5_clock_mhz = Some(1950);

        let mut near = fp(1920, 145.0);
        near.max_power_w = 170.0;
        near.power_p99_w = Some(168.0);
        near.boundary_voltage_mv = Some(925);
        near.p5_clock_mhz = Some(1920);
        near.perf_per_watt = 1920.0 / 168.0;

        let mut deeper = fp(1875, 120.0);
        deeper.max_power_w = 150.0;
        deeper.power_p99_w = Some(148.0);
        deeper.boundary_voltage_mv = Some(900);
        deeper.p5_clock_mhz = Some(1875);
        deeper.perf_per_watt = 1875.0 / 148.0;

        let profiles = synthesize_forge_profiles(
            &[(godforge, 0.95), (near, 0.95), (deeper, 0.95)],
            &ForgePolicy::balanced(),
        );
        assert_eq!(
            profiles.brokkrs.unwrap().clock_mhz,
            1920,
            "sustained-p99 R must beat the mean-power choice"
        );
    }

    #[cfg(windows)]
    #[test]
    fn f2_off_cap_gate_excludes_at_cap_godforge() {
        // v13.1: the top clock peaks within 6% of the cap → forced voltage droop below Vmin → TDR.
        // Reproduces Godforge 1920@918 (peak ~190 W at the 200 W cap) vs the honest off-cap 1905.
        let mk = |clk: u32, peak: f32, p99: f32, mv: u32| {
            let mut p = fp(clk, p99 - 2.0);
            p.max_power_w = peak;
            p.power_p99_w = Some(p99);
            p.boundary_voltage_mv = Some(mv);
            p.p5_clock_mhz = Some(clk);
            p
        };
        let frontier = vec![
            (mk(1920, 190.0, 188.0, 918), 0.95), // peak 190 > ceiling 188 → excluded
            (mk(1905, 186.0, 184.0, 906), 0.95), // peak 186 ≤ 188 → honest Godforge
            (mk(1770, 157.0, 155.0, 825), 0.95),
        ];
        // Cap known → at-cap top excluded; Godforge drops to the highest off-cap clock.
        let capped =
            synthesize_forge_profiles_capped(&frontier, &ForgePolicy::balanced(), 200.0);
        assert_eq!(capped.godforge.unwrap().clock_mhz, 1905, "at-cap top must be excluded");
        assert!(capped.log.iter().any(|l| l.contains("off-cap gate excluded")));
        // Cap unknown → gate is a no-op (legacy behaviour keeps the highest clock).
        let uncapped = synthesize_forge_profiles(&frontier, &ForgePolicy::balanced());
        assert_eq!(uncapped.godforge.unwrap().clock_mhz, 1920, "no cap → no off-cap gate");
    }

    #[cfg(windows)]
    #[test]
    fn f2_off_cap_gate_fails_closed_when_all_at_cap() {
        // Every qualified point reaches the cap → no off-cap profile exists → publish nothing so
        // Apply stays blocked (never ship a TDR-prone at-cap profile). Also covers zero/unknown peak:
        // it cannot prove headroom, so it is excluded (fail-closed) rather than trusted.
        let mk = |clk: u32, peak: f32, mv: u32| {
            let mut p = fp(clk, peak - 2.0);
            p.max_power_w = peak;
            p.power_p99_w = Some(peak - 2.0);
            p.boundary_voltage_mv = Some(mv);
            p.p5_clock_mhz = Some(clk);
            p
        };
        let frontier = vec![
            (mk(1935, 200.0, 925), 0.95), // at cap
            (mk(1920, 195.0, 918), 0.95), // peak 195 > ceiling 188
            (mk(1905, 190.0, 906), 0.95), // peak 190 > ceiling 188
        ];
        let p = synthesize_forge_profiles_capped(&frontier, &ForgePolicy::balanced(), 200.0);
        assert!(
            p.godforge.is_none() && p.brokkrs.is_none() && p.deep_calm.is_none(),
            "all-at-cap frontier must publish nothing (fail closed)"
        );
        assert!(p.log.iter().any(|l| l.contains("Apply stays blocked")));

        // A point with NO usable power measurement cannot prove headroom → fail closed on a known cap.
        let mut unknown = fp(1900, 0.0);
        unknown.max_power_w = 0.0;
        unknown.power_p99_w = None;
        assert!(!is_off_cap_safe(&unknown, 200.0), "unknown power → fail closed");
        assert!(is_off_cap_safe(&unknown, 0.0), "unknown cap → gate no-op (fail open)");
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
        // Hard power-capped frontier (200 W); this test verifies selection at the 98% Brokkr's
        // floor it was designed for, so it pins the policy explicitly (decoupled from the default,
        // which relaxed to 95%).
        let frontier = vec![
            (fp(1830, 190.0), 0.95),
            (fp(1815, 177.0), 0.95),
            (fp(1800, 170.0), 0.95),
            (fp(1770, 156.0), 0.95),
            (fp(1740, 150.0), 0.95),
        ];
        let policy = ForgePolicy { brokkrs_min_clock_frac: 0.98, deep_calm_min_clock_frac: 0.90, confidence_threshold: 0.85,
        };
        let p = synthesize_forge_profiles(&frontier, &policy);
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
        // Pins the 98% Brokkr's floor explicitly (decoupled from the default, which relaxed to 95%)
        // so the test keeps verifying max-R selection at the floor it was designed for.
        let policy = ForgePolicy { brokkrs_min_clock_frac: 0.98, deep_calm_min_clock_frac: 0.90, confidence_threshold: 0.85,
        };
        let p = synthesize_forge_profiles(&frontier, &policy);
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
        // Pins the 98% floor explicitly (decoupled from the default, which relaxed to 95%) so the
        // boundary semantics this test exercises stay anchored to 0.98.
        let policy = ForgePolicy { brokkrs_min_clock_frac: 0.98, deep_calm_min_clock_frac: 0.90, confidence_threshold: 0.85,
        };
        let at_floor = vec![(fp(2000, 200.0), 0.95), (fp(1960, 150.0), 0.95)];
        let p = synthesize_forge_profiles(&at_floor, &policy);
        assert_eq!(p.brokkrs.unwrap().clock_mhz, 1960, "exactly at floor → eligible");

        let below_floor = vec![(fp(2000, 200.0), 0.95), (fp(1959, 150.0), 0.95)];
        let p2 = synthesize_forge_profiles(&below_floor, &policy);
        assert_eq!(p2.brokkrs.unwrap().clock_mhz, 2000, "below floor → no candidate → Godforge");
    }

    #[cfg(windows)]
    #[test]
    fn balanced_policy_relaxed_brokkrs_floor_to_95() {
        // The default daily-use policy relaxed Brokkr's floor 0.98 → 0.95 (Deep Calm stays 0.90).
        let b = ForgePolicy::balanced();
        assert_eq!(b.brokkrs_min_clock_frac, 0.95, "Brokkr's floor relaxed to 95%");
        assert_eq!(b.deep_calm_min_clock_frac, 0.90, "Deep Calm floor unchanged at 90%");
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
            cancelled: false,
            clock_mhz: 1815,
            power_w: 180.0,
            max_power_w: 188.0,
            power_p99_w: Some(186.0),
            power_std_w: 2.0,
            capped_frac: 0.2,
            volt_mv: 869,
            sample_count: 120, // ≥100 → High clock/power quality
            duration_ms: 15_000,
            min_clock_mhz: 1770,
            p5_clock_mhz: 1800,
            p95_clock_mhz: 1830,
            volt_min_mv: Some(840),
            volt_avg_mv: Some(862),
            volt_max_mv: Some(869),
            volt_sample_count: 24, // 10..=49 → Medium voltage quality
            start_temp_c: Some(60.0),
            end_temp_c: Some(66.0),
            avg_temp_c: Some(63.0),
            max_temp_c: Some(66.0),
            thermal_throttled: false,
            render_frames: Some(900),
            render_fps: Some(60.0),
            qualification_coverage: None,
            prehang_stall_detected: false,
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
        let r = build_frontier(&[1830u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1800], &d, &ForgePolicy::balanced(), &carry, None, false, probe,
        );
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
        FrontierDescent { bins_desc, safe_start_mv, voltage_step_mv: step, lowest_safe_mv: floor,
        }
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
        let d = derive_descent(&seed.cluster_bins_mv, seed.safe_start_mv, FRONTIER_VOLT_STEP_MV,
        );
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
        let _ = build_frontier(&[1935, 1905, 1875], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[2000u32], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1815, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1800, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
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
        let r = build_frontier(&[1830u32, 1770], &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, probe,
        );
        assert!(r.frontier.is_empty());
        assert!(r.profiles.godforge.is_none());
        assert!(r.profiles.brokkrs.is_none());
        assert!(r.profiles.deep_calm.is_none());
    }

    // ── F1b warm-start voltage-bracket carry-forward (pure scheduler primitive) ────────
    /// build-frontier-shaped carry config: cap 1075, floor 875, step 25, margin 1 step.
    #[cfg(windows)]
    fn carry_cfg(enabled: bool) -> BracketCarryConfig {
        BracketCarryConfig { enabled, safe_start_cap_mv: 1075, floor_mv: 875, step_mv: 25, margin_steps: 1,
        }
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
        let off = build_frontier(&targets, &d, &ForgePolicy::balanced(), &BracketCarryConfig::disabled(&d), None, false, off_probe,
        );

        let on_calls = RefCell::new(0u32);
        let on_probe = |t: u32, v: u32| {
            *on_calls.borrow_mut() += 1;
            if v >= 925 { stable_sample(t, 180.0, 0.95) } else { unstable_sample() }
        };
        let on = build_frontier(&targets, &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, on_probe,
        );

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
            1815 => {
                if vbin >= 925 { stable_sample(1815, 180.0, 0.95) } else { unstable_sample() }
            }
            _ => {
                if vbin >= 1000 { stable_sample(1785, 175.0, 0.95) } else { unverified_probe() }
            }
        };
        let d = step_descent(1075, 25, 875);
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe,
        );
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
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe,
        );
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
        let r = build_frontier(&[1815u32, 1785], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe,
        );
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
        let _ = build_frontier(&[1815u32, 1785, 1755], &d, &ForgePolicy::balanced(), &carry_cfg(true), None, false, probe,
        );
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
        // Descends THROUGH the 6 inert top bins (>= 950 @ pcf 1.0), crosses the knee (925 @ 0.85), and
        // keeps the bounded tail until it has PHASE_B_MIN_USEFUL_POINTS off-cap points (925/900/875/850) —
        // then stops CLEANLY as KneeTailComplete (no longer at the first off-cap point).
        assert_eq!(traj.stop_reason, BracketStop::KneeTailComplete);
        assert_eq!(traj.points.len(), 10, "1075..850 inclusive (stops at the 4th off-cap point)");
        assert_eq!(detect_power_bound_knee(&traj.points), Some(6), "first off-cap point is the 7th (925 mV)");
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, PHASE_B_MIN_USEFUL_POINTS, "captured the richness target (4 useful points)");
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
        let traj = descend_phase_b(1815, 1075, &d, 99, &|_t: u32, _v: u32| {
            pb_sample(1810, 199.0, 1.0)
        });
        assert_eq!(traj.stop_reason, BracketStop::CleanFloor);
        assert_eq!(traj.points.len(), 12, "every bin probed to the floor");
        assert_eq!(detect_power_bound_knee(&traj.points), None);
    }

    // ── F1c follow-up: bounded below-knee tail (steep-knee fix, confirmed run 2026-06-16) ──────────
    /// The confirmed-hardware knee shape: pcf pinned at 1.0 above the knee, then drops STEEPLY below
    /// 0.5 in ONE bin (1.0 → 0.40 at 975 mV), and stays off-cap below. The old policy stopped at that
    /// first off-cap point (1 useful → collapse); the tail policy must continue and capture more.
    #[cfg(windows)]
    fn steep_knee_probe(_t: u32, v: u32) -> ProbeSample {
        if v >= 1000 {
            pb_sample(1825, 199.0, 1.0)
        } else if v == 975 {
            pb_sample(1820, 190.0, 0.40) // knee: 1.0 → 0.40 in one bin (below BIND_CAP_FRAC, as on hardware)
        } else {
            pb_sample(1810, 182.0, 0.20) // below-knee tail
        }
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_steep_knee_captures_tail_not_single_point() {
        // The regression the confirmed run surfaced: a steep knee must NOT truncate to one useful point.
        let d = step_descent(1050, 25, 900); // 1050,1025,1000,975,950,925,900
        let traj = descend_phase_b(1785, 1050, &d, 12, &steep_knee_probe);
        assert_eq!(traj.stop_reason, BracketStop::KneeTailComplete);
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert!(
            useful >= MIN_USEFUL_FRONTIER_POINTS,
            "captured a below-knee tail, not a single point (got {useful})",
        );
        assert_eq!(detect_power_bound_knee(&traj.points), Some(3), "knee at 975 mV (idx 3)");
        // The first off-cap point (975) did NOT end the descent — it continued to 950.
        assert!(traj.points.iter().any(|(p, _)| p.vf_table_voltage_mv == Some(950)));
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_tail_stops_when_enough_useful_points() {
        // A clean knee: the tail stops as soon as it has PHASE_B_MIN_USEFUL_POINTS off-cap points and
        // does NOT keep descending further.
        let d = step_descent(1050, 25, 800);
        let traj = descend_phase_b(1785, 1050, &d, 24, &steep_knee_probe);
        assert_eq!(traj.stop_reason, BracketStop::KneeTailComplete);
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, PHASE_B_MIN_USEFUL_POINTS, "stops at the richness target, not deeper");
        // Knee at 975; off-cap points 975/950/925/900 → stops at the 4th (900); 875 is never probed.
        assert!(traj.points.iter().any(|(p, _)| p.vf_table_voltage_mv == Some(900)));
        assert!(!traj.points.iter().any(|(p, _)| p.vf_table_voltage_mv == Some(875)));
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_tail_bounded_by_post_knee_bins() {
        // A JITTERY plateau: only the knee bin (975) is off-cap; the next bins bounce BACK on-cap, so
        // the useful target is never met — the tail must still stop at PHASE_B_POST_KNEE_TAIL_BINS.
        let d = step_descent(1050, 25, 800); // 1050,1025,1000,975,950,925,900,...
        let probe = |_t: u32, v: u32| {
            if v == 975 { pb_sample(1800, 185.0, 0.80) } else { pb_sample(1810, 199.0, 1.0) }
        };
        let traj = descend_phase_b(1785, 1050, &d, 24, &probe);
        assert_eq!(traj.stop_reason, BracketStop::KneeTailComplete);
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, 1, "jittery: only the knee bin was off-cap");
        // Tail probed exactly PHASE_B_POST_KNEE_TAIL_BINS bins from the knee (975,950,925,900,875); not a 6th.
        assert!(traj.points.iter().any(|(p, _)| p.vf_table_voltage_mv == Some(875)));
        assert!(!traj.points.iter().any(|(p, _)| p.vf_table_voltage_mv == Some(850)));
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_verifier_failure_after_knee_stops_immediately() {
        // After the knee (1 off-cap point), a verifier failure must STOP the tail at once — never
        // descend through an unverified ceiling.
        let d = step_descent(1050, 25, 800);
        let probe = |_t: u32, v: u32| {
            if v >= 1000 {
                pb_sample(1810, 199.0, 1.0)
            } else if v == 975 {
                pb_sample(1800, 185.0, 0.80) // knee (off-cap, 1 useful)
            } else {
                unverified_probe() // 950 onward: ceiling won't verify
            }
        };
        let traj = descend_phase_b(1785, 1050, &d, 24, &probe);
        assert_eq!(traj.stop_reason, BracketStop::SoftUnverified, "verify failure wins over the tail");
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, 1, "only the knee point captured before the verify failure");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_instability_after_knee_stops_immediately() {
        // After the knee, dwell instability must STOP the tail at once — never descend blindly.
        let d = step_descent(1050, 25, 800);
        let probe = |_t: u32, v: u32| {
            if v >= 1000 {
                pb_sample(1810, 199.0, 1.0)
            } else if v == 975 {
                pb_sample(1800, 185.0, 0.80)
            } else {
                unstable_sample() // verified, but the dwell is unstable
            }
        };
        let traj = descend_phase_b(1785, 1050, &d, 24, &probe);
        assert_eq!(traj.stop_reason, BracketStop::SoftUnstable, "instability wins over the tail");
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, 1);
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_budget_bounds_tail() {
        // A small Phase-B budget caps total probes even mid-tail (before a 2nd useful point).
        let d = step_descent(1050, 25, 800);
        let probe = |_t: u32, v: u32| {
            if v >= 1000 { pb_sample(1810, 199.0, 1.0) } else { pb_sample(1800, 185.0, 0.80) }
        };
        // budget 4: probes 1050/1025/1000 (on-cap) then 975 (knee, useful=1) → budget hit next iteration.
        let traj = descend_phase_b(1785, 1050, &d, 4, &probe);
        assert_eq!(traj.probes_used, 4);
        assert_eq!(traj.stop_reason, BracketStop::PerTargetCap, "--phase-b-probes caps the tail");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_global_budget_bounds_tail() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // The global --max-probes (modeled in the closure) drains the tail even with a big Phase-B budget.
        let d = step_descent(1050, 25, 800);
        let calls = AtomicU32::new(0);
        let max = 4u32;
        let probe = |_t: u32, v: u32| {
            if calls.fetch_add(1, SeqCst) >= max {
                return budget_sample();
            }
            if v >= 1000 { pb_sample(1810, 199.0, 1.0) } else { pb_sample(1800, 185.0, 0.80) }
        };
        let traj = descend_phase_b(1785, 1050, &d, 24, &probe);
        assert_eq!(traj.stop_reason, BracketStop::BudgetExhausted, "global --max-probes drains the tail");
    }

    #[cfg(windows)]
    #[test]
    fn phase_b_floor_bounds_tail() {
        // The knee is the hardware-floor bin: only 1 off-cap point exists, so the tail can never reach
        // the useful target and stops cleanly at the floor (never probes below it).
        let d = step_descent(1050, 25, 1000); // bins 1050,1025,1000 (floor 1000)
        let probe = |_t: u32, v: u32| {
            if v >= 1025 { pb_sample(1810, 199.0, 1.0) } else { pb_sample(1800, 185.0, 0.80) }
        };
        let traj = descend_phase_b(1785, 1050, &d, 24, &probe);
        assert_eq!(traj.stop_reason, BracketStop::CleanFloor, "the hardware floor bounds the tail");
        let useful = traj.points.iter().filter(|(p, _)| !is_power_bound_point(p)).count();
        assert_eq!(useful, 1, "only the floor bin was off-cap");
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
            (probe_to_point(1935, 1025, &pb_sample(1810, 199.0, 1.0)), 0.21,
            ),
            (probe_to_point(1815, 1025, &pb_sample(1810, 199.0, 1.0)), 0.21,
            ),
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
            "Phase A probes 1075/1050/1025; Phase B continues at 1000↓ and stops at the 4th off-cap bin",
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
        assert!(on.iter().any(|l| l.contains("bounded below-knee tail")));
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
            |target: u32, vbin: u32| {
            if vbin == 1075 { unverified_probe() } else { bind_sample(target, 0.0)
            }
        };
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
            |target: u32, vbin: u32| {
            if vbin == 1075 { unstable_sample() } else { bind_sample(target, 0.0)
            }
        };
        let (bracket, _p) = descend_target(1800, 1075, &d, None, true, &probe);
        assert_eq!(bracket.stop_reason, BracketStop::SoftUnstable);
    }

    #[cfg(windows)]
    #[test]
    fn bind_seeking_crash_and_abort_precede_binding() {
        let d = step_descent(1075, 25, 875);
        let crash =
            |target: u32, vbin: u32| {
            if vbin == 1075 { crashed_sample() } else { bind_sample(target, 0.0)
            }
        };
        let (b1, _) = descend_target(1800, 1075, &d, None, true, &crash);
        assert_eq!(b1.stop_reason, BracketStop::HardFailure);
        assert!(b1.is_hard_failed());
        let abort =
            |target: u32, vbin: u32| {
            if vbin == 1075 { aborted_sample() } else { bind_sample(target, 0.0)
            }
        };
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
            |target: u32, vbin: u32| {
            if vbin == 1075 { budget_sample() } else { bind_sample(target, 0.0)
            }
        };
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
        assert_eq!(p.boundary_voltage_mv, None);
        assert_eq!(p.apply_margin_mv, None);
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
    fn p95_clock_reports_upper_sustained_regime_without_using_raw_max_only() {
        let mut clocks = vec![1890; 95];
        clocks.extend([1905, 1905, 1920, 1950, 2100]);
        assert_eq!(p95_clock_mhz(&clocks), Some(1905));
        assert_eq!(p95_clock_mhz(&[1890]), Some(1890));
        assert_eq!(p95_clock_mhz(&[]), None);
    }

    #[test]
    fn sustained_power_p99_discards_one_sample_spike_and_documents_small_n_fallback() {
        assert_eq!(POWER_PEAK_PERCENTILE, 99);

        let mut full_window = vec![200.0; POWER_PEAK_MIN_SAMPLES - 1];
        full_window.push(500.0);
        assert_eq!(sustained_power_percentile(&full_window), Some(200.0));

        let mut short_window = vec![200.0; POWER_PEAK_MIN_SAMPLES - 2];
        short_window.push(500.0);
        assert_eq!(
            sustained_power_percentile(&short_window),
            Some(500.0),
            "n < 100 falls back to the measured raw maximum"
        );
        assert_eq!(sustained_power_percentile(&[]), None);
        assert_eq!(sustained_power_percentile(&[f32::NAN]), None);
    }

    #[cfg(windows)]
    #[test]
    fn prehang_stall_signal_requires_a_prior_valid_sample_and_threshold() {
        assert!(!prehang_stall_signal(false, PREHANG_STALL_MS * 2));
        assert!(!prehang_stall_signal(true, PREHANG_STALL_MS - 1));
        assert!(prehang_stall_signal(true, PREHANG_STALL_MS));
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
        prog.phase = "descend".into();
        prog.running = true; // a running snapshot must restore as NOT running
        prog.learned_points = 7;
        prog.stock_clock_mhz = 1786;
        prog.points = vec![fp(1830, 200.0), fp(1815, 181.0)];
        prog.godforge = Some(fp(1830, 200.0));
        prog.brokkrs = Some(fp(1815, 181.0));

        let json = encode_forge_state("RTX-TEST", &prog).expect("encode");
        match decode_forge_state(&json, "RTX-TEST") {
            ForgeStateLoad::Loaded(p) => {
                assert!(!p.running, "restored progress must never be running");
                assert_eq!(p.phase, "interrupted");
                assert_eq!(p.learned_points, 7);
                assert!(p.note.as_deref().unwrap_or_default().contains("7 dwell"));
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
    fn restored_pre_v7_f2_profiles_become_provisional() {
        let mut prog = idle();
        prog.is_undervolt = true;
        prog.phase = "finished".into();
        prog.profiles_qualified = true;
        prog.godforge = Some(fp(1890, 190.0));
        prog.brokkrs = Some(fp(1860, 180.0));
        prog.deep_calm = Some(fp(1800, 160.0));

        let json = encode_forge_state("RTX-TEST", &prog).expect("encode");
        match decode_forge_state(&json, "RTX-TEST") {
            ForgeStateLoad::Loaded(restored) => {
                assert!(!restored.profiles_qualified);
                assert_eq!(restored.phase, "provisional");
                assert!(restored
                    .note
                    .as_deref()
                    .unwrap_or_default()
                    .contains("v8"));
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn legacy_power_progress_defaults_live_f2_fields() {
        let mut value = serde_json::to_value(PowerSweepProgress::default()).expect("encode");
        let object = value.as_object_mut().expect("object");
        for field in [
            "mode",
            "current_clock_mhz",
            "current_voltage_mv",
            "completed_steps",
            "total_steps_estimate",
            "elapsed_ms",
            "estimated_remaining_ms",
            "estimated_total_upper_ms",
            "cmax_clock_mhz",
            "frontier_floor_clock_mhz",
            "frontier_clock_count",
            "learned_points",
            "last_outcome",
            "learning_saved",
            "frontier_complete",
            "profiles_qualified",
        ] {
            object.remove(field);
        }
        let restored: PowerSweepProgress = serde_json::from_value(value).expect("legacy decode");
        assert_eq!(restored.completed_steps, 0);
        assert_eq!(restored.total_steps_estimate, 0);
        assert_eq!(restored.estimated_total_upper_ms, None);
        assert_eq!(restored.cmax_clock_mhz, None);
        assert_eq!(restored.frontier_floor_clock_mhz, None);
        assert_eq!(restored.frontier_clock_count, None);
        assert!(!restored.learning_saved);
        assert!(!restored.frontier_complete);
        assert!(!restored.profiles_qualified);
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
