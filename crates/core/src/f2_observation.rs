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
/// Current homogeneous PowerRender discovery contract.
///
/// v4 preserves mean, sustained-p99 and sampled-peak power separately, validates suspicious p99
/// steps with reset-clean repeated dwells, and carries render/voltage telemetry so frontier
/// decisions and exact apply-margin-bin calibration use confirmed sustained-power evidence.
pub const F2_DISCOVERY_CONTRACT_VERSION: u32 = 4;
/// Current FailureSeekingGameLoop qualification contract.
///
/// v7 requires the High-FPS, Texture and Transitions qualification set and reconciles the exact
/// sustained-p95 electrical regime before a point may be applied.
///
/// v8 adds the FrameCadence phase (game-frame-scale heavy burst / short idle cycling with its own
/// stock golden) to all three patterns — evidence qualified without it cannot unlock Apply.
///
/// v9 adds VRAM-pressure and geometry/depth phases plus the fourth Memory pattern; the complete
/// qualification set is now HighFps + Texture + Transitions + Memory.
///
/// v10 rebuilds the texture path: TextureRop now samples a large VRAM-resident source with a
/// per-pixel scattered tap chain (TMU + memory controller together, cache-defeating). v9
/// positives were measured against an L2-resident source and proved optimistic on hardware —
/// they cannot unlock Apply.
///
/// v11 hardens the engine: TextureRop reverts to the L2-resident graceful silent-error detector,
/// the heavy memory sampling moves to the banded TextureStream phase (pre-hang watchdog +
/// stock-referenced degradation gate), patterns are severity-ordered, and the pre-hang stall
/// signal became a failing verdict.
pub const F2_QUALIFICATION_CONTRACT_VERSION: u32 = 11;

/// What kind of evidence one observation contributes. Old JSONL lines default to `Legacy`: they may
/// guide discovery, but can never satisfy the current qualification gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2EvidenceKind {
    #[default]
    Legacy,
    Discovery,
    Qualification,
    ApplyQualification,
}

/// Whether a stable qualification dwell exercised every required phase strongly enough to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2QualificationVerdict {
    Pass,
    Fail,
    Inconclusive,
}

/// Strength of the qualification evidence. Older strengths remain readable for compatibility;
/// FSGL4 is the current three-pattern v7 qualifier required for Apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2QualificationStrength {
    #[default]
    Fsgl1,
    Fsgl2,
    Fsgl3,
    Fsgl4,
}

/// Deterministic workload pattern. A/B remain readable for legacy observations; current (v9)
/// deployability requires the complete [`REQUIRED_QUALIFICATION_PATTERNS`] set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum F2QualificationPattern {
    A,
    B,
    HighFps,
    Texture,
    Transitions,
    Memory,
}

/// The complete pattern set the current qualification contract requires at a boundary and at the
/// exact Apply pair. Completeness checks index into this array — extending it automatically
/// tightens every gate.
pub const REQUIRED_QUALIFICATION_PATTERNS: [F2QualificationPattern; 4] = [
    F2QualificationPattern::HighFps,
    F2QualificationPattern::Texture,
    F2QualificationPattern::Transitions,
    F2QualificationPattern::Memory,
];

fn required_pattern_index(pattern: F2QualificationPattern) -> Option<usize> {
    REQUIRED_QUALIFICATION_PATTERNS.iter().position(|required| *required == pattern)
}

/// Per-phase telemetry captured during a qualification dwell. Optional values remain absent when a
/// driver/sample path could not provide them; they are never fabricated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct F2QualificationPhaseMetric {
    pub phase_name: String,
    pub phase_pattern: String,
    pub duration_ms: u64,
    pub frame_count: u64,
    pub checksum_count: u32,
    pub compute_check_count: u32,
    #[serde(default)]
    pub clock_avg: Option<f32>,
    #[serde(default)]
    pub clock_p5: Option<u32>,
    #[serde(default)]
    pub clock_p50: Option<u32>,
    #[serde(default)]
    pub clock_p95: Option<u32>,
    #[serde(default)]
    pub target_residency_pct: Option<f32>,
    #[serde(default)]
    pub power_avg: Option<f32>,
    #[serde(default)]
    pub power_p95: Option<f32>,
    #[serde(default)]
    pub power_capped_fraction: Option<f32>,
    #[serde(default)]
    pub temperature_avg: Option<f32>,
    #[serde(default)]
    pub temperature_max: Option<f32>,
    pub coverage_status: String,
}

