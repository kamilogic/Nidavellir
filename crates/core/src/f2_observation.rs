//! F2 discovery/learning store — records every F2 true-undervolt attempt with its full outcome, and
//! derives the learned F2 frontier the existing GPU profile classifiers consume.
//!
//! This is OBSERVATION / LEARNING data ONLY. It is NOT profile persistence and it NEVER applies,
//! persists, or promotes a selected profile. It records what an F2 attempt did (anchor, offset, caps,
//! verifier/dwell outcome, telemetry, safety outcome) so future runs can pick up where the last left
//! off, compute the minimum stable voltage per target, and hand a learned frontier to the existing
//! `synthesize_forge_profiles` classifier (via [`to_power_sweep_point`]) WITHOUT re-implementing
//! profile scoring.
//!
//! Persistence mirrors the [`crate::safe_loop::SafeLoopStore`] conventions (reuses
//! [`crate::safe_loop::default_data_dir`], serde, BOM-tolerant reads) but is APPEND-ONLY (JSONL — one
//! observation per line) because observations accumulate across runs rather than overwriting a single
//! record. Only the confirmed F2 motor appends; a dry-run never writes.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ipc::PowerSweepPoint;
use crate::safe_loop::default_data_dir;

/// The append-only F2 observation log filename under `default_data_dir()`.
pub const F2_OBSERVATIONS_FILE: &str = "f2_observations.jsonl";

/// Which F2 path produced an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum F2ObsMode {
    /// The official autonomous unknown-GPU path (anchored, conservative offset caps).
    DefaultProgressive,
    /// The explicit operator-provided dev/known-GPU shortcut (larger bounded cap).
    ManualPrior,
    /// A same-target autonomous sweep for the minimum stable voltage.
    TargetSweep,
    /// A multi-target ladder of target sweeps.
    LadderSweep,
}

/// The verifier verdict, in an owned serializable form (the service-side `PositiveOffsetVerification`
/// is crate-private, so observations carry this mirror instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2ObsVerifier {
    RaiseVerified,
    RaiseIncomplete,
    OverRaise,
    Unverifiable,
    /// No verify ran (e.g. the candidate was refused by the planner / a safety gate before any write).
    NotRun,
}

/// The dwell verdict, in an owned serializable form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2ObsDwell {
    Stable,
    Unstable,
    DeviceLost,
    ClockDrop,
    /// No dwell ran (arm/apply/verify failed first, or the candidate was refused before any write).
    NotRun,
}

/// Terminal outcome of an F2 attempt — a superset spanning planner refusal, the confirmed motor's
/// failure classes, and safety-gate aborts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2ObsOutcome {
    /// Dwell stable, reset confirmed, boot flag cleared — a good (stable) undervolt point.
    Validated,
    /// The planner refused the candidate (offset cap / floor / non-real bin / sanity).
    RejectedByPlanner,
    /// The post-write verify did not confirm the anchored raise.
    VerifierFailed,
    /// The dwell reported instability / silent error (no device loss).
    Unstable,
    /// The dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// The clock sagged below tolerance under load (held the dwell but not the clock).
    ClockDrop,
    /// `reset_to_stock` could not be confirmed — a SAFETY failure (boot flag retained, fail closed).
    ResetFailed,
    /// The candidate intent was blacklisted (known-bad).
    Blacklisted,
    /// A crash/TDR that Safe Loop recovered from — learning data, recovery remained safe.
    CrashOrRecovery,
    /// A safety gate (Safe Mode, armed flag, crash threshold, blacklist preflight) aborted the run.
    AbortedBySafetyGate,
}

impl F2ObsOutcome {
    /// A good point: the undervolt held stably AND the GPU reset cleanly.
    pub fn is_validated(self) -> bool {
        matches!(self, F2ObsOutcome::Validated)
    }

    /// A real instability/failure at this voltage (the undervolt did NOT hold) — distinct from a
    /// planner/safety-gate refusal that performed no write. Used to compute `first_bad` / `is_known_bad`.
    pub fn is_bad(self) -> bool {
        matches!(
            self,
            F2ObsOutcome::VerifierFailed
                | F2ObsOutcome::Unstable
                | F2ObsOutcome::DeviceLost
                | F2ObsOutcome::ClockDrop
                | F2ObsOutcome::ResetFailed
                | F2ObsOutcome::Blacklisted
                | F2ObsOutcome::CrashOrRecovery
        )
    }

