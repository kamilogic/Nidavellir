//! F2 discovery/learning glue (service side).
//!
//! The PURE learning algorithm — the observation store, the per-target queries (last-good / first-bad /
//! bracket), the learned frontier, and the classifier bridge — lives in
//! [`nidavellir_core::f2_observation`]. This module is the SERVICE-side bridge between that algorithm
//! and the F2 anchored motor + CLI:
//! - maps the confirmed-run telemetry ([`F2StepReport`]) into a persisted [`F2Observation`];
//! - records a whole same-target sweep (one observation per executed candidate) and summarizes the
//!   discovered minimum-stable-voltage bracket;
//! - formats the dry-run TARGET-SWEEP plan (planned candidates, stop rules, where observations would be
//!   recorded, the current learned-frontier preview, and the explicit no-op/no-write line).
//!
//! No hardware here. Recording happens ONLY in the confirmed branch (the caller appends after a real
//! run); a dry-run never writes. None of this selects, applies, persists, or promotes a profile.

use nidavellir_core::f2_observation::{
    bracket_for_target, first_bad_for_target, frontier_confidence, frontier_entry_for_target,
    last_good_for_target, F2EvidenceKind, F2FrontierEntry, F2Observation, F2ObsDwell, F2ObsMode,
    F2ObsOutcome, F2ObsVerifier, F2ObservationStore, F2QualificationCoverage, VoltageBracket,
};
use nidavellir_gpu_nvapi::{AnchoredPositiveOffsetPlan, PositiveOffsetLimits};

use crate::gpu_undervolt::{AnchoredDescentPlan, F2DwellOutcome, F2MultiStepReport, F2Outcome, F2StepReport};
use crate::gpu_verify::PositiveOffsetVerification;

/// Immutable context shared by every observation produced in one F2 run.
#[derive(Debug, Clone)]
pub struct ObsContext {
    pub run_id: String,
    pub timestamp: String,
    pub gpu_key: Option<String>,
    pub evidence_kind: F2EvidenceKind,
    pub discovery_contract_version: Option<u32>,
    pub qualification_contract_version: Option<u32>,
    pub qualification_coverage: Option<F2QualificationCoverage>,
    pub mode: F2ObsMode,
    pub requested_start_mv: Option<u32>,
    pub positive_offset_cap_mhz: i32,
}

/// Map the crate-private verifier verdict to the owned, serializable observation form.
fn map_verifier(v: Option<PositiveOffsetVerification>) -> F2ObsVerifier {
    match v {
        Some(PositiveOffsetVerification::RaiseVerified) => F2ObsVerifier::RaiseVerified,
        Some(PositiveOffsetVerification::RaiseIncomplete) => F2ObsVerifier::RaiseIncomplete,
        Some(PositiveOffsetVerification::OverRaise) => F2ObsVerifier::OverRaise,
        Some(PositiveOffsetVerification::Unverifiable) => F2ObsVerifier::Unverifiable,
        None => F2ObsVerifier::NotRun,
    }
}

/// Map the dwell verdict to the observation form.
fn map_dwell(d: Option<F2DwellOutcome>) -> F2ObsDwell {
    match d {
        Some(F2DwellOutcome::Stable) => F2ObsDwell::Stable,
        Some(F2DwellOutcome::SilentError) => F2ObsDwell::SilentError,
        Some(F2DwellOutcome::Unstable) => F2ObsDwell::Unstable,
        Some(F2DwellOutcome::DeviceLost) => F2ObsDwell::DeviceLost,
        Some(F2DwellOutcome::ClockDrop) => F2ObsDwell::ClockDrop,
        Some(F2DwellOutcome::Inconclusive) => F2ObsDwell::QualificationInconclusive,
        None => F2ObsDwell::NotRun,
    }
}

/// Map the confirmed-step terminal outcome to the observation outcome. Arm/apply failures aborted before
/// any stable/unstable result was learned, so they are an abort — NOT an instability that would bracket
/// the voltage downward.
fn map_outcome(o: &F2Outcome) -> F2ObsOutcome {
    match o {
        F2Outcome::Validated => F2ObsOutcome::Validated,
        F2Outcome::VerifyFailed => F2ObsOutcome::VerifierFailed,
        F2Outcome::SilentError => F2ObsOutcome::SilentError,
        F2Outcome::Unstable => F2ObsOutcome::Unstable,
        F2Outcome::DeviceLost => F2ObsOutcome::DeviceLost,
        F2Outcome::PowerBoundClockDrop => F2ObsOutcome::PowerBoundClockDrop,
        F2Outcome::ClockDrop => F2ObsOutcome::ClockDrop,
        F2Outcome::Inconclusive => F2ObsOutcome::QualificationInconclusive,
        F2Outcome::ResetFailed => F2ObsOutcome::ResetFailed,
        F2Outcome::ArmFailed(_) | F2Outcome::ApplyFailed(_) => F2ObsOutcome::AbortedBySafetyGate,
    }
}