/// Compact, append-only qualification coverage summary. Phase details remain service-internal; this
/// carries the durable facts needed to decide whether a validation may qualify Apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct F2QualificationCoverage {
    #[serde(default)]
    pub strength: F2QualificationStrength,
    #[serde(default)]
    pub pattern: Option<F2QualificationPattern>,
    #[serde(default)]
    pub pass_index: u32,
    pub verdict: F2QualificationVerdict,
    pub phases_completed: u32,
    pub phases_expected: u32,
    pub checksum_count: u32,
    pub sample_count: u32,
    #[serde(default)]
    pub compute_check_count: u32,
    #[serde(default)]
    pub target_residency_frac: Option<f32>,
    #[serde(default)]
    pub heavy_light_power_delta_w: Option<f32>,
    #[serde(default)]
    pub failure_phase: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub phase_metrics: Vec<F2QualificationPhaseMetric>,
}

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
    /// Long current-contract qualification of the exact post-margin Apply pair selected for a profile.
    ApplyQualification,
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
    SilentError,
    Unstable,
    DeviceLost,
    ClockDrop,
    /// Discovery completed reset-clean, but repeated PowerRender measurements did not establish a
    /// consistent sustained-p99 value for this bin.
    PowerTelemetryInconclusive,
    /// Dwell completed reset-clean, but the qualification did not collect enough current-contract
    /// coverage to prove the point.
    QualificationInconclusive,
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
    /// The dwell reported a silent compute error (no device loss).
    SilentError,
    /// The dwell reported instability without a classified silent error (no device loss).
    Unstable,
    /// The dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// The target did not hold yet, but the GPU was still at 99–100% of its power limit. This is a
    /// search-state observation, not a voltage-instability boundary.
    PowerBoundClockDrop,
    /// The clock sagged below tolerance under load (held the dwell but not the clock).
    ClockDrop,
    /// Discovery power telemetry could not be confirmed after the bounded repeat budget. This is
    /// neither stability evidence nor a voltage failure.
    PowerTelemetryInconclusive,
    /// The qualification workload ran reset-clean, but coverage was too weak to qualify or reject the
    /// point. This is not a voltage failure and must not become a bad-boundary veto.
    QualificationInconclusive,
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
                | F2ObsOutcome::SilentError
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
        matches!(
            self,
            F2ObsOutcome::DeviceLost | F2ObsOutcome::ResetFailed | F2ObsOutcome::CrashOrRecovery
        )
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
    #[serde(default)]
    pub evidence_kind: F2EvidenceKind,
    #[serde(default)]
    pub discovery_contract_version: Option<u32>,
    #[serde(default)]
    pub qualification_contract_version: Option<u32>,
    #[serde(default)]
    pub qualification_coverage: Option<F2QualificationCoverage>,
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
    /// Upper sustained (p95) clock under the same load. This reveals the boost regime exercised by
    /// an exact Apply pair without conflating it with the configured target.
    #[serde(default)]
    pub sustained_upper_clock_mhz: Option<u32>,
    #[serde(default)]
    pub watts: Option<u32>,
    /// Highest post-ramp power sample captured by the discovery dwell.
    #[serde(default)]
    pub max_watts: Option<u32>,
    /// Sustained high-power percentile captured from the retained post-ramp dwell samples.
    #[serde(default)]
    pub power_p99_w: Option<f32>,
    /// The discovery p99 passed the v4 adjacent-bin/repeat consistency gate.
    #[serde(default)]
    pub power_p99_confirmed: bool,
    /// Number of reset-clean PowerRender attempts used by the v4 consistency decision.
    #[serde(default)]
    pub power_p99_attempts: u32,
    /// Ramp-filtered voltage telemetry from the dwell; diagnostic only and never fabricated.
    #[serde(default)]
    pub measured_voltage_min_mv: Option<u32>,
    #[serde(default)]
    pub measured_voltage_avg_mv: Option<u32>,
    #[serde(default)]
    pub measured_voltage_max_mv: Option<u32>,
    #[serde(default)]
    pub measured_voltage_sample_count: u32,
    /// Render coverage captured by the workload itself, used to diagnose underloaded dwells.
    #[serde(default)]
    pub render_frames: Option<u64>,
    #[serde(default)]
    pub render_fps: Option<f64>,
    /// Fraction of steady-state samples where the NVIDIA power-cap flag was active.
    #[serde(default)]
    pub power_capped_frac: Option<f32>,
    #[serde(default)]
    pub max_temp_c: Option<f32>,
    /// NVML reported software or hardware thermal slowdown during the dwell.
    #[serde(default)]
    pub thermal_throttled: bool,
    /// Actual wall-clock duration of the dwell that produced this observation.
    #[serde(default)]
    pub dwell_duration_ms: Option<u64>,
    /// Number of retained steady-state clock/power samples.
    #[serde(default)]
    pub sample_count: Option<u32>,
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
    pub max_watts: Option<u32>,
    #[serde(default)]
    pub power_p99_w: Option<f32>,
    #[serde(default)]
    pub avg_clock_mhz: Option<u32>,
    #[serde(default)]
    pub sustained_clock_mhz: Option<u32>,
    #[serde(default)]
    pub sustained_upper_clock_mhz: Option<u32>,
    #[serde(default)]
    pub power_capped_frac: Option<f32>,
    #[serde(default)]
    pub dwell_duration_ms: Option<u64>,
    #[serde(default)]
    pub sample_count: Option<u32>,
    #[serde(default)]
    pub max_temp_c: Option<f32>,
    #[serde(default)]
    pub thermal_throttled: bool,
    /// Aggregate confidence (0–1) from repeat validations at `best_anchor_mv`.
    pub confidence: f64,
    /// Successful confirmations at this exact target/anchor point.
    #[serde(default)]
    pub validation_count: usize,
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
    (1.0 - 0.15_f64.powf(validated_count as f64)).min(0.99)
}

/// Confidence from the evidence actually collected at one exact point. Standard's proven 15 s
/// dwell with at least 100 retained samples contributes one evidence unit (0.85 confidence); shorter
/// dwells contribute less, while longer and independently repeated clean passes mature toward 0.99.
pub fn frontier_confidence_from_evidence(validations: &[&F2Observation]) -> f64 {
    let evidence_units = validations.iter().fold(0.0, |sum, obs| {
        let duration = obs.dwell_duration_ms.unwrap_or(15_000) as f64 / 15_000.0;
        let sample_quality = (obs.sample_count.unwrap_or(100) as f64 / 100.0).min(1.0);
        sum + duration * sample_quality
    });
    if evidence_units <= 0.0 {
        0.0
    } else {
        (1.0 - 0.15_f64.powf(evidence_units)).min(0.99)
    }
}

/// The last good (minimum stable) anchor for a target: the LOWEST-voltage Validated observation. This
/// is the best undervolt found so far (lower voltage = deeper undervolt). `None` if none validated.
pub fn last_good_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<&F2Observation> {
    let first_bad_mv = first_bad_for_target(obs, target_mhz).map(|o| o.anchor_mv);
    obs.iter()
        .filter(|o| o.target_mhz == target_mhz && o.outcome.is_validated())
        // A later failure at V invalidates V and every deeper/lower-voltage point, even if an older
        // run once validated there. Only clean points strictly above the nearest known failure remain.
        .filter(|o| first_bad_mv.is_none_or(|bad| o.anchor_mv > bad))
        .min_by_key(|o| o.anchor_mv)
}

/// True when an observation can define the current PowerRender discovery frontier. Discovery v4
/// requires confirmed p99 evidence, so older or telemetry-inconclusive positives cannot seed the
/// current frontier. Negative observations remain version-independent and conservative.
pub fn is_current_discovery_evidence(o: &F2Observation) -> bool {
    o.evidence_kind == F2EvidenceKind::Discovery
        && o.discovery_contract_version == Some(F2_DISCOVERY_CONTRACT_VERSION)
        && o.power_p99_confirmed
}

/// True only for a fully-covered pass from the current qualification contract. Legacy positives and
/// passes from older qualifiers can guide a start point but can never unlock Apply.
pub fn is_current_qualification_pass(o: &F2Observation) -> bool {
    o.evidence_kind == F2EvidenceKind::Qualification
        && o.qualification_contract_version == Some(F2_QUALIFICATION_CONTRACT_VERSION)
        && o.outcome.is_validated()
        && o.qualification_coverage
            .as_ref()
            .is_some_and(|coverage| {
                coverage.strength == F2QualificationStrength::Fsgl4
                    && coverage.pattern.is_some_and(is_v7_qualification_pattern)
                    && coverage.verdict == F2QualificationVerdict::Pass
            })
}