    /// A safety-critical outcome: the run MUST stop and must not continue to a deeper candidate. A
    /// `ResetFailed` left the GPU potentially un-reset (boot flag retained); a crash/recovery means the
    /// run cannot be trusted to continue. Ordinary instability (`Unstable`/`ClockDrop`/`VerifierFailed`)
    /// is NOT a safety failure when the reset was clean — it is normal learning data.
    pub fn is_safety_failure(self) -> bool {
        matches!(self, F2ObsOutcome::ResetFailed | F2ObsOutcome::CrashOrRecovery)
    }
}

/// One recorded F2 attempt with its full outcome. Append-only; persisted as one JSON object per line.
/// Optional fields use `#[serde(default)]` so partial/older lines still decode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct F2Observation {
    /// Stable id for the run that produced this observation (caller-provided; groups ladder candidates).
    pub run_id: String,
    /// RFC3339 timestamp (caller-provided so pure queries/tests stay deterministic).
    pub timestamp: String,
    /// GPU identity (the NVAPI curve name), when available.
    #[serde(default)]
    pub gpu_key: Option<String>,
    pub mode: F2ObsMode,
    pub target_mhz: u32,
    #[serde(default)]
    pub requested_start_mv: Option<u32>,
    pub anchor_mv: u32,
    pub base_mhz: u32,
    pub offset_mhz: i32,
    pub positive_offset_cap_mhz: i32,
    #[serde(default)]
    pub higher_bins_capped: u32,
    #[serde(default)]
    pub max_flatten_mhz: i32,
    #[serde(default)]
    pub lower_bins_elastic: u32,
    pub verifier_result: F2ObsVerifier,
    pub dwell_result: F2ObsDwell,
    #[serde(default)]
    pub avg_clock_mhz: Option<u32>,
    /// Sustained (p5) clock under load.
    #[serde(default)]
    pub sustained_clock_mhz: Option<u32>,
    #[serde(default)]
    pub watts: Option<u32>,
    #[serde(default)]
    pub silent_error: bool,
    #[serde(default)]
    pub device_lost: bool,
    #[serde(default)]
    pub unstable: bool,
    #[serde(default)]
    pub clock_drop: bool,
    /// A TDR/crash that Safe Loop detected/recovered (when distinguishable).
    #[serde(default)]
    pub tdr_or_crash: bool,
    #[serde(default)]
    pub reset_to_stock_attempted: bool,
    #[serde(default)]
    pub reset_to_stock_ok: bool,
    #[serde(default)]
    pub boot_flag_cleared: bool,
    #[serde(default)]
    pub blacklisted: bool,
    pub outcome: F2ObsOutcome,
    /// Per-attempt confidence basis (0–1) when known; the frontier recomputes an aggregate confidence.
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// The known voltage bracket around the minimum stable voltage `Vmin` for a target. In a (monotone)
/// descent every Validated point is `>= Vmin` and every bad point is `< Vmin`, so
/// `first_bad_mv < Vmin <= last_good_mv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoltageBracket {
    /// Highest voltage that FAILED (a lower bound below `Vmin`).
    pub first_bad_mv: u32,
    /// Lowest voltage that VALIDATED (an upper bound at/above `Vmin`).
    pub last_good_mv: u32,
    /// `last_good_mv - first_bad_mv` — how tightly `Vmin` is bracketed.
    pub width_mv: u32,
}

/// One learned F2 frontier entry per target: the best (lowest-voltage) validated undervolt point plus
/// the search state (first-bad, bracket, counts, confidence). This is the BRIDGE from discovery to the
/// existing profile classifiers — NOT profile selection. Persisted/reportable and serde-friendly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct F2FrontierEntry {
    pub target_mhz: u32,
    /// Best validated anchor voltage (the minimum stable voltage discovered so far).
    pub best_anchor_mv: u32,
    pub offset_mhz: i32,
    #[serde(default)]
    pub watts: Option<u32>,
    #[serde(default)]
    pub avg_clock_mhz: Option<u32>,
    #[serde(default)]
    pub sustained_clock_mhz: Option<u32>,
    /// Aggregate confidence (0–1) from repeat validations at `best_anchor_mv`.
    pub confidence: f64,
    #[serde(default)]
    pub first_bad_mv: Option<u32>,
    #[serde(default)]
    pub bracket_width_mv: Option<u32>,
    pub observation_count: usize,
    /// Timestamp of the best observation (RFC3339).
    pub last_updated: String,
    #[serde(default)]
    pub safety_notes: Option<String>,
}