/// Build a single [`F2Observation`] from a confirmed anchored step and the candidate plan it ran. Pure.
pub fn observation_from_anchored_step(
    ctx: &ObsContext,
    target_mhz: u32,
    anchored: &AnchoredPositiveOffsetPlan,
    report: &F2StepReport,
) -> F2Observation {
    let outcome = map_outcome(&report.outcome);
    F2Observation {
        run_id: ctx.run_id.clone(),
        timestamp: ctx.timestamp.clone(),
        gpu_key: ctx.gpu_key.clone(),
        evidence_kind: ctx.evidence_kind,
        discovery_contract_version: ctx.discovery_contract_version,
        qualification_contract_version: ctx.qualification_contract_version,
        qualification_coverage: report
            .qualification_coverage
            .clone()
            .or_else(|| ctx.qualification_coverage.clone()),
        mode: ctx.mode,
        target_mhz,
        requested_start_mv: ctx.requested_start_mv,
        anchor_mv: anchored.anchor.voltage_mv,
        base_mhz: anchored.anchor.base_mhz,
        offset_mhz: anchored.anchor.offset_mhz,
        positive_offset_cap_mhz: ctx.positive_offset_cap_mhz,
        higher_bins_capped: anchored.capped_above_bins,
        max_flatten_mhz: anchored.max_negative_flatten_mhz,
        lower_bins_elastic: anchored.elastic_below_bins,
        verifier_result: map_verifier(report.verify),
        dwell_result: map_dwell(report.dwell),
        avg_clock_mhz: report.avg_clock_mhz,
        sustained_clock_mhz: report.p5_clock_mhz,
        watts: report.power_w,
        max_watts: report.max_power_w,
        power_p99_w: report.power_p99_w,
        power_capped_frac: report.power_capped_frac,
        max_temp_c: report.max_temp_c,
        thermal_throttled: report.thermal_throttled,
        dwell_duration_ms: report.dwell_duration_ms,
        sample_count: report.sample_count,
        silent_error: matches!(report.dwell, Some(F2DwellOutcome::SilentError)),
        device_lost: matches!(report.outcome, F2Outcome::DeviceLost),
        unstable: matches!(report.outcome, F2Outcome::Unstable),
        clock_drop: matches!(report.outcome, F2Outcome::ClockDrop),
        tdr_or_crash: matches!(report.outcome, F2Outcome::DeviceLost),
        reset_to_stock_attempted: report.reset_ok.is_some(),
        reset_to_stock_ok: report.reset_ok == Some(true),
        boot_flag_cleared: report.boot_flag_cleared,
        blacklisted: report.blacklisted,
        outcome,
        confidence: outcome.is_validated().then(|| frontier_confidence(1)),
        notes: None,
    }
}

/// Summary of one same-target sweep after its observations are recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepSummary {
    pub target_mhz: u32,
    pub executed: usize,
    pub recorded: usize,
    /// Lowest validated anchor voltage (the minimum stable voltage found so far for this target).
    pub last_good_mv: Option<u32>,
    /// Highest failed anchor voltage (the shallowest undervolt that already failed).
    pub first_bad_mv: Option<u32>,
    pub bracket: Option<VoltageBracket>,
    pub stop_reason: String,
    pub frontier_updated: bool,
    /// True iff the run ended reset-clean (no `ResetFailed` / crash among the executed steps).
    pub safe: bool,
}

/// Record a whole same-target sweep: map every executed candidate step → observation, APPEND each to the
/// store, then summarize last-good / first-bad / bracket from the FULL store for that target (so prior
/// runs contribute to the bracket). The descent + report come from the already-run multi-step motor;
/// this performs the persistence + derivation. Returns the summary. This is the CONFIRMED-path recorder
/// — the caller invokes it ONLY after a real run; it never runs hardware itself.
pub fn record_target_sweep(
    ctx: &ObsContext,
    target_mhz: u32,
    descent: &AnchoredDescentPlan,
    report: &F2MultiStepReport,
    store: &F2ObservationStore,
) -> SweepSummary {
    let mut recorded = 0usize;
    let mut any_safety_failure = false;
    for (i, step) in report.steps.iter().enumerate() {
        let Some(cand) = descent.candidates.get(i) else { continue };
        let obs = observation_from_anchored_step(ctx, target_mhz, cand, step);
        any_safety_failure |= obs.outcome.is_safety_failure();
        if store.append(&obs).is_ok() {
            recorded += 1;
        }
    }
    let all = store.query_by_target(target_mhz);
    let last_good_mv = last_good_for_target(&all, target_mhz).map(|o| o.anchor_mv);
    let first_bad_mv = first_bad_for_target(&all, target_mhz).map(|o| o.anchor_mv);
    SweepSummary {
        target_mhz,
        executed: report.executed,
        recorded,
        last_good_mv,
        first_bad_mv,
        bracket: bracket_for_target(&all, target_mhz),
        stop_reason: format!("{:?}", report.stop_reason),
        frontier_updated: last_good_mv.is_some(),
        safe: !any_safety_failure,
    }
}