/// True only for a fully-covered pass at the exact post-margin Apply pair under the current
/// qualification contract. Kept separate from frontier qualification so an Apply failure caused by
/// a higher boost regime does not rewrite the learned voltage boundary.
pub fn is_current_apply_qualification_pass(o: &F2Observation) -> bool {
    o.evidence_kind == F2EvidenceKind::ApplyQualification
        && o.qualification_contract_version == Some(F2_QUALIFICATION_CONTRACT_VERSION)
        && o.outcome.is_validated()
        && o.qualification_coverage
            .as_ref()
            .is_some_and(|coverage| {
                coverage.strength == F2QualificationStrength::Fsgl4
                    && coverage.pattern.is_some_and(is_v7_qualification_pattern)
                    && coverage.verdict == F2QualificationVerdict::Pass
            })
}

fn is_v7_qualification_pattern(pattern: F2QualificationPattern) -> bool {
    required_pattern_index(pattern).is_some()
}

/// Lowest-voltage stable point proven by the current, homogeneous discovery contract. Negative
/// observations remain version-independent and invalidate the same/deeper voltage as before.
pub fn last_discovery_good_for_target(
    obs: &[F2Observation],
    target_mhz: u32,
) -> Option<&F2Observation> {
    let first_bad_mv = first_bad_for_target(obs, target_mhz).map(|o| o.anchor_mv);
    obs.iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && o.outcome.is_validated()
                && is_current_discovery_evidence(o)
        })
        .filter(|o| first_bad_mv.is_none_or(|bad| o.anchor_mv > bad))
        .min_by_key(|o| o.anchor_mv)
}

/// The first bad anchor for a target: the HIGHEST-voltage real-failure observation — the shallowest
/// undervolt that already failed (the closest failure below the validated region). `None` if none bad.
pub fn first_bad_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<&F2Observation> {
    obs.iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && o.evidence_kind != F2EvidenceKind::ApplyQualification
                && o.outcome.is_bad()
        })
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
        Some(VoltageBracket {
            first_bad_mv,
            last_good_mv,
            width_mv: last_good_mv - first_bad_mv,
        })
    } else {
        None
    }
}

/// True if `(target, anchor_mv)` is already known bad: there is a prior real-failure observation at
/// that voltage OR ANY HIGHER voltage for the target. Conservative — a failure at voltage `V` (too low
/// to hold the clock) implies `V` and everything LOWER is at least as risky.
pub fn is_known_bad(obs: &[F2Observation], target_mhz: u32, anchor_mv: u32) -> bool {
    obs.iter()
        .any(|o| {
            o.target_mhz == target_mhz
                && o.evidence_kind != F2EvidenceKind::ApplyQualification
                && o.outcome.is_bad()
                && o.anchor_mv >= anchor_mv
        })
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

/// Crash-proximity margin: a synthesized boundary must sit at least this far (≈ two physical VF
/// bins on modern NVIDIA curves) above the highest crash/TDR anchor observed for the target. A
/// crash means the silent-error threshold above it went undetected — the immediately adjacent bin
/// cannot be trusted just because it happened to pass. Generic rule; never derived from any
/// specific GPU's known points.
pub const F2_CRASH_PROXIMITY_MIN_MV: u32 = 12;

/// Highest crash/TDR/device-loss anchor recorded for a target, if any. Pure.
pub fn crash_floor_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<u32> {
    obs.iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && (o.device_lost || matches!(o.outcome, F2ObsOutcome::DeviceLost))
        })
        .map(|o| o.anchor_mv)
        .max()
}

/// Build one learned frontier entry for a target from its observations, or `None` if the target has no
/// Validated observation. Chooses the LOWEST-voltage validated point (the deepest undervolt) and
/// annotates it with first-bad / bracket / counts / aggregate confidence, enforcing the
/// crash-proximity margin ([`F2_CRASH_PROXIMITY_MIN_MV`]). Pure.
pub fn frontier_entry_for_target(obs: &[F2Observation], target_mhz: u32) -> Option<F2FrontierEntry> {
    let crash_floor = crash_floor_for_target(obs, target_mhz);
    let best = obs
        .iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && o.outcome.is_validated()
                && is_current_discovery_evidence(o)
        })
        .filter(|o| {
            first_bad_for_target(obs, target_mhz)
                .is_none_or(|bad| o.anchor_mv > bad.anchor_mv)
        })
        .filter(|o| {
            crash_floor.is_none_or(|crash_mv| {
                o.anchor_mv >= crash_mv.saturating_add(F2_CRASH_PROXIMITY_MIN_MV)
            })
        })
        .min_by_key(|o| o.anchor_mv)?;
    let evidence_at_best: Vec<&F2Observation> = obs
        .iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && o.outcome.is_validated()
                && o.anchor_mv == best.anchor_mv
                && (is_current_discovery_evidence(o) || is_current_qualification_pass(o))
        })
        .collect();
    let qualification_count = REQUIRED_QUALIFICATION_PATTERNS
    .into_iter()
    .filter(|pattern| {
        evidence_at_best.iter().any(|o| {
            is_current_qualification_pass(o)
                && o.qualification_coverage.as_ref().and_then(|c| c.pattern)
                    == Some(*pattern)
        })
    })
    .count();
    let observation_count = obs.iter().filter(|o| o.target_mhz == target_mhz).count();
    let bracket = bracket_for_target(obs, target_mhz);
    Some(F2FrontierEntry {
        target_mhz,
        best_anchor_mv: best.anchor_mv,
        offset_mhz: best.offset_mhz,
        watts: best.watts,
        max_watts: best.max_watts,
        power_p99_w: best.power_p99_w,
        avg_clock_mhz: best.avg_clock_mhz,
        sustained_clock_mhz: best.sustained_clock_mhz,
        sustained_upper_clock_mhz: best.sustained_upper_clock_mhz,
        power_capped_frac: best.power_capped_frac,
        dwell_duration_ms: best.dwell_duration_ms,
        sample_count: best.sample_count,
        max_temp_c: best.max_temp_c,
        thermal_throttled: best.thermal_throttled,
        confidence: frontier_confidence_from_evidence(&evidence_at_best),
        validation_count: qualification_count,
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

/// Build a frontier using observations from one exact physical GPU only.
pub fn learned_frontier_for_gpu(obs: &[F2Observation], gpu_key: &str) -> Vec<F2FrontierEntry> {
    let scoped: Vec<F2Observation> = obs
        .iter()
        .filter(|o| o.gpu_key.as_deref() == Some(gpu_key))
        .cloned()
        .collect();
    learned_frontier(&scoped)
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
    let max_power = entry.max_watts.unwrap_or(entry.watts.unwrap_or(0)) as f32;
    let power_p99 = entry.power_p99_w.filter(|power| power.is_finite() && *power > 0.0);
    let sustained_clock = entry.sustained_clock_mhz.unwrap_or(clock);
    let perf_per_watt = if let Some(power_p99) = power_p99 {
        sustained_clock as f64 / power_p99 as f64
    } else {
        0.0
    };
    let point = PowerSweepPoint {
        voltage_mv: entry.best_anchor_mv,
        clock_mhz: clock,
        offset_mhz: entry.offset_mhz,
        power_w: power,
        max_power_w: max_power,
        power_p99_w: power_p99,
        // In F1, a power-bound point means the voltage probe did not reveal a useful tuning
        // boundary, so synthesis excludes it. In F2 the anchored target was explicitly sustained;
        // being at the cap is valid Cmax evidence and must not disqualify the forged point.
        power_capped_frac: 0.0,
        stable: true,
        perf_per_watt,
        vf_table_voltage_mv: Some(entry.best_anchor_mv),
        boundary_voltage_mv: Some(entry.best_anchor_mv),
        apply_margin_mv: Some(0),
        p5_clock_mhz: entry.sustained_clock_mhz,
        p95_clock_mhz: entry.sustained_upper_clock_mhz,
        max_temp_c: entry.max_temp_c,
        thermal_throttled: entry.thermal_throttled,
        target_clock_mhz: Some(entry.target_mhz),
        confidence: Some(entry.confidence),
        validation_count: Some(entry.validation_count as u32),
        ..Default::default()
    };
    (point, entry.confidence)
}

/// Highest sustained-p99 discovery observation for one exact target/apply anchor. Only current v4,
/// p99-confirmed, reset-clean, thermally valid PowerRender evidence is eligible. A power-bound clock
/// drop remains valid power/clock telemetry for the apply bin, but it is never promoted into
/// stability evidence. Repeated measurements deliberately choose the largest measured p99 so a
/// profile is calibrated conservatively without inventing a monotonic correction.
pub fn current_discovery_observation_at_anchor<'a>(
    obs: &'a [F2Observation],
    target_mhz: u32,
    anchor_mv: u32,
    gpu_key: &str,
) -> Option<&'a F2Observation> {
    obs.iter()
        .filter(|o| {
            o.target_mhz == target_mhz
                && o.anchor_mv == anchor_mv
                && o.gpu_key.as_deref() == Some(gpu_key)
                && matches!(
                    o.outcome,
                    F2ObsOutcome::Validated | F2ObsOutcome::PowerBoundClockDrop
                )
                && is_current_discovery_evidence(o)
                && o.reset_to_stock_ok
                && o.boot_flag_cleared
                && !o.thermal_throttled
                && o.avg_clock_mhz.is_some()
                && o.sustained_clock_mhz.is_some()
                && o.watts.is_some_and(|watts| watts > 0)
                && o.max_watts.is_some_and(|watts| watts > 0)
                && o.power_p99_w
                    .is_some_and(|power| power.is_finite() && power > 0.0)
        })
        .max_by(|a, b| {
            a.power_p99_w
                .unwrap_or(0.0)
                .total_cmp(&b.power_p99_w.unwrap_or(0.0))
        })
}