/// Current UTC timestamp as RFC3339 (the project's timestamp convention; see `safe_loop`). Kept here so
/// service callers need not depend on `chrono` directly.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A run id unique per F2 run (groups all candidate observations of one sweep/ladder run). Format:
/// `<prefix>-<unix_millis>`.
pub fn new_run_id(prefix: &str) -> String {
    format!("{prefix}-{}", chrono::Utc::now().timestamp_millis())
}

/// Pure, BOM-tolerant JSONL parser: one observation per line, blank/malformed lines skipped (so a
/// truncated final line from a crash never invalidates the whole log).
pub fn parse_observations(data: &str) -> Vec<F2Observation> {
    data.lines()
        .map(|l| l.trim_start_matches('\u{feff}').trim())
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<F2Observation>(l).ok())
        .collect()
}

/// Confidence (0–1) for a learned frontier point given how many times that exact point validated.
/// Deliberately simple and SEPARATE from the F1b Wilson trial model (this is F2 learning, not profile
/// scoring): one clean validation already clears the balanced classifier threshold (0.85), and repeats
/// raise it toward a 0.99 ceiling. Pure + monotone non-decreasing in `validated_count`.
pub fn frontier_confidence(validated_count: usize) -> f64 {
    if validated_count == 0 {
        return 0.0;
    }
    (0.86 + 0.03 * (validated_count as f64 - 1.0)).min(0.99)
}

/// The last good (minimum stable) anchor for a target: the LOWEST-voltage Validated observation. This
/// is the best undervolt found so far (lower voltage = deeper undervolt). `None` if none validated.
pub fn last_good_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<&F2Observation> {
    obs.iter()
        .filter(|o| o.target_mhz == target_mhz && o.outcome.is_validated())
        .min_by_key(|o| o.anchor_mv)
}

/// The first bad anchor for a target: the HIGHEST-voltage real-failure observation — the shallowest
/// undervolt that already failed (the closest failure below the validated region). `None` if none bad.
pub fn first_bad_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<&F2Observation> {
    obs.iter()
        .filter(|o| o.target_mhz == target_mhz && o.outcome.is_bad())
        .max_by_key(|o| o.anchor_mv)
}

/// The known voltage bracket for a target: `Vmin` lies in `(first_bad_mv, last_good_mv]`. Returns
/// `None` unless BOTH a validated and a bad point exist AND `last_good > first_bad` (a consistent,
/// monotone descent — if a failure sits at/above the lowest validated point the data is inconsistent
/// and no bracket is claimed).
pub fn bracket_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<VoltageBracket> {
    let last_good_mv = last_good_for_target(obs, target_mhz)?.anchor_mv;
    let first_bad_mv = first_bad_for_target(obs, target_mhz)?.anchor_mv;
    if last_good_mv > first_bad_mv {
        Some(VoltageBracket { first_bad_mv, last_good_mv, width_mv: last_good_mv - first_bad_mv })
    } else {
        None
    }
}

/// True if `(target, anchor_mv)` is already known bad: there is a prior real-failure observation at
/// that voltage OR ANY HIGHER voltage for the target. Conservative — a failure at voltage `V` (too low
/// to hold the clock) implies `V` and everything LOWER is at least as risky.
pub fn is_known_bad(obs: &[F2Observation], target_mhz: u32, anchor_mv: u32) -> bool {
    obs.iter()
        .any(|o| o.target_mhz == target_mhz && o.outcome.is_bad() && o.anchor_mv >= anchor_mv)
}