/// Format the TARGET-SWEEP dry-run plan (pure + testable). Shows the autonomous progressive mode, the
/// target, the offset caps, the confirmed candidate cap, the planned descent candidates (safer/higher
/// voltage → lower), the exact STOP rules, where observations WOULD be recorded, the current learned
/// frontier preview (read-only), the explicit no-op/no-write line, and the no-persist line.
#[allow(clippy::too_many_arguments)]
pub fn target_sweep_plan_lines(
    target_mhz: u32,
    descent: &AnchoredDescentPlan,
    limits: &PositiveOffsetLimits,
    confirmed_cap: usize,
    obs_path: &str,
    frontier_preview: Option<&F2FrontierEntry>,
    // The deepest prior VALIDATED same-target/same-GPU resume point (anchor_mv, offset_mhz), if any.
    chained_baseline: Option<(u32, i32)>,
    preflight_safe: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push(
        "=== undervolt-probe TARGET SWEEP (F2 autonomous minimum-stable-voltage discovery, dry-run preview) ==="
            .to_string(),
    );
    out.push(
        "mode               : AUTONOMOUS PROGRESSIVE (official unknown-GPU path — anchored, physical VF frontier; NOT manual-prior)"
            .to_string(),
    );
    out.push(format!(
        "focus target       : {target_mhz} MHz (single target; discover the minimum stable anchor voltage)"
    ));
    out.push(format!(
        "offset envelope    : abs +{} MHz, step +{} MHz (derived from the physical VF clock domain; \
         no arbitrary discovery cap, target never exceeds stock clock ceiling)",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!(
        "voltage floor      : {} mV (the descent never goes below it)",
        limits.hw_floor_mv
    ));
    out.push(format!(
        "clock ceiling      : {} MHz (a planned clock above it is rejected)",
        limits.clock_ceiling_mhz
    ));
    out.push(format!(
        "confirmed span     : {confirmed_cap} physical candidate(s) in this plan; no arbitrary step cap"
    ));
    if descent.candidates.is_empty() {
        out.push(
            "candidates         : none (no bin needs a bounded positive raise to hold the target)"
                .to_string(),
        );
    } else {
        out.push(format!(
            "candidates         : {} planned, safer/higher voltage first (descend until first non-stable):",
            descent.candidates.len()
        ));
        for (i, c) in descent.candidates.iter().enumerate() {
            out.push(format!(
                "  #{:<2} anchor {:>4} mV  base {:>4} MHz  +{:>3} MHz (step Δ+{} MHz) -> {} MHz | {} capped (max -{} MHz), {} elastic",
                i + 1,
                c.anchor.voltage_mv,
                c.anchor.base_mhz,
                c.anchor.offset_mhz,
                c.anchor.step_delta_mhz,
                c.anchor.effective_mhz,
                c.capped_above_bins,
                c.max_negative_flatten_mhz,
                c.elastic_below_bins
            ));
        }
    }
    match chained_baseline {
        Some((mv, off)) => out.push(format!(
            "chained baseline   : resume from prior validated {mv} mV (+{off} MHz) for this GPU/target — \
             each candidate's per-step +{} cap applies to the offset DELTA from the last validated point \
             (not from stock); the absolute +{} cap still bounds each candidate's offset",
            limits.step_max_offset_mhz, limits.abs_max_offset_mhz
        )),
        None => out.push(
            "chained baseline   : none (no prior validated point for this GPU/target — the descent starts \
             from stock +0; per-step measured from 0)"
                .to_string(),
        ),
    }
    out.push(format!(
        "descent stop       : {}",
        descent.stop_reason.clone().unwrap_or_else(|| "descent exhausted candidate bins".to_string())
    ));
    out.push(
        "stop rules         : planner floor / step budget / VerifierFailed / Unstable / silent error / \
         DeviceLost(TDR) / ClockDrop / ResetFailed / blacklist hit / safety-gate failure"
            .to_string(),
    );
    out.push(
        "continue rule      : descend to the next (lower-voltage) candidate ONLY after Stable + confirmed reset"
            .to_string(),
    );
    out.push(format!(
        "observations       : a confirmed run records ONE observation per executed candidate to {obs_path}"
    ));
    match frontier_preview {
        Some(e) => out.push(format!(
            "learned frontier   : target {} → best {} mV (+{} MHz), first_bad {:?}, bracket {:?} mV, {} obs, conf {:.2}",
            e.target_mhz, e.best_anchor_mv, e.offset_mhz, e.first_bad_mv, e.bracket_width_mv,
            e.observation_count, e.confidence
        )),
        None => out.push(
            "learned frontier   : none yet for this target (no prior validated observation)".to_string(),
        ),
    }
    out.push(format!(
        "Safe Loop preflight: {}",
        if preflight_safe { "OK (a confirmed run would be allowed to start)" } else { "REFUSE (see above)" }
    ));
    out.push("reset_to_stock     : a confirmed run MUST reset to stock after EVERY candidate".to_string());
    out.push("no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write, no observation recorded".to_string());
    out.push("profile            : none persisted, applied, or promoted".to_string());
    out
}

/// Read-only learned-frontier preview for a target from the system store (no write). Used by the
/// dry-run. Returns `None` if the store has no validated observation for the target.
pub fn frontier_preview_for(store: &F2ObservationStore, target_mhz: u32) -> Option<F2FrontierEntry> {
    frontier_entry_for_target(&store.query_by_target(target_mhz), target_mhz)
}

// ── F2 ladder sweep: conservative priors + sequencing (pure, testable) ──────────────────────────

/// Conservative descent FLOOR (mV) for a ladder target given the previous (lower) target's discovered
/// minimum stable voltage. A HIGHER clock needs at least as much voltage as a lower clock, so the
/// higher target's descent must NOT probe BELOW where the lower target's minimum already sat — doing so
/// would be known-risky AND cannot hold the higher clock. This BOUNDS the descent conservatively; it
/// NEVER assumes the prior voltage actually holds the higher clock (the descent still validates from the
/// top down). Returns `max(base_floor_mv, prev_last_good_mv)` (or `base_floor_mv` when there is no prior).
pub fn ladder_target_floor(base_floor_mv: u32, prev_last_good_mv: Option<u32>) -> u32 {
    match prev_last_good_mv {
        Some(p) => base_floor_mv.max(p),
        None => base_floor_mv,
    }
}

/// Direction-aware descent bounds for a ladder target: `(start_mv, floor_mv)`.
///
/// The prior target's last-good is treated DIFFERENTLY depending on the ladder direction, because a
/// "minimum stable voltage" relationship is monotone in clock (a lower clock can run at or BELOW a
/// higher clock's min-V; a higher clock needs at least as much):
///
/// - **DESCENDING** (`prev_target_mhz` is `Some` AND `target_mhz < prev_target_mhz`): the prior (higher)
///   clock's last-good is a CEILING / descent START point — the lower clock can hold that voltage and
///   then go DEEPER — so we start the descent AT the prior last-good and keep the FULL base hardware
///   floor so the lower clock can reach its own deeper min-V. Using the prior as a floor here would
///   OVER-FLOOR the descent and collapse the frontier. Returns `(prior.or(global_start), base_floor)`.
/// - **ASCENDING or first target**: unchanged from today — the prior (lower) clock's last-good is a
///   conservative FLOOR (a higher clock won't probe below where a lower clock already needed voltage),
///   and the descent starts from the caller's global start. Returns
///   `(global_start, ladder_target_floor(base_floor, prior))`.
pub fn ladder_target_descent_bounds(
    base_floor_mv: u32,
    prior_last_good_mv: Option<u32>,
    target_mhz: u32,
    prev_target_mhz: Option<u32>,
    global_start_mv: Option<u32>,
) -> (Option<u32>, u32) {
    match prev_target_mhz {
        Some(prev) if target_mhz < prev => {
            // Descending: prior last-good is a START ceiling, not a floor; keep the full base floor.
            (prior_last_good_mv.or(global_start_mv), base_floor_mv)
        }
        _ => {
            // Ascending or first target: prior last-good is a conservative floor (today's behavior).
            (global_start_mv, ladder_target_floor(base_floor_mv, prior_last_good_mv))
        }
    }
}

/// Whether a ladder may continue to the NEXT target after the current target's sweep. Conservative: the
/// current target's sweep must have ended reset-CLEAN (no `ResetFailed`/crash). A normal
/// unstable/clock-drop candidate that reset cleanly does NOT stop the ladder; an unsafe end does.
pub fn ladder_should_continue(current_target_safe: bool) -> bool {
    current_target_safe
}

/// One ladder target's planned descent (dry-run view): the conservative floor in force, the prior used,
/// and the descent shape. Pure data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LadderTargetPlan {
    pub target_mhz: u32,
    pub conservative_floor_mv: u32,
    pub prior_last_good_mv: Option<u32>,
    pub candidate_count: usize,
    pub anchor_hi_mv: Option<u32>,
    pub anchor_lo_mv: Option<u32>,
    pub descent_stop: Option<String>,
}