/// Sustained-clock tolerance (MHz) mirroring the service classifier's `F2_CLOCK_DROP_TOL_MHZ`: an
/// Apply-qualification dwell whose sustained (p5) clock stayed within this margin of target still
/// exercised the hard VF point despite any thermal-slowdown flag. Kept in sync with
/// `gpu_undervolt::F2_CLOCK_DROP_TOL_MHZ` (30).
pub const F2_APPLY_CLOCK_HOLD_TOL_MHZ: u32 = 30;

/// True when an Apply-qualification observation's power/clock telemetry is trustworthy. A
/// thermal-slowdown flag only invalidates it when the slowdown actually backed the card OFF the
/// qualified point — i.e. the sustained (p5) clock sagged below target beyond tolerance. When the
/// card HELD >= target despite the flag (a momentary memory-junction hotspot at a cool core temp),
/// the point ran at its real operating clock/power, so the reading stands. Fails closed when the
/// sustained clock is unknown. Mirrors the held-clock rule in `classify_f2_stress_dwell`; power
/// discovery/calibration keep the stricter unconditional `!thermal_throttled`.
fn apply_qual_reading_trustworthy(o: &F2Observation, target_mhz: u32) -> bool {
    if !o.thermal_throttled {
        return true;
    }
    o.sustained_clock_mhz
        .is_some_and(|held| held + F2_APPLY_CLOCK_HOLD_TOL_MHZ >= target_mhz)
}

/// Highest sustained p99 measured by a complete, reset-clean v7 three-pattern set at one exact Apply
/// anchor in one run. Partial sets, failed/inconclusive passes, old qualification contracts, and
/// thermal slowdowns that sagged the sustained clock are excluded; a thermal-slowdown flag that
/// still HELD the target clock is trusted (see `apply_qual_reading_trustworthy`). The maximum across
/// all approved patterns is returned so profile presentation cannot understate power already observed
/// during its deployability soak (accepting a held-throttled reading can only raise it).
fn apply_qualification_p99_at_anchor(
    obs: &[F2Observation],
    run_id: Option<&str>,
    target_mhz: u32,
    anchor_mv: u32,
    gpu_key: &str,
) -> Option<f32> {
    let mut runs = std::collections::BTreeMap::<
        &str,
        ([bool; REQUIRED_QUALIFICATION_PATTERNS.len()], f32),
    >::new();
    for observation in obs.iter().filter(|o| {
        run_id.is_none_or(|expected| o.run_id == expected)
            && o.target_mhz == target_mhz
            && o.anchor_mv == anchor_mv
            && o.gpu_key.as_deref() == Some(gpu_key)
            && is_current_apply_qualification_pass(o)
            && o.reset_to_stock_ok
            && o.boot_flag_cleared
            && apply_qual_reading_trustworthy(o, target_mhz)
            && o.power_p99_w
                .is_some_and(|power| power.is_finite() && power > 0.0)
    }) {
        let entry = runs
            .entry(observation.run_id.as_str())
            .or_insert(([false; REQUIRED_QUALIFICATION_PATTERNS.len()], 0.0));
        let Some(index) = observation
            .qualification_coverage
            .as_ref()
            .and_then(|coverage| coverage.pattern)
            .and_then(required_pattern_index)
        else {
            continue;
        };
        entry.0[index] = true;
        let power = observation.power_p99_w.unwrap_or(0.0);
        entry.1 = entry.1.max(power);
    }
    runs.into_values()
        .filter(|(seen, _)| seen.iter().all(|present| *present))
        .map(|(_, power)| power)
        .max_by(f32::total_cmp)
}

/// Highest sustained p99 from the complete approved v7 set produced by one exact Forge run.
pub fn current_apply_qualification_p99_at_anchor(
    obs: &[F2Observation],
    run_id: &str,
    target_mhz: u32,
    anchor_mv: u32,
    gpu_key: &str,
) -> Option<f32> {
    apply_qualification_p99_at_anchor(obs, Some(run_id), target_mhz, anchor_mv, gpu_key)
}