/// The validated descent baseline for chained same-target descent: the DEEPEST (lowest-voltage,
/// largest-offset) prior `Validated` observation for `target_mhz` that left the GPU clean — reset
/// confirmed, boot flag cleared, and no instability/crash flags — optionally scoped to `gpu_key` (a
/// baseline learned on a DIFFERENT GPU must never bound this GPU's descent). This is the cross-run
/// RESUME point: the official target sweep measures a candidate's per-step increase against this
/// baseline's `offset_mhz` instead of stock `+0`, so a descent already validated to `+15` may reach a
/// `+30` candidate in one bounded step. Returns `None` when no such point exists (the descent then
/// starts from stock baseline `0` — unchanged first-run behavior). A no-write planner/safety-gate abort
/// (`RejectedByPlanner` / `AbortedBySafetyGate`) is NOT validated, so it never becomes a baseline. The
/// ABSOLUTE offset cap still bounds each candidate independently; this only relaxes the per-step delta.
/// Pure.
pub fn validated_descent_baseline<'a>(
    obs: &'a [F2Observation],
    target_mhz: u32,
    gpu_key: Option<&str>,
) -> Option<&'a F2Observation> {
    obs.iter()
        .filter(|o| o.target_mhz == target_mhz && o.outcome.is_validated())
        // Defensive: a Validated point already implies clean cleanup, but a hand-edited/older log line
        // could disagree — require the clean flags explicitly before trusting it as a resume baseline.
        .filter(|o| o.reset_to_stock_ok && o.boot_flag_cleared)
        .filter(|o| !o.device_lost && !o.unstable && !o.silent_error && !o.clock_drop)
        .filter(|o| match gpu_key {
            Some(k) => o.gpu_key.as_deref() == Some(k),
            None => true,
        })
        .min_by_key(|o| o.anchor_mv)
}

/// Build one learned frontier entry for a target from its observations, or `None` if the target has no
/// Validated observation. Chooses the LOWEST-voltage validated point (the deepest undervolt) and
/// annotates it with first-bad / bracket / counts / aggregate confidence. Pure.
pub fn frontier_entry_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<F2FrontierEntry> {
    let best = last_good_for_target(obs, target_mhz)?;
    let validations_at_best = obs
        .iter()
        .filter(|o| {
            o.target_mhz == target_mhz && o.outcome.is_validated() && o.anchor_mv == best.anchor_mv
        })
        .count();
    let observation_count = obs.iter().filter(|o| o.target_mhz == target_mhz).count();
    let bracket = bracket_for_target(obs, target_mhz);
    Some(F2FrontierEntry {
        target_mhz,
        best_anchor_mv: best.anchor_mv,
        offset_mhz: best.offset_mhz,
        watts: best.watts,
        avg_clock_mhz: best.avg_clock_mhz,
        sustained_clock_mhz: best.sustained_clock_mhz,
        confidence: frontier_confidence(validations_at_best),
        first_bad_mv: first_bad_for_target(obs, target_mhz).map(|o| o.anchor_mv),
        bracket_width_mv: bracket.map(|b| b.width_mv),
        observation_count,
        last_updated: best.timestamp.clone(),
        safety_notes: None,
    })
}

/// Build the full learned F2 frontier: one entry per target that has a Validated observation, sorted by
/// target. Pure — this is the discovery→classifier bridge input.
pub fn learned_frontier(obs: &[F2Observation]) -> Vec<F2FrontierEntry> {
    let mut targets: Vec<u32> = obs.iter().map(|o| o.target_mhz).collect();
    targets.sort_unstable();
    targets.dedup();
    targets
        .into_iter()
        .filter_map(|t| frontier_entry_for_target(obs, t))
        .collect()
}

/// Bridge ONE learned frontier entry to the canonical telemetry point the existing GPU profile
/// classifiers (`synthesize_forge_profiles`) consume, paired with its confidence. F2 RAISES a
/// lower-voltage bin (true undervolt), so the apply axis `vf_table_voltage_mv` is that LOWER anchor bin
/// (the opposite direction from F1's down-cap), the point is non-power-bound (`power_capped_frac = 0`),
/// and `stable = true`. Every field the classifier does not need is left default. Pure — builds DATA
/// only; it never selects, applies, persists, or promotes a profile.
pub fn to_power_sweep_point(entry: &F2FrontierEntry) -> (PowerSweepPoint, f64) {
    let clock = entry.avg_clock_mhz.unwrap_or(entry.target_mhz);
    let power = entry.watts.unwrap_or(0) as f32;
    let perf_per_watt = if power > 0.0 { clock as f64 / power as f64 } else { 0.0 };
    let point = PowerSweepPoint {
        voltage_mv: entry.best_anchor_mv,
        clock_mhz: clock,
        offset_mhz: entry.offset_mhz,
        power_w: power,
        max_power_w: power,
        power_capped_frac: 0.0,
        stable: true,
        perf_per_watt,
        vf_table_voltage_mv: Some(entry.best_anchor_mv),
        p5_clock_mhz: entry.sustained_clock_mhz,
        target_clock_mhz: Some(entry.target_mhz),
        ..Default::default()
    };
    (point, entry.confidence)
}