/// Build a ladder target plan from an already-planned descent + the conservative floor + prior.
pub fn ladder_target_plan(
    target_mhz: u32,
    conservative_floor_mv: u32,
    prior_last_good_mv: Option<u32>,
    descent: &AnchoredDescentPlan,
) -> LadderTargetPlan {
    LadderTargetPlan {
        target_mhz,
        conservative_floor_mv,
        prior_last_good_mv,
        candidate_count: descent.candidates.len(),
        anchor_hi_mv: descent.candidates.first().map(|c| c.anchor.voltage_mv),
        anchor_lo_mv: descent.candidates.last().map(|c| c.anchor.voltage_mv),
        descent_stop: descent.stop_reason.clone(),
    }
}

/// Format the LADDER-SWEEP dry-run plan (pure + testable). Shows the target list, the conservative-prior
/// policy, each target's planned descent (with the floor + prior it would use), the per-target stop +
/// ladder continuation rules, where observations would be recorded, and the explicit no-op/no-write line.
pub fn ladder_plan_lines(plans: &[LadderTargetPlan], confirmed_cap: usize, obs_path: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.push(
        "=== undervolt-probe LADDER SWEEP (F2 multi-target minimum-stable-voltage discovery, dry-run preview) ==="
            .to_string(),
    );
    out.push(format!(
        "targets            : {} (each runs an autonomous progressive target sweep, in order)",
        plans.iter().map(|p| p.target_mhz.to_string()).collect::<Vec<_>>().join(", ")
    ));
    out.push(
        "conservative prior : a higher target NEVER assumes a lower target's voltage holds — it only \
         uses the lower target's last-good as a descent FLOOR (won't probe below it) and re-validates top-down"
            .to_string(),
    );
    out.push(format!(
        "confirmed span     : up to {confirmed_cap} physical candidate(s) in the shown target plans; no arbitrary step cap"
    ));
    for p in plans {
        out.push(format!("--- target {} MHz ---", p.target_mhz));
        out.push(format!(
            "  conservative floor: {} mV{}",
            p.conservative_floor_mv,
            match p.prior_last_good_mv {
                Some(prev) => format!(" (raised to the prior target's last-good {prev} mV)"),
                None => " (base floor — no prior target)".to_string(),
            }
        ));
        if p.candidate_count == 0 {
            out.push("  candidates        : none (no bin needs a bounded positive raise to hold the target)".to_string());
        } else {
            out.push(format!(
                "  candidates        : {} planned (anchors {} → {} mV, safer/higher voltage first)",
                p.candidate_count,
                p.anchor_hi_mv.map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                p.anchor_lo_mv.map(|v| v.to_string()).unwrap_or_else(|| "?".into())
            ));
        }
        out.push(format!(
            "  descent stop      : {}",
            p.descent_stop.clone().unwrap_or_else(|| "descent exhausted candidate bins".to_string())
        ));
    }
    out.push(
        "per-target stop    : planner floor / cap / VerifierFailed / Unstable / silent error / DeviceLost / ClockDrop / ResetFailed / blacklist / safety-gate"
            .to_string(),
    );
    out.push(
        "ladder stop        : STOPS the whole ladder on a SAFETY failure (ResetFailed / unrecovered crash / inconsistent boot flag); a normal bad candidate stops only THAT target and the ladder continues only if that target ended reset-clean"
            .to_string(),
    );
    out.push(format!(
        "observations       : a confirmed run records ONE observation per executed candidate (all targets) to {obs_path}"
    ));
    out.push("no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write, no observation recorded".to_string());
    out.push("profile            : none persisted, applied, or promoted".to_string());
    out
}