/// Highest sustained p95 clock reached by a complete, reset-clean v7 set at one exact Apply pair.
/// Missing telemetry in any required pattern fails closed.
pub fn current_apply_qualification_p95_clock_at_anchor(
    obs: &[F2Observation],
    run_id: &str,
    target_mhz: u32,
    anchor_mv: u32,
    gpu_key: &str,
) -> Option<u32> {
    let mut seen = [false; REQUIRED_QUALIFICATION_PATTERNS.len()];
    let mut highest = 0u32;
    for observation in obs.iter().filter(|o| {
        o.run_id == run_id
            && o.target_mhz == target_mhz
            && o.anchor_mv == anchor_mv
            && o.gpu_key.as_deref() == Some(gpu_key)
            && is_current_apply_qualification_pass(o)
            && o.reset_to_stock_ok
            && o.boot_flag_cleared
            && apply_qual_reading_trustworthy(o, target_mhz)
    }) {
        let clock = observation
            .sustained_upper_clock_mhz
            .filter(|clock| *clock > 0)?;
        let Some(index) = observation
            .qualification_coverage
            .as_ref()
            .and_then(|coverage| coverage.pattern)
            .and_then(required_pattern_index)
        else {
            continue;
        };
        seen[index] = true;
        highest = highest.max(clock);
    }
    (seen.iter().all(|present| *present) && highest > 0).then_some(highest)
}

/// Highest sustained p99 across every complete current-contract v7 run for one exact Apply pair.
/// This restores the conservative published wattage when a qualified Forge snapshot is reloaded.
pub fn highest_apply_qualification_p99_at_anchor(
    obs: &[F2Observation],
    target_mhz: u32,
    anchor_mv: u32,
    gpu_key: &str,
) -> Option<f32> {
    apply_qualification_p99_at_anchor(obs, None, target_mhz, anchor_mv, gpu_key)
}