/// Bridge a whole learned frontier to the `(PowerSweepPoint, confidence)` pairing the existing
/// classifier consumes. Pure; builds data only.
pub fn frontier_to_points(entries: &[F2FrontierEntry]) -> Vec<(PowerSweepPoint, f64)> {
    entries.iter().map(to_power_sweep_point).collect()
}

/// File-backed, append-only F2 observation log. Mirrors [`crate::safe_loop::SafeLoopStore`]'s
/// path/serde conventions (reuses [`default_data_dir`]) but APPENDS one JSON line per observation
/// (JSONL) because observations accumulate. Reads tolerate a leading BOM and skip malformed lines.
/// NEVER written during a dry-run — only the confirmed F2 motor appends.
#[derive(Debug, Clone)]
pub struct F2ObservationStore {
    base: PathBuf,
}

impl F2ObservationStore {
    /// The machine-wide store under `default_data_dir()` (`%ProgramData%/Nidavellir`).
    pub fn system() -> Self {
        Self { base: default_data_dir() }
    }

    /// A store rooted at an explicit base directory (used by tests).
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// The JSONL log path.
    pub fn path(&self) -> PathBuf {
        self.base.join(F2_OBSERVATIONS_FILE)
    }

    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base)
    }

    /// Append one observation as a JSON line. Best-effort (mirrors the project's non-fsync convention);
    /// creates the file/dir on first write.
    pub fn append(&self, obs: &F2Observation) -> std::io::Result<()> {
        self.ensure_dir()?;
        let line = serde_json::to_string(obs)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.path())?;
        writeln!(f, "{line}")
    }

    /// Load every well-formed observation (missing file → empty; malformed lines skipped).
    pub fn load_all(&self) -> Vec<F2Observation> {
        match std::fs::read_to_string(self.path()) {
            Ok(data) => parse_observations(&data),
            Err(_) => Vec::new(),
        }
    }

    /// All observations for a target.
    pub fn query_by_target(&self, target_mhz: u32) -> Vec<F2Observation> {
        self.load_all().into_iter().filter(|o| o.target_mhz == target_mhz).collect()
    }

    /// The learned frontier over the entire log.
    pub fn learned_frontier(&self) -> Vec<F2FrontierEntry> {
        learned_frontier(&self.load_all())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(target: u32, anchor: u32, outcome: F2ObsOutcome) -> F2Observation {
        F2Observation {
            run_id: "run-test".into(),
            timestamp: "2026-06-21T00:00:00Z".into(),
            gpu_key: Some("RTX 3060 Ti".into()),
            mode: F2ObsMode::TargetSweep,
            target_mhz: target,
            requested_start_mv: None,
            anchor_mv: anchor,
            base_mhz: target.saturating_sub(15),
            offset_mhz: 15,
            positive_offset_cap_mhz: 30,
            higher_bins_capped: 26,
            max_flatten_mhz: 150,
            lower_bins_elastic: 40,
            verifier_result: if outcome.is_validated() {
                F2ObsVerifier::RaiseVerified
            } else {
                F2ObsVerifier::Unverifiable
            },
            dwell_result: if outcome.is_validated() {
                F2ObsDwell::Stable
            } else {
                F2ObsDwell::Unstable
            },
            avg_clock_mhz: Some(target + 15),
            sustained_clock_mhz: Some(target + 15),
            watts: Some(180),
            silent_error: false,
            device_lost: false,
            unstable: !outcome.is_validated(),
            clock_drop: false,
            tdr_or_crash: false,
            reset_to_stock_attempted: true,
            reset_to_stock_ok: true,
            boot_flag_cleared: true,
            blacklisted: false,
            outcome,
            confidence: outcome.is_validated().then_some(0.86),
            notes: None,
        }
    }

    #[test]
    fn outcome_classification() {
        assert!(F2ObsOutcome::Validated.is_validated());
        for o in [
            F2ObsOutcome::VerifierFailed,
            F2ObsOutcome::Unstable,
            F2ObsOutcome::DeviceLost,
            F2ObsOutcome::ClockDrop,
            F2ObsOutcome::ResetFailed,
            F2ObsOutcome::Blacklisted,
            F2ObsOutcome::CrashOrRecovery,
        ] {
            assert!(o.is_bad(), "{o:?} should be bad");
        }
        // Planner/gate refusals performed no write → NOT "bad" (no instability learned).
        assert!(!F2ObsOutcome::RejectedByPlanner.is_bad());
        assert!(!F2ObsOutcome::AbortedBySafetyGate.is_bad());
        // Only reset-failure and unrecovered crash are SAFETY failures.
        assert!(F2ObsOutcome::ResetFailed.is_safety_failure());
        assert!(F2ObsOutcome::CrashOrRecovery.is_safety_failure());
        assert!(!F2ObsOutcome::Unstable.is_safety_failure());
        assert!(!F2ObsOutcome::ClockDrop.is_safety_failure());
    }

    #[test]
    fn last_good_is_lowest_validated() {
        let v = vec![
            obs(1800, 975, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::Validated),
            obs(1800, 962, F2ObsOutcome::Validated),
            obs(1800, 956, F2ObsOutcome::Unstable),
        ];
        assert_eq!(last_good_for_target(&v, 1800).unwrap().anchor_mv, 962);
        // A target with no validated point → None.
        assert!(last_good_for_target(&v, 1815).is_none());
    }

    #[test]
    fn first_bad_is_highest_failure() {
        let v = vec![
            obs(1800, 962, F2ObsOutcome::Validated),
            obs(1800, 956, F2ObsOutcome::Unstable),
            obs(1800, 950, F2ObsOutcome::DeviceLost),
        ];
        // The shallowest (highest-voltage) failure brackets Vmin from below.
        assert_eq!(first_bad_for_target(&v, 1800).unwrap().anchor_mv, 956);
    }

    #[test]
    fn bracket_only_when_consistent() {
        let consistent = vec![
            obs(1800, 962, F2ObsOutcome::Validated),
            obs(1800, 956, F2ObsOutcome::Unstable),
        ];
        let b = bracket_for_target(&consistent, 1800).unwrap();
        assert_eq!((b.first_bad_mv, b.last_good_mv, b.width_mv), (956, 962, 6));
        // Inconsistent: a failure at/above the lowest validated point → no bracket claimed.
        let inconsistent = vec![
            obs(1800, 962, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::Unstable),
        ];
        assert!(bracket_for_target(&inconsistent, 1800).is_none());
    }

    #[test]
    fn is_known_bad_is_conservative_downward() {
        let v = vec![obs(1800, 956, F2ObsOutcome::Unstable)];
        // The exact failed voltage and anything LOWER is known bad.
        assert!(is_known_bad(&v, 1800, 956));
        assert!(is_known_bad(&v, 1800, 950));
        // A HIGHER voltage is not implied bad by a lower-voltage failure.
        assert!(!is_known_bad(&v, 1800, 962));
        // Different target is unaffected.
        assert!(!is_known_bad(&v, 1815, 956));
    }

    #[test]
    fn validated_descent_baseline_picks_deepest_clean_validated() {
        let v = vec![
            obs(1800, 975, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::Validated),
            obs(1800, 956, F2ObsOutcome::Unstable), // a real failure is not a baseline
        ];
        // Deepest (lowest-voltage) clean validated point — its offset is the resume baseline.
        let b = validated_descent_baseline(&v, 1800, None).unwrap();
        assert_eq!(b.anchor_mv, 968);
        // A target with no validated point → None (descent then starts from stock 0).
        assert!(validated_descent_baseline(&v, 1815, None).is_none());
    }

    #[test]
    fn validated_descent_baseline_ignores_no_write_aborts_and_other_gpus() {
        // A no-write safety-gate abort is NOT a baseline even at the lowest voltage.
        let mut aborted = obs(1800, 950, F2ObsOutcome::AbortedBySafetyGate);
        aborted.reset_to_stock_ok = true;
        aborted.boot_flag_cleared = true;
        aborted.unstable = false;
        let v = vec![obs(1800, 975, F2ObsOutcome::Validated), aborted];
        assert_eq!(validated_descent_baseline(&v, 1800, None).unwrap().anchor_mv, 975);
        // A validated point on a DIFFERENT GPU must not bound this GPU's descent.
        let mut other_gpu = obs(1800, 962, F2ObsOutcome::Validated);
        other_gpu.gpu_key = Some("RTX 4090".into());
        let v2 = vec![obs(1800, 975, F2ObsOutcome::Validated), other_gpu];
        assert_eq!(
            validated_descent_baseline(&v2, 1800, Some("RTX 3060 Ti")).unwrap().anchor_mv,
            975
        );
        // Unfiltered (gpu_key None) sees both → deepest wins.
        assert_eq!(validated_descent_baseline(&v2, 1800, None).unwrap().anchor_mv, 962);
    }

    #[test]
    fn validated_descent_baseline_rejects_dirty_cleanup_flags() {
        // A record claiming Validated but with an un-cleared boot flag is not trusted as a baseline.
        let mut dirty = obs(1800, 962, F2ObsOutcome::Validated);
        dirty.boot_flag_cleared = false;
        let v = vec![obs(1800, 975, F2ObsOutcome::Validated), dirty];
        assert_eq!(validated_descent_baseline(&v, 1800, None).unwrap().anchor_mv, 975);
    }

    #[test]
    fn frontier_confidence_monotone() {
        assert_eq!(frontier_confidence(0), 0.0);
        assert!(frontier_confidence(1) >= 0.85); // a single clean validation clears the balanced gate
        assert!(frontier_confidence(2) > frontier_confidence(1));
        assert!(frontier_confidence(100) <= 0.99);
    }

    #[test]
    fn learned_frontier_picks_best_per_target() {
        let v = vec![
            obs(1800, 975, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::Validated),
            obs(1800, 962, F2ObsOutcome::Validated),
            obs(1800, 956, F2ObsOutcome::Unstable),
            obs(1815, 980, F2ObsOutcome::Validated),
            obs(1830, 990, F2ObsOutcome::Unstable), // no validated → excluded
        ];
        let fr = learned_frontier(&v);
        // Two targets have a validated point (1800, 1815); 1830 has none → excluded.
        assert_eq!(fr.iter().map(|e| e.target_mhz).collect::<Vec<_>>(), vec![1800, 1815]);
        let e1800 = &fr[0];
        assert_eq!(e1800.best_anchor_mv, 962); // lowest validated
        assert_eq!(e1800.first_bad_mv, Some(956));
        assert_eq!(e1800.bracket_width_mv, Some(6));
        assert_eq!(e1800.observation_count, 4);
        assert!(e1800.confidence >= 0.85);
    }

    #[test]
    fn bridge_builds_classifier_compatible_points() {
        let v = vec![obs(1800, 962, F2ObsOutcome::Validated)];
        let fr = learned_frontier(&v);
        let (p, conf) = to_power_sweep_point(&fr[0]);
        // Classifier-relevant fields: stable, non-power-bound, apply axis = the lower anchor bin.
        assert!(p.stable);
        assert_eq!(p.power_capped_frac, 0.0);
        assert_eq!(p.vf_table_voltage_mv, Some(962));
        assert_eq!(p.target_clock_mhz, Some(1800));
        assert_eq!(p.clock_mhz, 1815);
        assert_eq!(p.p5_clock_mhz, Some(1815));
        assert!(p.perf_per_watt > 0.0);
        assert!(conf >= 0.85); // passes the balanced confidence gate
        // Whole-frontier bridge preserves count.
        assert_eq!(frontier_to_points(&fr).len(), 1);
    }

    #[test]
    fn jsonl_parse_is_bom_tolerant_and_skips_malformed() {
        let good = serde_json::to_string(&obs(1800, 962, F2ObsOutcome::Validated)).unwrap();
        let data = format!("\u{feff}{good}\n\n{{not valid json}}\n{good}\n");
        let parsed = parse_observations(&data);
        assert_eq!(parsed.len(), 2); // both good lines; blank + malformed skipped
        assert_eq!(parsed[0].target_mhz, 1800);
    }

    #[test]
    fn store_append_accumulates_and_queries() {
        // Unique temp base so the JSONL append round-trips without colliding with other tests.
        let base = std::env::temp_dir().join(format!("nidav-f2-obs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = F2ObservationStore::new(&base);
        assert!(store.load_all().is_empty()); // missing file → empty
        store.append(&obs(1800, 968, F2ObsOutcome::Validated)).unwrap();
        store.append(&obs(1800, 962, F2ObsOutcome::Validated)).unwrap();
        store.append(&obs(1815, 980, F2ObsOutcome::Unstable)).unwrap();
        // Append accumulates (does NOT overwrite).
        assert_eq!(store.load_all().len(), 3);
        assert_eq!(store.query_by_target(1800).len(), 2);
        assert_eq!(store.learned_frontier().iter().map(|e| e.target_mhz).collect::<Vec<_>>(), vec![1800]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