/// Format the LEARNED FRONTIER report (pure + testable): one line per target entry plus a header. The
/// classifier-bridge preview (Godforge/Brokkr's/Deep Calm) is appended separately by the windows caller
/// (it needs the service-private synthesizer). NEVER selects/applies/persists/promotes a profile.
pub fn frontier_report_lines(entries: &[F2FrontierEntry]) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== F2 LEARNED FRONTIER (derived from recorded observations; read-only) ===".to_string());
    if entries.is_empty() {
        out.push("(no validated observations yet — frontier is empty)".to_string());
        return out;
    }
    for e in entries {
        out.push(format!(
            "target {:>4} MHz: best {} mV (+{} MHz), sustained {:?} MHz, {:?} W, conf {:.2}, first_bad {:?} mV, bracket {:?} mV, {} obs",
            e.target_mhz,
            e.best_anchor_mv,
            e.offset_mhz,
            e.sustained_clock_mhz,
            e.watts,
            e.confidence,
            e.first_bad_mv,
            e.bracket_width_mv,
            e.observation_count
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidavellir_gpu_nvapi::PositiveOffsetPlan;

    fn ctx() -> ObsContext {
        ObsContext {
            run_id: "f2-sweep-1".into(),
            timestamp: "2026-06-21T00:00:00Z".into(),
            gpu_key: Some("RTX 3060 Ti".into()),
            evidence_kind: F2EvidenceKind::Discovery,
            discovery_contract_version: Some(
                nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
            ),
            qualification_contract_version: None,
            qualification_coverage: None,
            mode: F2ObsMode::TargetSweep,
            requested_start_mv: None,
            positive_offset_cap_mhz: 30,
        }
    }

    // Minimal anchored plan whose only fields the mapper reads are the anchor + cap/flatten/elastic counts.
    fn anchored(anchor_mv: u32, base: u32, target: u32) -> AnchoredPositiveOffsetPlan {
        let anchor = PositiveOffsetPlan {
            index: 0,
            voltage_mv: anchor_mv,
            base_mhz: base,
            offset_mhz: target as i32 - base as i32,
            prev_offset_mhz: 0,
            step_delta_mhz: target as i32 - base as i32,
            effective_mhz: target,
        };
        AnchoredPositiveOffsetPlan {
            target_mhz: target,
            anchor,
            entries: Vec::new(),
            capped_above_bins: 26,
            above_already_ok_bins: 1,
            elastic_below_bins: 40,
            max_positive_offset_mhz: anchor.offset_mhz,
            max_negative_flatten_mhz: 150,
        }
    }

    fn step(outcome: F2Outcome, verify: Option<PositiveOffsetVerification>, dwell: Option<F2DwellOutcome>,
    ) -> F2StepReport {
        F2StepReport {
            outcome,
            armed: true,
            applied: true,
            verify,
            dwell,
            avg_clock_mhz: Some(1815),
            p5_clock_mhz: Some(1815),
            power_w: Some(180),
            max_power_w: Some(188),
            power_p99_w: Some(186.0),
            power_capped_frac: Some(0.5),
            max_temp_c: Some(68.0),
            thermal_throttled: false,
            dwell_duration_ms: Some(15_000),
            sample_count: Some(300),
            qualification_coverage: None,
            reset_ok: Some(true),
            boot_flag_cleared: true,
            blacklisted: false,
            validated: false,
        }
    }

    fn validated_step() -> F2StepReport {
        let mut s = step(F2Outcome::Validated, Some(PositiveOffsetVerification::RaiseVerified), Some(F2DwellOutcome::Stable),
        );
        s.validated = true;
        s
    }

    #[test]
    fn mapper_captures_validated_point() {
        let o = observation_from_anchored_step(&ctx(), 1800, &anchored(975, 1785, 1800), &validated_step(),
        );
        assert_eq!(o.target_mhz, 1800);
        assert_eq!((o.anchor_mv, o.base_mhz, o.offset_mhz), (975, 1785, 15));
        assert_eq!(o.positive_offset_cap_mhz, 30);
        assert_eq!((o.higher_bins_capped, o.max_flatten_mhz, o.lower_bins_elastic), (26, 150, 40));
        assert_eq!(o.verifier_result, F2ObsVerifier::RaiseVerified);
        assert_eq!(o.dwell_result, F2ObsDwell::Stable);
        assert_eq!((o.avg_clock_mhz, o.sustained_clock_mhz, o.watts), (Some(1815), Some(1815), Some(180)));
        assert_eq!(o.max_watts, Some(188));
        assert_eq!(o.power_p99_w, Some(186.0));
        assert_eq!(o.outcome, F2ObsOutcome::Validated);
        assert!(o.reset_to_stock_ok && o.boot_flag_cleared);
        assert!(o.confidence.unwrap() >= 0.85);
    }

    #[test]
    fn mapper_captures_unstable_and_safety_outcomes() {
        let u = observation_from_anchored_step(
            &ctx(),
            1800,
            &anchored(956, 1785, 1800),
            &step(F2Outcome::Unstable, Some(PositiveOffsetVerification::RaiseVerified), Some(F2DwellOutcome::Unstable),
            ),
        );
        assert_eq!(u.outcome, F2ObsOutcome::Unstable);
        assert!(u.unstable && !u.silent_error);
        assert!(u.outcome.is_bad() && !u.outcome.is_safety_failure());

        let silent = observation_from_anchored_step(
            &ctx(),
            1800,
            &anchored(950, 1785, 1800),
            &step(
                F2Outcome::SilentError,
                Some(PositiveOffsetVerification::RaiseVerified),
                Some(F2DwellOutcome::SilentError),
            ),
        );
        assert_eq!(silent.outcome, F2ObsOutcome::SilentError);
        assert!(silent.silent_error && !silent.unstable);

        let lost = observation_from_anchored_step(
            &ctx(),
            1800,
            &anchored(950, 1785, 1800),
            &step(
                F2Outcome::DeviceLost,
                Some(PositiveOffsetVerification::RaiseVerified),
                Some(F2DwellOutcome::DeviceLost),
            ),
        );
        assert_eq!(lost.outcome, F2ObsOutcome::DeviceLost);
        assert!(lost.outcome.is_safety_failure());

        let r = observation_from_anchored_step(
            &ctx(),
            1800,
            &anchored(950, 1785, 1800),
            &step(F2Outcome::ResetFailed, Some(PositiveOffsetVerification::RaiseVerified), Some(F2DwellOutcome::Unstable),
            ),
        );
        assert_eq!(r.outcome, F2ObsOutcome::ResetFailed);
        assert!(r.outcome.is_safety_failure());
    }

    fn descent(anchors: &[(u32, u32)], target: u32) -> AnchoredDescentPlan {
        AnchoredDescentPlan {
            focus_target_mhz: target,
            start_mv: None,
            max_steps: anchors.len(),
            candidates: anchors.iter().map(|&(mv, base)| anchored(mv, base, target)).collect(),
            stop_reason: Some("step budget reached".into()),
            skipped_above_target: 0,
        }
    }

    fn multi_report(steps: Vec<F2StepReport>, last_good: Option<usize>, stop: crate::gpu_undervolt::F2MultiStopReason,
    ) -> F2MultiStepReport {
        F2MultiStepReport {
            planned: steps.len(),
            executed: steps.len(),
            steps,
            last_good_index: last_good,
            stop_reason: stop,
        }
    }

    #[test]
    fn record_target_sweep_persists_and_brackets() {
        use crate::gpu_undervolt::F2MultiStopReason;
        let base = std::env::temp_dir().join(format!("nidav-f2-sweep-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = F2ObservationStore::new(&base);

        // Descent 975 -> 968 -> 962, last one Unstable (descent stops). Two validated, one unstable.
        let d = descent(&[(975, 1785), (968, 1770), (962, 1770)], 1800);
        let rep = multi_report(
            vec![
                validated_step(),
                validated_step(),
                step(F2Outcome::Unstable, Some(PositiveOffsetVerification::RaiseVerified), Some(F2DwellOutcome::Unstable),
                ),
            ],
            Some(1), // last validated index
            F2MultiStopReason::Unstable,
        );
        let summary = record_target_sweep(&ctx(), 1800, &d, &rep, &store);
        assert_eq!((summary.executed, summary.recorded), (3, 3));
        assert_eq!(store.load_all().len(), 3);
        // Minimum stable voltage = lowest validated anchor (968); first failure at 962 brackets it.
        assert_eq!(summary.last_good_mv, Some(968));
        assert_eq!(summary.first_bad_mv, Some(962));
        assert_eq!(summary.bracket.map(|b| b.width_mv), Some(6));
        assert!(summary.frontier_updated);
        assert!(summary.safe); // an Unstable candidate that reset cleanly is NOT a safety failure
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn record_target_sweep_flags_unsafe_on_reset_failure() {
        use crate::gpu_undervolt::F2MultiStopReason;
        let base = std::env::temp_dir().join(format!("nidav-f2-sweep-unsafe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = F2ObservationStore::new(&base);
        let d = descent(&[(975, 1785), (968, 1770)], 1800);
        let rep = multi_report(
            vec![
                validated_step(),
                step(F2Outcome::ResetFailed, Some(PositiveOffsetVerification::RaiseVerified), Some(F2DwellOutcome::Unstable),
                ),
            ],
            Some(0),
            F2MultiStopReason::ResetFailed,
        );
        let summary = record_target_sweep(&ctx(), 1800, &d, &rep, &store);
        assert!(!summary.safe); // reset failure → safety failure
        assert_eq!(summary.last_good_mv, Some(975));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn target_sweep_plan_lines_show_rules_and_no_write() {
        let d = descent(&[(975, 1785), (968, 1770), (962, 1770)], 1800);
        let limits = PositiveOffsetLimits::conservative(612, 1950);
        let text = target_sweep_plan_lines(1800, &d, &limits, 3, "C:/ProgramData/Nidavellir/f2_observations.jsonl", None, None, true,
        ).join("\n");
        assert!(text.contains("TARGET SWEEP"));
        assert!(text.contains("AUTONOMOUS PROGRESSIVE"));
        assert!(text.contains("NOT manual-prior"));
        assert!(text.contains("confirmed span     : 3 physical candidate(s)"));
        assert!(text.contains("#1  anchor  975 mV"));
        assert!(text.contains("stop rules"));
        assert!(text.contains("observations       : a confirmed run records"));
        assert!(text.contains("f2_observations.jsonl"));
        assert!(text.contains("learned frontier   : none yet"));
        // No prior baseline → the descent starts from stock +0.
        assert!(text.contains("chained baseline   : none"));
        assert!(text.contains("from stock +0"));
        // No-op / no-write + no-persist.
        assert!(text.contains("no Safe Loop arm"));
        assert!(text.contains("no VF write"));
        assert!(text.contains("no observation recorded"));
        assert!(text.contains("none persisted, applied, or promoted"));
        // Avoid the forbidden substring the usage test guards.
        assert!(!text.contains("candidate bins"));
    }

    #[test]
    fn target_sweep_plan_lines_show_chained_baseline_when_resuming() {
        // A prior validated 975 mV / +15 point makes the descent resume from it: the per-step cap then
        // bounds each candidate's DELTA from +15, so the +30 candidates become reachable.
        let d = descent(&[(975, 1785), (968, 1770), (962, 1770)], 1800);
        let limits = PositiveOffsetLimits::conservative(612, 1950);
        let text = target_sweep_plan_lines(
            1800, &d, &limits, 3, "C:/ProgramData/Nidavellir/f2_observations.jsonl", None, Some((975, 15)), true,
        )
        .join("\n");
        assert!(text.contains("chained baseline   : resume from prior validated 975 mV (+15 MHz)"));
        assert!(text.contains("offset DELTA from the last validated point"));
        // The absolute cap is still advertised as bounding each candidate.
        assert!(text.contains("absolute +30 cap"));
        assert!(!text.contains("chained baseline   : none"));
    }

    #[test]
    fn target_sweep_plan_lines_show_physical_envelope_and_step_delta() {
        let d = descent(&[(975, 1785), (900, 1755), (850, 1740)], 1800); // +15, +45, +60
        let limits = PositiveOffsetLimits::hardware_frontier(612, 1950, 1740);
        let text = target_sweep_plan_lines(
            1800, &d, &limits, 3, "C:/ProgramData/Nidavellir/f2_observations.jsonl", None, None, true,
        )
        .join("\n");
        assert!(text.contains("physical VF frontier"));
        assert!(text.contains("physical VF clock domain"));
        assert!(text.contains("no arbitrary discovery cap"));
        // Per-candidate step delta is surfaced (chained-increment transparency).
        assert!(text.contains("step Δ+"));
        assert!(text.contains("target never exceeds stock clock ceiling"));
    }

    #[test]
    fn ladder_floor_uses_prior_as_lower_bound_only() {
        // No prior → base floor unchanged.
        assert_eq!(ladder_target_floor(612, None), 612);
        // Prior last-good ABOVE the base floor raises the floor (higher target won't probe below it).
        assert_eq!(ladder_target_floor(612, Some(962)), 962);
        // Prior BELOW the base floor never lowers the hardware floor.
        assert_eq!(ladder_target_floor(970, Some(962)), 970);
    }

    #[test]
    fn ladder_descent_bounds_are_direction_aware() {
        // DESCENDING (target < prev): prior last-good is a START ceiling; floor stays the full base.
        let (start, floor) = ladder_target_descent_bounds(612, Some(962), 1785, Some(1800), Some(975));
        assert_eq!((start, floor), (Some(962), 612));
        // Descending with no prior last-good falls back to the global start; floor still base.
        let (start, floor) = ladder_target_descent_bounds(612, None, 1785, Some(1800), Some(975));
        assert_eq!((start, floor), (Some(975), 612));

        // ASCENDING (target > prev): prior last-good is a conservative FLOOR; global start unchanged.
        let (start, floor) = ladder_target_descent_bounds(612, Some(962), 1815, Some(1800), Some(975));
        assert_eq!((start, floor), (Some(975), 962)); // max(base, prior)
        // Ascending where the prior sits below the base floor never lowers the hardware floor.
        let (start, floor) = ladder_target_descent_bounds(970, Some(962), 1815, Some(1800), Some(975));
        assert_eq!((start, floor), (Some(975), 970));

        // FIRST target (no prev): global start passes through; floor is the base floor.
        let (start, floor) = ladder_target_descent_bounds(612, None, 1800, None, Some(975));
        assert_eq!((start, floor), (Some(975), 612));
    }

    #[test]
    fn ladder_continues_only_when_prev_target_safe() {
        assert!(ladder_should_continue(true));
        assert!(!ladder_should_continue(false)); // a safety failure stops the ladder
    }

    #[test]
    fn ladder_plan_lines_show_conservative_prior_and_no_write() {
        let d1 = descent(&[(975, 1785), (968, 1770)], 1800);
        let d2 = descent(&[(982, 1797)], 1815);
        let plans = vec![
            ladder_target_plan(1800, 612, None, &d1),
            // 1815's floor was raised to 1800's last-good (968 mV): the prior is a FLOOR, not an assumption.
            ladder_target_plan(1815, 968, Some(968), &d2),
        ];
        let text = ladder_plan_lines(&plans, 3, "C:/ProgramData/Nidavellir/f2_observations.jsonl").join("\n");
        assert!(text.contains("LADDER SWEEP"));
        assert!(text.contains("targets            : 1800, 1815"));
        assert!(text.contains("NEVER assumes a lower target's voltage holds"));
        assert!(text.contains("--- target 1800 MHz ---"));
        assert!(text.contains("--- target 1815 MHz ---"));
        assert!(text.contains("raised to the prior target's last-good 968 mV"));
        assert!(text.contains("ladder stop        : STOPS the whole ladder on a SAFETY failure"));
        assert!(text.contains("no VF write"));
        assert!(text.contains("none persisted, applied, or promoted"));
    }

    #[test]
    fn frontier_report_lines_render_entries() {
        // Empty frontier.
        assert!(frontier_report_lines(&[]).join("\n").contains("frontier is empty"));
        // One entry (a validated point at 962 mV for 1800 MHz).
        let v = vec![observation_from_anchored_step(&ctx(), 1800, &anchored(962, 1785, 1800), &validated_step(),
        )];
        let fr = nidavellir_core::f2_observation::learned_frontier(&v);
        let text = frontier_report_lines(&fr).join("\n");
        assert!(text.contains("F2 LEARNED FRONTIER"));
        assert!(text.contains("target 1800 MHz: best 962 mV"));
    }
}