/// Failure-phase telemetry: counts of qualification dwells that failed inside a named phase,
/// keyed by `(target_mhz, anchor_mv, pattern, failure_phase)`. This is the data source for
/// evidence-driven pattern weighting and adaptive apply margins — over time it shows WHICH
/// stress phase actually predicts instability on this hardware. Pure; counts persisted
/// evidence only.
pub fn qualification_failure_histogram(
    obs: &[F2Observation],
) -> std::collections::BTreeMap<(u32, u32, String, String), u32> {
    let mut histogram = std::collections::BTreeMap::new();
    for observation in obs {
        let Some(coverage) = observation.qualification_coverage.as_ref() else { continue };
        // `failure_phase` is recorded only when a phase actually failed.
        let Some(phase) = coverage.failure_phase.clone() else { continue };
        let pattern = coverage
            .pattern
            .map(|pattern| format!("{pattern:?}"))
            .unwrap_or_else(|| "legacy".to_string());
        *histogram
            .entry((observation.target_mhz, observation.anchor_mv, pattern, phase))
            .or_insert(0u32) += 1;
    }
    histogram
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

    /// Observations for one target on one exact physical GPU.
    pub fn query_by_target_for_gpu(&self, target_mhz: u32, gpu_key: &str) -> Vec<F2Observation> {
        self.load_all()
            .into_iter()
            .filter(|o| o.target_mhz == target_mhz && o.gpu_key.as_deref() == Some(gpu_key))
            .collect()
    }

    /// The learned frontier over the entire log.
    pub fn learned_frontier(&self) -> Vec<F2FrontierEntry> {
        learned_frontier(&self.load_all())
    }

    /// Learned frontier isolated to one exact physical GPU.
    pub fn learned_frontier_for_gpu(&self, gpu_key: &str) -> Vec<F2FrontierEntry> {
        learned_frontier_for_gpu(&self.load_all(), gpu_key)
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
            evidence_kind: F2EvidenceKind::Discovery,
            discovery_contract_version: Some(F2_DISCOVERY_CONTRACT_VERSION),
            qualification_contract_version: None,
            qualification_coverage: None,
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
            sustained_upper_clock_mhz: Some(target + 15),
            watts: Some(180),
            max_watts: Some(188),
            power_p99_w: Some(186.0),
            power_p99_confirmed: true,
            power_p99_attempts: 1,
            measured_voltage_min_mv: Some(anchor),
            measured_voltage_avg_mv: Some(anchor),
            measured_voltage_max_mv: Some(anchor),
            measured_voltage_sample_count: 1,
            render_frames: Some(900),
            render_fps: Some(60.0),
            power_capped_frac: Some(0.0),
            max_temp_c: Some(68.0),
            thermal_throttled: false,
            dwell_duration_ms: Some(15_000),
            sample_count: Some(300),
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

    fn qualification_pass(o: F2Observation) -> F2Observation {
        qualification_pass_with_pattern(o, F2QualificationPattern::HighFps)
    }

    fn qualification_pass_with_pattern(
        mut o: F2Observation,
        pattern: F2QualificationPattern,
    ) -> F2Observation {
        o.evidence_kind = F2EvidenceKind::Qualification;
        o.discovery_contract_version = None;
        o.qualification_contract_version = Some(F2_QUALIFICATION_CONTRACT_VERSION);
        o.qualification_coverage = Some(F2QualificationCoverage {
            strength: F2QualificationStrength::Fsgl4,
            pattern: Some(pattern),
            pass_index: match pattern {
                F2QualificationPattern::A => 1,
                F2QualificationPattern::B => 2,
                F2QualificationPattern::HighFps => 1,
                F2QualificationPattern::Texture => 2,
                F2QualificationPattern::Transitions => 3,
                F2QualificationPattern::Memory => 4,
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
        o
    }

    fn apply_qualification_pass(
        o: F2Observation,
        pattern: F2QualificationPattern,
    ) -> F2Observation {
        let mut o = qualification_pass_with_pattern(o, pattern);
        o.evidence_kind = F2EvidenceKind::ApplyQualification;
        o.mode = F2ObsMode::ApplyQualification;
        o
    }

    fn legacy_fsgl2_qualification_pass(o: F2Observation) -> F2Observation {
        let mut o = qualification_pass(o);
        o.qualification_coverage.as_mut().unwrap().strength = F2QualificationStrength::Fsgl2;
        o
    }

    #[test]
    fn interleaved_qualification_failure_selects_shallower_qualified_point() {
        // Mirrors the interleaved Cmax descent: PowerRender validates 975→968→962, qualification
        // PASSES at 975 and 968, then FAILS at 962. The deeper failure records a bad observation, so
        // the frontier must select 968 (the deepest QUALIFIED bin), never the rejected 962, and report
        // it as qualified. This is the downstream invariant the per-bin interleaved discovery relies on.
        let t = 1935;
        let mut v = vec![
            obs(t, 975, F2ObsOutcome::Validated),
            qualification_pass(obs(t, 975, F2ObsOutcome::Validated)),
            obs(t, 968, F2ObsOutcome::Validated),
            qualification_pass(obs(t, 968, F2ObsOutcome::Validated)),
            obs(t, 962, F2ObsOutcome::Validated),
        ];
        let mut deeper_qual_fail = obs(t, 962, F2ObsOutcome::Unstable);
        deeper_qual_fail.evidence_kind = F2EvidenceKind::Qualification;
        deeper_qual_fail.discovery_contract_version = None;
        deeper_qual_fail.qualification_contract_version = Some(F2_QUALIFICATION_CONTRACT_VERSION);
        v.push(deeper_qual_fail);

        let entry = frontier_entry_for_target(&v, t).expect("a qualified frontier point exists");
        assert_eq!(entry.best_anchor_mv, 968, "deepest QUALIFIED bin, not the rejected 962");
        assert!(entry.validation_count >= 1, "selected point carries its qualification pass");
        assert_eq!(entry.first_bad_mv, Some(962), "the rejected deeper bin bounds the frontier");
    }

    #[test]
    fn outcome_classification() {
        assert!(F2ObsOutcome::Validated.is_validated());
        for o in [
            F2ObsOutcome::VerifierFailed,
            F2ObsOutcome::SilentError,
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
        assert!(!F2ObsOutcome::QualificationInconclusive.is_bad());
        // Only reset-failure and unrecovered crash are SAFETY failures.
        assert!(F2ObsOutcome::ResetFailed.is_safety_failure());
        assert!(F2ObsOutcome::DeviceLost.is_safety_failure());
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
    fn later_failure_invalidates_same_or_deeper_old_validations() {
        let v = vec![
            obs(1800, 975, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::Validated),
            obs(1800, 968, F2ObsOutcome::SilentError),
        ];
        assert_eq!(last_good_for_target(&v, 1800).unwrap().anchor_mv, 975);
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
    fn confidence_comes_from_dwell_duration_and_independent_passes() {
        let mut short = obs(1800, 975, F2ObsOutcome::Validated);
        short.dwell_duration_ms = Some(10_000);
        short.sample_count = Some(100);
        let standard = obs(1800, 975, F2ObsOutcome::Validated);
        let mut long = obs(1800, 975, F2ObsOutcome::Validated);
        long.dwell_duration_ms = Some(35_000);
        let short_conf = frontier_confidence_from_evidence(&[&short]);
        let standard_conf = frontier_confidence_from_evidence(&[&standard]);
        let long_conf = frontier_confidence_from_evidence(&[&long, &long, &long]);
        assert!(short_conf < standard_conf);
        assert!(standard_conf >= 0.85);
        assert!(long_conf > standard_conf);
        assert!(long_conf <= 0.99);
    }

    #[test]
    fn learned_frontier_is_scoped_to_exact_gpu_key() {
        let a = obs(1800, 975, F2ObsOutcome::Validated);
        let mut b = obs(1950, 1000, F2ObsOutcome::Validated);
        b.gpu_key = Some("GPU-B-UUID".into());
        let fr = learned_frontier_for_gpu(&[a, b], "RTX 3060 Ti");
        assert_eq!(
            fr.iter().map(|e| e.target_mhz).collect::<Vec<_>>(),
            vec![1800]
        );
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
        assert_eq!(e1800.validation_count, 0);
        assert!(e1800.confidence >= 0.85);
    }

    #[test]
    fn learned_frontier_counts_only_current_qualification_passes() {
        let discovery = obs(1800, 962, F2ObsOutcome::Validated);
        let mut inconclusive = qualification_pass(discovery.clone());
        inconclusive.outcome = F2ObsOutcome::QualificationInconclusive;
        inconclusive.qualification_coverage.as_mut().unwrap().verdict =
            F2QualificationVerdict::Inconclusive;
        let mut old_pass = qualification_pass(discovery.clone());
        old_pass.qualification_contract_version = Some(F2_QUALIFICATION_CONTRACT_VERSION - 1);
        let current_pass = qualification_pass(discovery.clone());

        let entry = frontier_entry_for_target(
            &[discovery, inconclusive, old_pass, current_pass],
            1800,
        )
        .unwrap();
        assert_eq!(entry.best_anchor_mv, 962);
        assert_eq!(entry.validation_count, 1);
    }

    #[test]
    fn learned_frontier_ignores_legacy_strengths_for_apply_qualification() {
        let discovery = obs(1800, 962, F2ObsOutcome::Validated);
        let mut fsgl1 = qualification_pass(discovery.clone());
        fsgl1.qualification_coverage.as_mut().unwrap().strength =
            F2QualificationStrength::Fsgl1;
        fsgl1.qualification_coverage.as_mut().unwrap().pattern = None;
        let fsgl2 = legacy_fsgl2_qualification_pass(discovery.clone());
        let current_fsgl3 = qualification_pass(discovery.clone());

        let entry = frontier_entry_for_target(&[discovery, fsgl1, fsgl2, current_fsgl3], 1800)
            .unwrap();
        assert_eq!(entry.best_anchor_mv, 962);
        assert_eq!(entry.validation_count, 1);
    }

    #[test]
    fn learned_frontier_counts_distinct_v7_patterns() {
        let discovery = obs(1800, 962, F2ObsOutcome::Validated);
        let high_fps_1 =
            qualification_pass_with_pattern(discovery.clone(), F2QualificationPattern::HighFps);
        let high_fps_2 =
            qualification_pass_with_pattern(discovery.clone(), F2QualificationPattern::HighFps);
        let only_high_fps =
            frontier_entry_for_target(&[discovery.clone(), high_fps_1, high_fps_2], 1800).unwrap();
        assert_eq!(only_high_fps.validation_count, 1);

        let high_fps =
            qualification_pass_with_pattern(discovery.clone(), F2QualificationPattern::HighFps);
        let texture =
            qualification_pass_with_pattern(discovery.clone(), F2QualificationPattern::Texture);
        let transitions =
            qualification_pass_with_pattern(discovery.clone(), F2QualificationPattern::Transitions);
        let complete =
            frontier_entry_for_target(&[discovery, high_fps, texture, transitions], 1800).unwrap();
        assert_eq!(complete.validation_count, 3);
    }

    #[test]
    fn apply_qualification_is_current_but_does_not_rewrite_frontier_failure_bracket() {
        let discovery = obs(1800, 881, F2ObsOutcome::Validated);
        let apply_pass = apply_qualification_pass(
            obs(1800, 893, F2ObsOutcome::Validated),
            F2QualificationPattern::HighFps,
        );
        assert!(is_current_apply_qualification_pass(&apply_pass));
        assert!(!is_current_qualification_pass(&apply_pass));

        let mut apply_failure = apply_qualification_pass(
            obs(1800, 893, F2ObsOutcome::SilentError),
            F2QualificationPattern::Texture,
        );
        apply_failure.qualification_coverage.as_mut().unwrap().verdict =
            F2QualificationVerdict::Fail;
        let observations = [discovery, apply_pass, apply_failure];
        assert_eq!(
            last_discovery_good_for_target(&observations, 1800)
                .unwrap()
                .anchor_mv,
            881
        );
        assert!(first_bad_for_target(&observations, 1800).is_none());
    }

    #[test]
    fn bridge_builds_classifier_compatible_points() {
        let mut capped = obs(1800, 962, F2ObsOutcome::Validated);
        capped.power_capped_frac = Some(1.0);
        let v = vec![capped];
        let fr = learned_frontier(&v);
        let (p, conf) = to_power_sweep_point(&fr[0]);
        // F2 retains the cap evidence in its frontier, but its sustained point must remain eligible
        // for synthesis: F1's "power-bound probe" exclusion has different semantics.
        assert_eq!(fr[0].power_capped_frac, Some(1.0));
        assert!(p.stable);
        assert_eq!(p.power_capped_frac, 0.0);
        assert_eq!(p.vf_table_voltage_mv, Some(962));
        assert_eq!(p.boundary_voltage_mv, Some(962));
        assert_eq!(p.apply_margin_mv, Some(0));
        assert_eq!(p.target_clock_mhz, Some(1800));
        assert_eq!(p.clock_mhz, 1815);
        assert_eq!(p.p5_clock_mhz, Some(1815));
        assert_eq!(p.p95_clock_mhz, Some(1815));
        assert_eq!(p.power_w, 180.0);
        assert_eq!(p.max_power_w, 188.0);
        assert_eq!(p.power_p99_w, Some(186.0));
        assert_eq!(p.max_temp_c, Some(68.0));
        assert_eq!(p.confidence, Some(conf));
        assert_eq!(p.validation_count, Some(0));
        assert!(p.perf_per_watt > 0.0);
        assert!(conf >= 0.85); // passes the balanced confidence gate
        // Whole-frontier bridge preserves count.
        assert_eq!(frontier_to_points(&fr).len(), 1);
    }

    #[test]
    fn discovery_v4_rejects_v3_and_unconfirmed_positive_evidence() {
        let mut old = obs(1800, 962, F2ObsOutcome::Validated);
        old.discovery_contract_version = Some(3);
        assert!(!is_current_discovery_evidence(&old));
        assert!(last_discovery_good_for_target(&[old.clone()], 1800).is_none());
        assert!(learned_frontier(&[old]).is_empty());

        let mut unconfirmed = obs(1800, 962, F2ObsOutcome::Validated);
        unconfirmed.power_p99_confirmed = false;
        assert!(!is_current_discovery_evidence(&unconfirmed));
        assert!(learned_frontier(&[unconfirmed]).is_empty());

        let mut legacy = obs(1800, 962, F2ObsOutcome::Validated);
        legacy.evidence_kind = F2EvidenceKind::Legacy;
        legacy.discovery_contract_version = None;
        assert!(!is_current_discovery_evidence(&legacy));
        assert!(last_discovery_good_for_target(&[legacy], 1800).is_none());
    }

    #[test]
    fn apply_anchor_power_uses_highest_current_thermal_safe_p99() {
        let mut lower_p99 = obs(1800, 975, F2ObsOutcome::Validated);
        lower_p99.max_watts = Some(205);
        lower_p99.power_p99_w = Some(188.0);
        let mut higher_p99 = lower_p99.clone();
        higher_p99.max_watts = Some(198);
        higher_p99.power_p99_w = Some(196.0);
        higher_p99.watts = Some(190);
        higher_p99.outcome = F2ObsOutcome::PowerBoundClockDrop;
        let mut throttled = higher_p99.clone();
        throttled.max_watts = Some(200);
        throttled.power_p99_w = Some(200.0);
        throttled.thermal_throttled = true;
        let mut old = higher_p99.clone();
        old.max_watts = Some(199);
        old.power_p99_w = Some(199.0);
        old.discovery_contract_version = Some(F2_DISCOVERY_CONTRACT_VERSION - 1);

        let observations = [lower_p99, higher_p99, throttled, old];
        let selected =
            current_discovery_observation_at_anchor(&observations, 1800, 975, "RTX 3060 Ti")
                .unwrap();
        assert_eq!(selected.watts, Some(190));
        assert_eq!(selected.max_watts, Some(198));
        assert_eq!(selected.power_p99_w, Some(196.0));
        assert!(!selected.thermal_throttled);
    }

    #[test]
    fn apply_qualification_power_requires_complete_clean_set_and_uses_highest_p99() {
        let mut high_fps = apply_qualification_pass(
            obs(1830, 862, F2ObsOutcome::Validated),
            F2QualificationPattern::HighFps,
        );
        high_fps.run_id = "apply-v7".into();
        high_fps.power_p99_w = Some(172.25);
        let mut texture = apply_qualification_pass(
            obs(1830, 862, F2ObsOutcome::Validated),
            F2QualificationPattern::Texture,
        );
        texture.run_id = "apply-v7".into();
        texture.power_p99_w = Some(172.587);
        let mut transitions = apply_qualification_pass(
            obs(1830, 862, F2ObsOutcome::Validated),
            F2QualificationPattern::Transitions,
        );
        transitions.run_id = "apply-v7".into();
        transitions.power_p99_w = Some(173.125);
        transitions.sustained_upper_clock_mhz = Some(1890);
        let mut memory = apply_qualification_pass(
            obs(1830, 862, F2ObsOutcome::Validated),
            F2QualificationPattern::Memory,
        );
        memory.run_id = "apply-v7".into();
        memory.power_p99_w = Some(172.9);
        // Thermal slowdown that ALSO sagged the sustained clock below tolerance stays excluded
        // (fail-closed). The held-clock case (throttle but clock >= target) is trusted now and is
        // covered by `apply_qualification_held_thermal_reading_is_trusted_but_sag_is_excluded`.
        let mut throttled = texture.clone();
        throttled.power_p99_w = Some(180.0);
        throttled.thermal_throttled = true;
        throttled.sustained_clock_mhz = Some(1830 - F2_APPLY_CLOCK_HOLD_TOL_MHZ - 1);
        let mut other_run_high_fps = high_fps.clone();
        other_run_high_fps.run_id = "other-run".into();
        other_run_high_fps.power_p99_w = Some(189.0);
        let mut other_run_texture = texture.clone();
        other_run_texture.run_id = "other-run".into();
        other_run_texture.power_p99_w = Some(190.0);
        let mut other_run_transitions = transitions.clone();
        other_run_transitions.run_id = "other-run".into();
        other_run_transitions.power_p99_w = Some(191.0);
        let mut other_run_memory = memory.clone();
        other_run_memory.run_id = "other-run".into();
        other_run_memory.power_p99_w = Some(190.5);

        let observations = [
            high_fps.clone(),
            texture,
            transitions,
            memory,
            throttled,
            other_run_high_fps,
            other_run_texture,
            other_run_transitions,
            other_run_memory,
        ];
        assert_eq!(
            current_apply_qualification_p99_at_anchor(
                &observations,
                "apply-v7",
                1830,
                862,
                "RTX 3060 Ti"
            ),
            Some(173.125)
        );
        assert_eq!(
            current_apply_qualification_p95_clock_at_anchor(
                &observations,
                "apply-v7",
                1830,
                862,
                "RTX 3060 Ti"
            ),
            Some(1890)
        );
        assert_eq!(
            highest_apply_qualification_p99_at_anchor(&observations, 1830, 862, "RTX 3060 Ti"),
            Some(191.0),
            "restored snapshots use the highest complete approved run"
        );
        assert_eq!(
            current_apply_qualification_p99_at_anchor(
                &[high_fps],
                "apply-v7",
                1830,
                862,
                "RTX 3060 Ti"
            ),
            None,
            "one approved pattern alone cannot publish soak power"
        );
    }

    #[test]
    fn apply_qualification_held_thermal_reading_is_trusted_but_sag_is_excluded() {
        // A complete v7 triad whose dwells thermal-throttled but HELD the target clock (sustained
        // p5 >= target - tol) is now trusted: the exact-Apply point ran at its real operating
        // clock/power despite a momentary hotspot, so p95/p99 publish rather than fail closed. This
        // is the fix for a power-bound top point (e.g. 1935 @ 200 W cap) that hotspot-throttles on
        // every 5-min soak and would otherwise leave a whole run with zero applicable profiles.
        let held = |pattern: F2QualificationPattern, p99: f32| -> F2Observation {
            let mut o = apply_qualification_pass(obs(1935, 956, F2ObsOutcome::Validated), pattern);
            o.run_id = "held-thermal".into();
            o.gpu_key = Some("RTX 4070".into());
            o.thermal_throttled = true;
            o.sustained_clock_mhz = Some(1935); // held at target despite the flag
            o.sustained_upper_clock_mhz = Some(1965);
            o.power_p99_w = Some(p99);
            o
        };
        let held_set = [
            held(F2QualificationPattern::HighFps, 199.0),
            held(F2QualificationPattern::Texture, 199.7),
            held(F2QualificationPattern::Transitions, 199.1),
            held(F2QualificationPattern::Memory, 199.3),
        ];
        assert_eq!(
            current_apply_qualification_p99_at_anchor(&held_set, "held-thermal", 1935, 956, "RTX 4070"),
            Some(199.7),
            "held-clock thermal readings publish (highest p99), never understating power"
        );
        assert_eq!(
            current_apply_qualification_p95_clock_at_anchor(&held_set, "held-thermal", 1935, 956, "RTX 4070"),
            Some(1965),
            "held-clock thermal readings publish the sustained upper clock"
        );

        // Sag the sustained clock below tolerance on one pattern → that pattern is untrustworthy,
        // the triad is incomplete, and both gates fail closed.
        let mut sagged_set = held_set.clone();
        sagged_set[1].sustained_clock_mhz = Some(1935 - F2_APPLY_CLOCK_HOLD_TOL_MHZ - 1);
        assert_eq!(
            current_apply_qualification_p99_at_anchor(&sagged_set, "held-thermal", 1935, 956, "RTX 4070"),
            None,
            "a thermal slowdown that sagged the clock still fails closed"
        );
        assert_eq!(
            current_apply_qualification_p95_clock_at_anchor(&sagged_set, "held-thermal", 1935, 956, "RTX 4070"),
            None,
            "a thermal slowdown that sagged the clock still fails closed"
        );
    }

    #[test]
    fn frontier_boundary_respects_crash_proximity_margin() {
        // Crash at V taints the adjacent bin: with a device-loss at 906 mV, a validated 912 mV
        // (one bin up) cannot become the boundary; 918 mV (~two bins) can.
        let mut crash = obs(1920, 906, F2ObsOutcome::DeviceLost);
        crash.device_lost = true;
        let near = obs(1920, 912, F2ObsOutcome::Validated);
        let clear = obs(1920, 918, F2ObsOutcome::Validated);
        assert_eq!(crash_floor_for_target(&[crash.clone()], 1920), Some(906));
        let entry =
            frontier_entry_for_target(&[crash.clone(), near.clone(), clear], 1920).unwrap();
        assert_eq!(entry.best_anchor_mv, 918);
        // Only the tainted bin available -> no boundary at all.
        assert!(frontier_entry_for_target(&[crash, near], 1920).is_none());
    }

    #[test]
    fn failure_histogram_counts_failed_phases_per_point_and_pattern() {
        let mut fail_a = apply_qualification_pass(
            obs(1935, 956, F2ObsOutcome::Unstable),
            F2QualificationPattern::HighFps,
        );
        fail_a.qualification_coverage.as_mut().unwrap().failure_phase =
            Some("frame-cadence".into());
        let mut fail_b = fail_a.clone();
        fail_b.qualification_coverage.as_mut().unwrap().failure_phase =
            Some("frame-cadence".into());
        let mut fail_other = apply_qualification_pass(
            obs(1935, 950, F2ObsOutcome::Unstable),
            F2QualificationPattern::Memory,
        );
        fail_other.qualification_coverage.as_mut().unwrap().failure_phase =
            Some("vram-pressure".into());
        // A clean pass (no failure_phase) contributes nothing.
        let clean = apply_qualification_pass(
            obs(1935, 962, F2ObsOutcome::Validated),
            F2QualificationPattern::Texture,
        );

        let histogram =
            qualification_failure_histogram(&[fail_a, fail_b, fail_other, clean]);
        assert_eq!(
            histogram
                .get(&(1935, 956, "HighFps".into(), "frame-cadence".into()))
                .copied(),
            Some(2)
        );
        assert_eq!(
            histogram
                .get(&(1935, 950, "Memory".into(), "vram-pressure".into()))
                .copied(),
            Some(1)
        );
        assert_eq!(histogram.len(), 2);
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
