//! F2 true-undervolt foundation — ISOLATED from F1/build-frontier.
//!
//! True undervolt RAISES a lower-voltage VF bin (a bounded POSITIVE offset) so a focus target clock
//! holds at a lower voltage. That is the OPPOSITE of F1/build-frontier, whose
//! `apply_vf_ceiling_monotone` flattens DOWN and deliberately refuses positive offsets, and whose
//! verifier treats any clock above target as an overshoot failure. F2 therefore lives on its own
//! path: the bounded, fail-closed positive-offset planner/writer in `nidavellir-gpu-nvapi`
//! (`plan_bounded_positive_offset` / `apply_bounded_positive_offset`) and the positive-offset-aware
//! verifier in `gpu_verify` (`verify_positive_offset`). The F1 flatten-down writer + verifier and
//! `apply_vf_ceiling_monotone` are NOT touched or relaxed.
//!
//! This module provides:
//! - a pure search skeleton ([`plan_undervolt_probe`]) that descends real voltage bins and computes
//!   the bounded positive offset each bin needs to hold the focus target, stopping at the first
//!   bound/floor violation;
//! - a pure Safe Loop preflight ([`undervolt_preflight`], dry-run display) plus the confirmed-mode
//!   fail-closed preflight ([`confirmed_f2_refusal`]);
//! - the dry-run plan formatter ([`undervolt_plan_lines`]); and
//! - the `undervolt-probe` entry ([`run_undervolt_probe`]): a read-only DRY-RUN by default, and —
//!   with `--confirm` — the real CONFIRMED hardware branch ([`run_confirmed_f2_step`] over the
//!   [`F2Ops`] trait): arm Safe Loop → apply ONE bounded positive offset → verify → dwell once →
//!   `reset_to_stock` on EVERY exit path → clear the boot flag ONLY after a confirmed reset; and
//! - a bounded same-target ANCHORED MULTI-STEP descent ([`plan_anchored_undervolt_descent`] +
//!   [`run_confirmed_f2_multi_step`] over the [`F2MultiStepOps`] trait): `--steps 2..=`[`F2_CONFIRMED_MAX_STEPS`]
//!   executes a SHORT sequence of anchored candidates at ONE target (safer/higher voltage → lower
//!   voltage), running the SAME per-step motor and STOPPING at the first non-stable candidate.
//!
//! Safety invariants: no profile apply/persist/promotion, no MULTI-TARGET automation (the multi-step
//! descent stays on a single target), no autonomous crash-seeking, no power-limit/TDP/clock-lock
//! change; the confirmed multi-step branch is bounded by [`F2_CONFIRMED_MAX_STEPS`] (a larger request
//! fails closed); the dry-run reads Safe Loop state READ-ONLY (it never arms the boot flag, mutates
//! the record, applies, dwells, or writes VF); and the confirmed branch never leaves a positive
//! offset applied after exit (a reset that cannot be confirmed fails closed and retains the boot flag
//! for startup recovery).

use std::ffi::OsString;

use nidavellir_core::safe_loop::{
    SafeLoopRecord, SafeLoopStore, TuningPoint, SAFE_MODE_CRASH_THRESHOLD,
};
use nidavellir_gpu_nvapi::{
    plan_bounded_anchored_positive_offset, plan_bounded_positive_offset, AnchoredBinRole,
    AnchoredPositiveOffsetPlan, PositiveOffsetLimits, PositiveOffsetPlan,
};

#[cfg(windows)]
use nidavellir_core::safe_loop::{BlacklistRegion, BootFlag, DEFAULT_BLACKLIST_RADIUS};
#[cfg(windows)]
use tracing::{info, warn};

#[cfg(windows)]
use crate::gpu_verify::AnchoredOffsetVerification;
use crate::gpu_verify::PositiveOffsetVerification;

/// Default number of descent steps (candidate bins) when `--steps` is omitted. Small by design —
/// F2 v1 is a single focus target with a small bounded raise.
const F2_DEFAULT_STEPS: usize = 4;

/// Maximum confirmed ANCHORED multi-step candidates per `--confirm` run. The first bounded
/// multi-step implementation stays small on purpose: a confirmed run may descend AT MOST this many
/// anchored candidates at ONE target (safer/higher voltage → lower voltage), stopping at the first
/// non-stable candidate. A larger `--steps` request FAILS CLOSED in confirmed mode; the read-only
/// dry-run may still preview a longer plan. Single-step (`--steps 1`) keeps its own validated path.
const F2_CONFIRMED_MAX_STEPS: usize = 3;

/// Tolerance (MHz) for the verifier (one boost bin) — used by the dry-run self-check and the
/// confirmed-mode post-write verify.
#[cfg(windows)]
const F2_VERIFY_TOL_MHZ: u32 = 15;

/// Sustained-clock (p5) tolerance (MHz) below the focus target before a STABLE dwell is reclassified
/// as a clock drop. If the undervolt cannot hold the target clock under load (p5 sags below
/// target − this), the descent stops — the voltage is too low to hold this clock — even without a
/// crash or silent error. ~two boost bins (mirrors the load verifier's p5 tolerance).
#[cfg(windows)]
const F2_CLOCK_DROP_TOL_MHZ: u32 = 30;

/// Residual offset (kHz) at or below which a post-reset readback counts as "cleared". F2 must NEVER
/// leave a positive offset applied; a larger residual makes the confirmed path treat the reset as
/// failed and RETAIN the boot flag (fail closed).
#[cfg(windows)]
const F2_RESET_TOL_KHZ: i32 = 1000;

// Sanity bounds for an F2 static-base point (mirror the service core-VF sanity domain). Used ONLY to
// drop foreign / memory-domain / zeroed points from the raw base read before planning; the planner
// in gpu-nvapi independently re-checks that the curve it is handed is all-sane.
#[cfg(windows)]
const F2_SANE_MV_MIN: u32 = 600;
#[cfg(windows)]
const F2_SANE_MV_MAX: u32 = 1150;
#[cfg(windows)]
const F2_SANE_MHZ_MIN: u32 = 500;
#[cfg(windows)]
const F2_SANE_MHZ_MAX: u32 = 3500;

#[cfg(windows)]
fn is_f2_sane_point(voltage_mv: u32, freq_mhz: u32) -> bool {
    (F2_SANE_MV_MIN..=F2_SANE_MV_MAX).contains(&voltage_mv)
        && (F2_SANE_MHZ_MIN..=F2_SANE_MHZ_MAX).contains(&freq_mhz)
}

/// Which F2 plan the probe builds. `Anchored` is the DEFAULT: a classic undervolt point that raises
/// the anchor bin to the target AND caps the higher-voltage plateau so the GPU cannot boost above the
/// target. `Simple` is the original single-bin positive-offset descent (proves the motor only; leaves
/// the boost curve free above the target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UndervoltMode {
    /// Classic anchored undervolt point (raise the anchor + cap the plateau). The main F2 path.
    #[default]
    Anchored,
    /// Original single-bin positive-offset descent (retained for comparison/diagnostics).
    Simple,
}

/// Parsed `undervolt-probe` flags (syntax only; missing/non-numeric values FAIL CLOSED). `--confirm`
/// is handled separately by the caller. `mode` defaults to [`UndervoltMode::Anchored`]; `--simple`
/// selects the original single-bin mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndervoltArgs {
    pub target_mhz: Option<u32>,
    pub start_mv: Option<u32>,
    pub steps: Option<usize>,
    pub mode: UndervoltMode,
}

/// Parse the `undervolt-probe` flags (`--target-mhz`, `--start-mv`, `--steps`). Pure + unit-testable;
/// a missing or non-numeric value returns `Err` (fail closed). Unknown args are ignored (the parser
/// is tolerant of `--confirm`, which the caller detects via `has_confirm_flag`).
pub fn parse_undervolt_args(args: &[OsString]) -> Result<UndervoltArgs, String> {
    let strs: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let mut out = UndervoltArgs::default();
    let mut i = 0;
    while i < strs.len() {
        match strs[i].as_str() {
            "--target-mhz" => {
                let v = strs.get(i + 1).ok_or_else(|| "--target-mhz needs a value".to_string())?;
                out.target_mhz =
                    Some(v.parse().map_err(|_| format!("--target-mhz: invalid number '{v}'"))?);
                i += 2;
            }
            "--start-mv" => {
                let v = strs.get(i + 1).ok_or_else(|| "--start-mv needs a value".to_string())?;
                out.start_mv =
                    Some(v.parse().map_err(|_| format!("--start-mv: invalid number '{v}'"))?);
                i += 2;
            }
            "--steps" => {
                let v = strs.get(i + 1).ok_or_else(|| "--steps needs a value".to_string())?;
                out.steps = Some(v.parse().map_err(|_| format!("--steps: invalid number '{v}'"))?);
                i += 2;
            }
            "--simple" => {
                out.mode = UndervoltMode::Simple;
                i += 1;
            }
            "--anchored" => {
                out.mode = UndervoltMode::Anchored;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(out)
}

/// The result of the pure F2 search: the planned (would-write-if-confirmed) bounded positive-offset
/// points, plus why the descent stopped and how many bins were skipped as already-at-target. This is
/// a PLAN only — it performs no hardware action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndervoltProbePlan {
    pub focus_target_mhz: u32,
    pub start_mv: Option<u32>,
    pub max_steps: usize,
    /// Bounded positive-offset points that WOULD be applied (one per candidate bin), in descent order.
    pub points: Vec<PositiveOffsetPlan>,
    /// Why the descent stopped (a bound/floor violation or the step budget); `None` = bins exhausted.
    pub stop_reason: Option<String>,
    /// Bins skipped because their static base already holds the target (no raise needed).
    pub skipped_above_target: u32,
}

/// Pure F2 search skeleton: descend the real voltage bins of `static_base_curve` (highest → lowest)
/// and, for each bin that still needs a raise to hold `focus_target_mhz`, plan a bounded positive
/// offset via [`plan_bounded_positive_offset`]. A bin whose static base already holds the target is
/// SKIPPED (counted, not a candidate). The descent STOPS at the first bound/floor violation (a deeper
/// bin only needs a larger offset, so it cannot pass either) or when `max_steps` candidates have been
/// planned. `start_mv`, when set, ignores bins above it. No hardware — returns a plan only.
pub fn plan_undervolt_probe(
    static_base_curve: &[(usize, u32, u32)],
    focus_target_mhz: u32,
    start_mv: Option<u32>,
    limits: &PositiveOffsetLimits,
    max_steps: usize,
) -> UndervoltProbePlan {
    let mut bins: Vec<(usize, u32, u32)> = static_base_curve.to_vec();
    bins.sort_by(|a, b| b.1.cmp(&a.1)); // descending by voltage (start near the curve, descend)

    let mut points: Vec<PositiveOffsetPlan> = Vec::new();
    let mut stop_reason: Option<String> = None;
    let mut skipped_above_target = 0u32;
    let mut prev_offset = 0i32;

    for (idx, mv, base) in bins {
        if let Some(s) = start_mv {
            if mv > s {
                continue; // above the requested descent start
            }
        }
        if points.len() >= max_steps {
            stop_reason = Some(format!("step budget reached ({max_steps} step(s))"));
            break;
        }
        if base >= focus_target_mhz {
            // Already holds the target at this voltage without a raise — not an F2 candidate.
            skipped_above_target += 1;
            continue;
        }
        match plan_bounded_positive_offset(static_base_curve, idx, focus_target_mhz, prev_offset, limits) {
            Ok(plan) => {
                prev_offset = plan.offset_mhz;
                points.push(plan);
            }
            Err(reason) => {
                // First bound/floor violation — deeper (lower-voltage) bins only need a larger
                // offset, so the descent stops cleanly here.
                stop_reason = Some(reason);
                break;
            }
        }
    }

    UndervoltProbePlan {
        focus_target_mhz,
        start_mv,
        max_steps,
        points,
        stop_reason,
        skipped_above_target,
    }
}

/// Pick the anchored-undervolt ANCHOR bin: the HIGHEST-voltage real bin whose static base is below
/// `target_mhz` (so a bounded positive offset can raise it to target), honoring `start_mv` (bins
/// above it are ignored). Choosing the highest such bin keeps the raise smallest and leaves the whole
/// plateau above it to be capped. Returns the bin index, or `None` if no bin needs a raise (every
/// considered bin already holds the target). Pure.
fn select_anchor_bin(
    static_base_curve: &[(usize, u32, u32)],
    target_mhz: u32,
    start_mv: Option<u32>,
) -> Option<usize> {
    static_base_curve
        .iter()
        .filter(|&&(_, mv, base)| start_mv.map_or(true, |s| mv <= s) && base < target_mhz)
        .max_by_key(|&&(_, mv, _)| mv)
        .map(|&(idx, _, _)| idx)
}

/// The result of planning an ANCHORED undervolt probe: the single anchored curve plan when a valid
/// anchor exists and the bounded raise + plateau caps all pass, or a `note` explaining why none was
/// produced (no candidate bin, or a fail-closed rejection). Single-target, single anchored curve.
/// This is a PLAN only — it performs no hardware action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredProbePlan {
    pub focus_target_mhz: u32,
    pub start_mv: Option<u32>,
    /// The selected anchor bin voltage (mV), if one was found.
    pub anchor_mv: Option<u32>,
    /// The validated anchored curve plan (raise the anchor + cap the plateau + elastic below).
    pub plan: Option<AnchoredPositiveOffsetPlan>,
    /// Why no plan was produced (no candidate / fail-closed rejection); `None` when `plan` is `Some`.
    pub note: Option<String>,
}

/// Pure ANCHORED-undervolt planner: select the anchor bin, then build the bounded anchored curve
/// (raise the anchor to target + cap every higher-voltage bin down to target + leave lower bins
/// elastic) via [`plan_bounded_anchored_positive_offset`]. Single-target, single anchored curve. No
/// hardware — returns a plan (or a fail-closed note) only.
pub fn plan_anchored_undervolt(
    static_base_curve: &[(usize, u32, u32)],
    focus_target_mhz: u32,
    start_mv: Option<u32>,
    limits: &PositiveOffsetLimits,
) -> AnchoredProbePlan {
    let Some(anchor_idx) = select_anchor_bin(static_base_curve, focus_target_mhz, start_mv) else {
        return AnchoredProbePlan {
            focus_target_mhz,
            start_mv,
            anchor_mv: None,
            plan: None,
            note: Some(
                "no bin below target needs a bounded positive raise (nothing to anchor)".to_string(),
            ),
        };
    };
    let anchor_mv = static_base_curve
        .iter()
        .find(|(i, _, _)| *i == anchor_idx)
        .map(|&(_, mv, _)| mv);
    // Single-step anchored: prev_offset = 0. The planner is fail-closed and never silently clamps.
    match plan_bounded_anchored_positive_offset(static_base_curve, anchor_idx, focus_target_mhz, 0, limits)
    {
        Ok(plan) => AnchoredProbePlan {
            focus_target_mhz,
            start_mv,
            anchor_mv,
            plan: Some(plan),
            note: None,
        },
        Err(reason) => AnchoredProbePlan {
            focus_target_mhz,
            start_mv,
            anchor_mv,
            plan: None,
            note: Some(reason),
        },
    }
}

/// The result of planning a same-target ANCHORED multi-step DESCENT: a bounded sequence of anchored
/// candidates that hold the SAME `focus_target_mhz` at progressively LOWER voltage (safer/higher
/// voltage first), plus why the descent stopped. Each candidate is a complete anchored curve (anchor
/// raise + plateau cap + elastic below). This is a PLAN only — it performs no hardware action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredDescentPlan {
    pub focus_target_mhz: u32,
    pub start_mv: Option<u32>,
    pub max_steps: usize,
    /// Anchored candidates in descent order (highest anchor voltage first → lowest last).
    pub candidates: Vec<AnchoredPositiveOffsetPlan>,
    /// Why the descent stopped (a bound/floor/per-step violation or the step budget); `None` = bins
    /// exhausted (no more candidate bin below target).
    pub stop_reason: Option<String>,
    /// Bins skipped because their static base already holds the target (no anchor raise needed).
    pub skipped_above_target: u32,
}

/// Pure same-target ANCHORED multi-step planner: descend the real voltage bins (highest → lowest) and,
/// for each bin whose static base is still below `focus_target_mhz`, plan a complete anchored curve via
/// [`plan_bounded_anchored_positive_offset`] (anchor that bin to target + cap every higher-voltage bin
/// down to target + leave lower bins elastic). The anchor offset CHAINS through `prev_offset` so the
/// per-step cap bounds how fast the descent deepens between consecutive candidates (same rule the
/// single-bin descent uses). A bin already at/above target is SKIPPED (counted). The descent STOPS at
/// the first anchored-plan rejection (a deeper bin only needs a larger raise, so it cannot pass either)
/// or when `max_steps` candidates have been planned. Same target only; no hardware — returns a plan.
pub fn plan_anchored_undervolt_descent(
    static_base_curve: &[(usize, u32, u32)],
    focus_target_mhz: u32,
    start_mv: Option<u32>,
    limits: &PositiveOffsetLimits,
    max_steps: usize,
) -> AnchoredDescentPlan {
    let mut bins: Vec<(usize, u32, u32)> = static_base_curve.to_vec();
    bins.sort_by(|a, b| b.1.cmp(&a.1)); // descending by voltage (safer/higher voltage first)

    let mut candidates: Vec<AnchoredPositiveOffsetPlan> = Vec::new();
    let mut stop_reason: Option<String> = None;
    let mut skipped_above_target = 0u32;
    let mut prev_offset = 0i32;

    for (idx, mv, base) in bins {
        if let Some(s) = start_mv {
            if mv > s {
                continue; // above the requested descent start
            }
        }
        if candidates.len() >= max_steps {
            stop_reason = Some(format!("step budget reached ({max_steps} step(s))"));
            break;
        }
        if base >= focus_target_mhz {
            // Already holds the target at this voltage without a raise — not an anchor candidate.
            skipped_above_target += 1;
            continue;
        }
        match plan_bounded_anchored_positive_offset(
            static_base_curve,
            idx,
            focus_target_mhz,
            prev_offset,
            limits,
        ) {
            Ok(plan) => {
                prev_offset = plan.anchor.offset_mhz;
                candidates.push(plan);
            }
            Err(reason) => {
                // First bound/floor/per-step violation — deeper (lower-voltage) bins only need a
                // larger raise, so the descent stops cleanly here.
                stop_reason = Some(reason);
                break;
            }
        }
    }

    AnchoredDescentPlan {
        focus_target_mhz,
        start_mv,
        max_steps,
        candidates,
        stop_reason,
        skipped_above_target,
    }
}

/// Read-only Safe Loop preflight verdict for a candidate F2 run. Pure; never mutates Safe Loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightVerdict {
    /// True only when none of the refusal conditions hold.
    pub safe: bool,
    /// Human-readable refusal reasons (empty when `safe`).
    pub reasons: Vec<String>,
    pub safe_mode: bool,
    pub consecutive_crashes: u32,
    pub boot_flag_armed: bool,
    pub blacklisted_points: usize,
}

/// Pure Safe Loop preflight: REFUSE (read-only) when Safe Mode is active, a boot flag is already
/// armed (a prior run did not clear — recovery not complete), or any planned point falls inside a
/// blacklisted region. Returns the verdict; mutates nothing. This is the DRY-RUN display preflight;
/// the confirmed path enforces its own fail-closed gate in [`confirmed_f2_refusal`] before any write.
pub fn undervolt_preflight(
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
    points: &[TuningPoint],
) -> PreflightVerdict {
    let mut reasons = Vec::new();
    if record.safe_mode {
        reasons.push("Safe Mode is active — refuse".to_string());
    }
    if boot_flag_armed {
        reasons.push("a Safe Loop boot flag is already armed (prior run did not clear) — refuse".to_string());
    }
    let blacklisted = points.iter().filter(|p| record.is_blacklisted(p)).count();
    if blacklisted > 0 {
        reasons.push(format!("{blacklisted} planned point(s) fall in a blacklisted region — refuse"));
    }
    PreflightVerdict {
        safe: reasons.is_empty(),
        reasons,
        safe_mode: record.safe_mode,
        consecutive_crashes: record.consecutive_crashes,
        boot_flag_armed,
        blacklisted_points: blacklisted,
    }
}

/// Format the full `undervolt-probe` dry-run plan as printable lines (pure + testable). Includes the
/// focus target, candidate bins + planned positive offsets, the offset caps, the voltage floor and
/// clock ceiling, the Safe Loop preflight, the blacklist + reset_to_stock requirements for a future
/// confirmed run, and the explicit no-op (no-write) line.
pub fn undervolt_plan_lines(
    plan: &UndervoltProbePlan,
    limits: &PositiveOffsetLimits,
    preflight: &PreflightVerdict,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== undervolt-probe PLAN (F2 true-undervolt, dry-run preview) ===".to_string());
    out.push(format!(
        "focus target       : {} MHz (single target; F2 holds an existing clock — it never overclocks)",
        plan.focus_target_mhz
    ));
    out.push(format!(
        "offset caps        : abs +{} MHz, per-step +{} MHz (constants — NOT CLI-widenable)",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!(
        "voltage floor      : {} mV (lowest real core VF bin; the descent never goes below it)",
        limits.hw_floor_mv
    ));
    out.push(format!(
        "clock ceiling      : {} MHz (stock boost top; a planned clock above it is rejected)",
        limits.clock_ceiling_mhz
    ));
    out.push(format!(
        "start voltage      : {}",
        plan.start_mv
            .map_or("curve top (highest candidate bin)".to_string(), |s| format!("{s} mV (descend from here)"))
    ));
    out.push(format!("step budget        : {} step(s)", plan.max_steps));
    out.push(format!(
        "bins skipped       : {} bin(s) already at/above target (no raise needed)",
        plan.skipped_above_target
    ));
    if plan.points.is_empty() {
        out.push("candidate bins     : none (no bin needs a bounded positive raise to hold the target)".to_string());
    } else {
        out.push(format!("candidate bins     : {} planned positive-offset point(s):", plan.points.len()));
        for p in &plan.points {
            out.push(format!(
                "  bin {:>4} mV  base {:>4} MHz  +{:>2} MHz (step +{})  -> {} MHz",
                p.voltage_mv, p.base_mhz, p.offset_mhz, p.step_delta_mhz, p.effective_mhz
            ));
        }
    }
    out.push(format!(
        "descent stop       : {}",
        plan.stop_reason.clone().unwrap_or_else(|| "descent exhausted candidate bins".to_string())
    ));
    out.push(format!(
        "Safe Loop preflight: safe_mode={} consecutive_crashes={} boot_flag_armed={} blacklisted_points={}",
        preflight.safe_mode, preflight.consecutive_crashes, preflight.boot_flag_armed,
        preflight.blacklisted_points
    ));
    out.push(format!(
        "preflight verdict  : {}",
        if preflight.safe {
            "OK (a future confirmed run would be allowed to start)".to_string()
        } else {
            format!("REFUSE — {}", preflight.reasons.join("; "))
        }
    ));
    out.push(
        "blacklist check    : a confirmed run must re-check each point against the Safe Loop blacklist (read-only above)"
            .to_string(),
    );
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock on every exit path".to_string());
    out.push("no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write".to_string());
    out
}

/// Format the ANCHORED `undervolt-probe` dry-run plan as printable lines (pure + testable). Shows the
/// target, the selected anchor bin (mV / base MHz / positive offset), the higher-voltage bins that
/// will be capped to target, the max positive offset and max downward flatten, the voltage floor and
/// clock ceiling, the Safe Loop preflight, the reset_to_stock requirement, the anchored guarantee
/// (boost above target is prevented), and the explicit no-op (no-write) line.
pub fn anchored_plan_lines(
    probe: &AnchoredProbePlan,
    limits: &PositiveOffsetLimits,
    preflight: &PreflightVerdict,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== undervolt-probe PLAN (F2 anchored true-undervolt, dry-run preview) ===".to_string());
    out.push(
        "mode               : ANCHORED (classic undervolt point — raise the anchor + cap the plateau)"
            .to_string(),
    );
    out.push(format!(
        "focus target       : {} MHz (single target; F2 holds an existing clock — it never overclocks)",
        probe.focus_target_mhz
    ));
    out.push(format!(
        "offset caps        : abs +{} MHz, per-step +{} MHz (constants — NOT CLI-widenable)",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!(
        "voltage floor      : {} mV (lowest real core VF bin; the anchor never goes below it)",
        limits.hw_floor_mv
    ));
    out.push(format!(
        "clock ceiling      : {} MHz (a planned clock above it is rejected)",
        limits.clock_ceiling_mhz
    ));
    out.push(format!(
        "start voltage      : {}",
        probe.start_mv.map_or("curve top (highest candidate bin)".to_string(), |s| format!(
            "{s} mV (anchor at/below here)"
        ))
    ));
    match &probe.plan {
        None => {
            out.push(format!(
                "anchored plan      : none — {}",
                probe.note.clone().unwrap_or_else(|| "no plan".to_string())
            ));
        }
        Some(p) => {
            out.push(format!(
                "anchor bin         : {} mV  base {} MHz  +{} MHz (step +{}) -> {} MHz (raised to target)",
                p.anchor.voltage_mv,
                p.anchor.base_mhz,
                p.anchor.offset_mhz,
                p.anchor.step_delta_mhz,
                p.anchor.effective_mhz
            ));
            out.push(format!(
                "max positive offset: +{} MHz (at the anchor bin only)",
                p.max_positive_offset_mhz
            ));
            out.push(format!(
                "max neg flatten    : -{} MHz (largest downward cap on a higher-voltage bin)",
                p.max_negative_flatten_mhz
            ));
            out.push(format!(
                "higher-voltage bins: {} capped DOWN to target, {} already at/below target (never raised):",
                p.capped_above_bins, p.above_already_ok_bins
            ));
            let mut caps: Vec<&_> = p
                .entries
                .iter()
                .filter(|e| e.role == AnchoredBinRole::CappedAbove)
                .collect();
            caps.sort_by(|a, b| a.voltage_mv.cmp(&b.voltage_mv));
            for e in caps {
                out.push(format!(
                    "  cap  {:>4} mV  base {:>4} MHz  {:>+4} MHz offset -> {} MHz",
                    e.voltage_mv, e.base_mhz, e.offset_mhz, e.effective_mhz
                ));
            }
            out.push(format!(
                "lower-voltage bins : {} left elastic (offset 0, never raised)",
                p.elastic_below_bins
            ));
        }
    }
    out.push(format!(
        "Safe Loop preflight: safe_mode={} consecutive_crashes={} boot_flag_armed={} blacklisted_points={}",
        preflight.safe_mode, preflight.consecutive_crashes, preflight.boot_flag_armed,
        preflight.blacklisted_points
    ));
    out.push(format!(
        "preflight verdict  : {}",
        if preflight.safe {
            "OK (a future confirmed run would be allowed to start)".to_string()
        } else {
            format!("REFUSE — {}", preflight.reasons.join("; "))
        }
    ));
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock on every exit path".to_string());
    out.push(
        "anchored guarantee : anchored mode prevents boost above the target during this probe"
            .to_string(),
    );
    out.push("no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write".to_string());
    out
}

/// Format the same-target ANCHORED multi-step DESCENT dry-run plan as printable lines (pure +
/// testable). Shows the target, the confirmed step cap, the planned candidates (per-candidate anchor
/// mV / base / positive offset / plateau caps / max flatten), the descent stop reason, the Safe Loop
/// preflight, the reset_to_stock requirement, the exact confirmed-mode STOP semantics, and the
/// explicit no-op (no-write) line. `confirmed_cap` is the confirmed-branch ceiling on executed steps.
pub fn anchored_descent_plan_lines(
    descent: &AnchoredDescentPlan,
    limits: &PositiveOffsetLimits,
    preflight: &PreflightVerdict,
    confirmed_cap: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push(
        "=== undervolt-probe PLAN (F2 anchored multi-step descent, dry-run preview) ===".to_string(),
    );
    out.push(
        "mode               : ANCHORED (same-target descent — raise the anchor + cap the plateau, then descend voltage)"
            .to_string(),
    );
    out.push(format!(
        "focus target       : {} MHz (single target; same-target descent never overclocks)",
        descent.focus_target_mhz
    ));
    out.push(format!(
        "confirmed cap      : up to {confirmed_cap} anchored candidate(s) per --confirm run \
         (F2_CONFIRMED_MAX_STEPS={confirmed_cap}; more FAILS CLOSED in confirmed mode)"
    ));
    out.push(format!(
        "offset caps        : abs +{} MHz, per-step +{} MHz (constants — NOT CLI-widenable)",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!(
        "voltage floor      : {} mV (lowest real core VF bin; the descent never goes below it)",
        limits.hw_floor_mv
    ));
    out.push(format!(
        "clock ceiling      : {} MHz (a planned clock above it is rejected)",
        limits.clock_ceiling_mhz
    ));
    out.push(format!(
        "start voltage      : {}",
        descent.start_mv.map_or("curve top (highest candidate bin)".to_string(), |s| format!(
            "{s} mV (anchor at/below here)"
        ))
    ));
    out.push(format!("step budget        : {} candidate(s) requested", descent.max_steps));
    out.push(format!(
        "bins skipped       : {} bin(s) already at/above target (no anchor raise needed)",
        descent.skipped_above_target
    ));
    if descent.candidates.is_empty() {
        out.push(
            "candidates         : none (no bin needs a bounded positive raise to hold the target)"
                .to_string(),
        );
    } else {
        out.push(format!(
            "candidates         : {} planned anchored candidate(s), safer/higher voltage first:",
            descent.candidates.len()
        ));
        for (i, c) in descent.candidates.iter().enumerate() {
            out.push(format!(
                "  #{:<2} anchor {:>4} mV  base {:>4} MHz  +{:>2} MHz (step +{}) -> {} MHz | \
                 {} bin(s) capped DOWN (max -{} MHz), {} elastic",
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
    out.push(format!(
        "descent stop       : {}",
        descent
            .stop_reason
            .clone()
            .unwrap_or_else(|| "descent exhausted candidate bins".to_string())
    ));
    out.push(format!(
        "Safe Loop preflight: safe_mode={} consecutive_crashes={} boot_flag_armed={} blacklisted_points={}",
        preflight.safe_mode, preflight.consecutive_crashes, preflight.boot_flag_armed,
        preflight.blacklisted_points
    ));
    out.push(format!(
        "preflight verdict  : {}",
        if preflight.safe {
            "OK (a future confirmed run would be allowed to start)".to_string()
        } else {
            format!("REFUSE — {}", preflight.reasons.join("; "))
        }
    ));
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock after EVERY candidate".to_string());
    out.push(
        "anchored guarantee : anchored mode prevents boost above the target during each candidate"
            .to_string(),
    );
    out.push(
        "confirmed stop     : STOPS at the first VerifierFailed / Unstable / DeviceLost / ClockDrop / \
         ResetFailed / Blacklisted; otherwise CompletedAllPlanned"
            .to_string(),
    );
    out.push(
        "no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write"
            .to_string(),
    );
    out
}

/// Help/usage text for `undervolt-probe`. Pure — printing it reads no hardware, plans nothing, and
/// mutates nothing. Includes an explicit `--confirm` hardware warning.
pub fn undervolt_usage() -> String {
    [
        "Usage: nidavellir-service undervolt-probe [OPTIONS]",
        "",
        "F2 true-undervolt probe. The default ANCHORED mode plans a classic undervolt point: it raises",
        "the chosen (lower-voltage) anchor bin to the focus clock AND caps every higher-voltage bin to",
        "the same clock so the GPU cannot boost above the target. Dry-run by default; with --confirm,",
        "anchored mode executes one or more supervised anchored candidates at the SAME target,",
        "descending voltage and STOPPING at the first non-stable candidate.",
        "",
        "Options:",
        "  --target-mhz <MHz>   Focus target clock (default: stock boost top; never overclocks).",
        "  --start-mv <mV>      Highest voltage bin to anchor at (default: curve top).",
        "  --steps <N>          Confirmed ANCHORED candidates to attempt at the same target, descending",
        "                       voltage (1..=3; cap F2_CONFIRMED_MAX_STEPS=3; larger FAILS CLOSED in",
        "                       confirmed mode). --steps 1 keeps the validated single-step path. The",
        "                       read-only dry-run may preview a longer plan. --simple uses N as the",
        "                       descent budget (default: 4) and REQUIRES --steps 1 under --confirm.",
        "  --anchored           Plan the anchored classic undervolt point(s) (DEFAULT).",
        "  --simple             Plan the original single-bin positive-offset descent (boost above the",
        "                       target is NOT prevented; for comparison/diagnostics only).",
        "  --confirm            Execute supervised anchored candidate(s) (see WARNING). Default: dry-run.",
        "  -h, --help           Print this help and exit (no hardware read, no plan, no mutation).",
        "",
        "First hardware optimization should start small, e.g. `--steps 3`, with an operator present.",
        "",
        "WARNING: --confirm MAY WRITE bounded positive VF offsets and run load dwells. It can TDR or",
        "         reboot the machine and REQUIRES an operator present and able to reboot. Anchored",
        "         confirmed mode runs at most F2_CONFIRMED_MAX_STEPS (3) candidates at ONE target,",
        "         arms Safe Loop before each write, resets to stock after EVERY candidate, stops at the",
        "         first non-stable candidate, and never persists, applies, or promotes a profile.",
    ]
    .join("\n")
}

// ── F2 confirmed single-step: fail-closed state machine (trait-isolated, unit-testable) ────────
// The confirmed branch performs the only real hardware sequence: arm Safe Loop → apply ONE bounded
// positive offset → verify the write → dwell once → reset_to_stock on EVERY exit path → clear the
// boot flag ONLY after a clean reset. It is abstracted behind [`F2Ops`] so the orchestration +
// cleanup invariants are tested with a mock (no hardware). v1 is single-target, single-step: no
// multi-step descent, no autonomous crash-seeking, no profile persistence/apply/promotion.

/// Simplified outcome of the single confirmed dwell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F2DwellOutcome {
    /// Dwell completed cleanly and held the target clock.
    Stable,
    /// Instability / silent error under load (no device loss).
    Unstable,
    /// TDR / crash / device-lost during the dwell.
    DeviceLost,
    /// Dwell did not crash or error, but the sustained (p5) clock sagged below target − tolerance —
    /// the undervolt could not hold the focus clock under load (voltage too low for this clock).
    ClockDrop,
}

/// One confirmed dwell's outcome plus its headline measurements. The measurements are carried so the
/// multi-step report can show avg/p5 clock + watts per candidate and so the real dwell can classify a
/// [`F2DwellOutcome::ClockDrop`] from p5. Not used in any `Eq` comparison (carries an `f32`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F2DwellResult {
    pub outcome: F2DwellOutcome,
    pub avg_clock_mhz: u32,
    pub p5_clock_mhz: u32,
    pub power_w: f32,
}

/// Terminal classification of a confirmed single step. The most-severe applicable variant wins
/// (e.g. a failed reset dominates as `ResetFailed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum F2Outcome {
    /// Arming the boot flag failed — no write happened.
    ArmFailed(String),
    /// The positive-offset write failed.
    ApplyFailed(String),
    /// The post-write verify did not confirm the intended raise.
    VerifyFailed,
    /// The dwell reported instability (no device loss).
    Unstable,
    /// The dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// The dwell stayed up but the sustained (p5) clock sagged below target − tolerance.
    ClockDrop,
    /// `reset_to_stock` could not be confirmed — boot flag RETAINED, fail closed.
    ResetFailed,
    /// Dwell stable, reset confirmed, boot flag cleared.
    Validated,
}

/// Structured report of a confirmed single step (also drives the printed output and the tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct F2StepReport {
    pub outcome: F2Outcome,
    pub armed: bool,
    pub applied: bool,
    pub verify: Option<PositiveOffsetVerification>,
    pub dwell: Option<F2DwellOutcome>,
    /// Headline dwell measurements (when a dwell ran): average clock, sustained (p5) clock, watts
    /// (rounded). `None` when no dwell was reached (arm/apply/verify failed before the dwell).
    pub avg_clock_mhz: Option<u32>,
    pub p5_clock_mhz: Option<u32>,
    pub power_w: Option<u32>,
    /// `Some(true)` reset confirmed, `Some(false)` reset failed/unconfirmed, `None` not reached.
    pub reset_ok: Option<bool>,
    pub boot_flag_cleared: bool,
    pub blacklisted: bool,
    pub validated: bool,
}

/// The hardware / Safe-Loop operations a confirmed F2 single step performs. Abstracted so the
/// orchestrator [`run_confirmed_f2_step`] is unit-testable with a mock (no hardware). The real impl
/// is windows-only.
pub trait F2Ops {
    /// Arm the Safe Loop boot flag with the F2 intent BEFORE any VF write.
    fn arm_boot_flag(&mut self) -> Result<(), String>;
    /// Apply the single bounded positive offset (F2 writer only).
    fn apply_positive_offset(&mut self) -> Result<(), String>;
    /// Read back and verify the positive offset took (offset-presence; idle freq is unreliable).
    fn verify(&mut self) -> PositiveOffsetVerification;
    /// Dwell / measure once under load (outcome + headline measurements).
    fn dwell(&mut self) -> F2DwellResult;
    /// Reset the GPU to stock. `Ok` ONLY when the reset is confirmed (no residual offset).
    fn reset_to_stock(&mut self) -> Result<(), String>;
    /// Clear the Safe Loop boot flag (call ONLY after a confirmed clean reset / success).
    fn clear_boot_flag(&mut self) -> Result<(), String>;
    /// Record the crash/instability point in the Safe Loop blacklist (crash knowledge).
    fn blacklist_point(&mut self) -> Result<(), String>;
}

/// Drive the confirmed single step. Preflight refusal is handled by the caller via
/// [`confirmed_f2_refusal`]; this assumes a vetted candidate and executes the fail-closed sequence.
/// Returns a full report. NEVER persists/applies/promotes a profile (no such op exists).
pub fn run_confirmed_f2_step<O: F2Ops>(ops: &mut O) -> F2StepReport {
    let mut r = F2StepReport {
        outcome: F2Outcome::Validated, // overwritten before return
        armed: false,
        applied: false,
        verify: None,
        dwell: None,
        avg_clock_mhz: None,
        p5_clock_mhz: None,
        power_w: None,
        reset_ok: None,
        boot_flag_cleared: false,
        blacklisted: false,
        validated: false,
    };

    // 1. ARM the boot flag BEFORE any write. If it fails, nothing was written; best-effort reset
    //    (idempotent), leave the (un-armed) flag alone.
    if let Err(e) = ops.arm_boot_flag() {
        r.reset_ok = Some(ops.reset_to_stock().is_ok());
        r.outcome = F2Outcome::ArmFailed(e);
        return r;
    }
    r.armed = true;

    // 2. APPLY the single bounded positive offset.
    if let Err(e) = ops.apply_positive_offset() {
        r.outcome = F2Outcome::ApplyFailed(e);
        return finish_after_write(ops, r, false);
    }
    r.applied = true;

    // 3. VERIFY the write took.
    let v = ops.verify();
    r.verify = Some(v);
    if v != PositiveOffsetVerification::RaiseVerified {
        r.outcome = F2Outcome::VerifyFailed;
        return finish_after_write(ops, r, false);
    }

    // 4. DWELL once under load.
    let d = ops.dwell();
    r.dwell = Some(d.outcome);
    r.avg_clock_mhz = Some(d.avg_clock_mhz);
    r.p5_clock_mhz = Some(d.p5_clock_mhz);
    r.power_w = Some(d.power_w.round() as u32);
    match d.outcome {
        F2DwellOutcome::DeviceLost => {
            // Crash / TDR: best-effort reset, record the blacklist, and RETAIN the boot flag — a
            // reboot may be imminent, so startup recovery must still fire. Never validate.
            r.reset_ok = Some(ops.reset_to_stock().is_ok());
            r.blacklisted = ops.blacklist_point().is_ok();
            r.outcome = F2Outcome::DeviceLost;
            r
        }
        F2DwellOutcome::Unstable => {
            r.outcome = F2Outcome::Unstable;
            finish_after_write(ops, r, true)
        }
        F2DwellOutcome::ClockDrop => {
            // Stable but the sustained clock sagged below tolerance — not a crash/instability to
            // blacklist; reset, clear on a confirmed reset, never validate, and stop the descent.
            r.outcome = F2Outcome::ClockDrop;
            finish_after_write(ops, r, false)
        }
        F2DwellOutcome::Stable => {
            r.outcome = F2Outcome::Validated;
            finish_after_write(ops, r, false)
        }
    }
}

/// Shared cleanup after a write (apply-fail / verify-fail / unstable / validated): reset to stock,
/// optionally blacklist, and clear the boot flag ONLY when the reset is confirmed. A failed reset
/// RETAINS the flag and dominates the outcome as `ResetFailed` (fail closed).
fn finish_after_write<O: F2Ops>(ops: &mut O, mut r: F2StepReport, blacklist: bool) -> F2StepReport {
    let reset = ops.reset_to_stock();
    r.reset_ok = Some(reset.is_ok());
    if blacklist {
        r.blacklisted = ops.blacklist_point().is_ok();
    }
    if reset.is_ok() {
        // Reset confirmed → GPU recovered, no offset left applied → safe to clear the boot flag.
        r.boot_flag_cleared = ops.clear_boot_flag().is_ok();
        if r.outcome == F2Outcome::Validated {
            r.validated = true;
        }
    } else {
        // Reset NOT confirmed → retain the flag; the most severe outcome dominates.
        r.outcome = F2Outcome::ResetFailed;
    }
    r
}

// ── F2 confirmed ANCHORED multi-step descent: bounded same-target orchestration ─────────────────
// The single-step state machine above proves ONE anchored point. The bounded multi-step descent
// executes a SHORT sequence of anchored candidates at the SAME target, from safer/higher voltage to
// lower voltage, running the SAME validated per-candidate motor ([`run_confirmed_f2_step`]) for each.
// It STOPS at the first non-stable candidate and never attempts a deeper (lower-voltage) candidate
// after any non-stable result. Single-target, anchored-only, capped at [`F2_CONFIRMED_MAX_STEPS`];
// no multi-target automation, no autonomous crash-seeking, no profile persistence/apply/promotion.
// Trait-isolated ([`F2MultiStepOps`]) so the orchestration + stop semantics are unit-tested with a
// mock (no hardware).

/// Why a confirmed anchored multi-step descent stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F2MultiStopReason {
    /// Every planned (capped) candidate validated stably — descent ran to the end of the plan.
    CompletedAllPlanned,
    /// A candidate's post-write verify did not confirm the anchored raise.
    VerifierFailed,
    /// A candidate's dwell reported instability (no device loss).
    Unstable,
    /// A candidate's dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// A candidate held but its sustained (p5) clock sagged below target − tolerance.
    ClockDrop,
    /// A candidate was refused by the per-candidate Safe Loop / blacklist precheck before its write.
    Blacklisted,
    /// A candidate's `reset_to_stock` could not be confirmed (boot flag retained — fail closed).
    ResetFailed,
    /// Arming the boot flag failed for a candidate (no write happened).
    ArmFailed,
    /// A candidate's bounded positive-offset write failed.
    ApplyFailed,
    /// No candidate was available to execute (empty plan / budget exhausted).
    NoMoreCandidates,
}

/// Structured report of a confirmed anchored multi-step descent (drives the printed output + tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct F2MultiStepReport {
    /// Candidates the orchestrator was willing to attempt (planned count capped to the step cap).
    pub planned: usize,
    /// Candidates actually executed (a step ran the per-candidate motor).
    pub executed: usize,
    /// Per-candidate reports in execution order (index `i` pairs with planned candidate `i`).
    pub steps: Vec<F2StepReport>,
    /// Index (into the candidates list) of the LAST stably-validated candidate, if any.
    pub last_good_index: Option<usize>,
    pub stop_reason: F2MultiStopReason,
}

/// The per-candidate operations a confirmed anchored multi-step descent performs. Extends [`F2Ops`]
/// (the validated single-candidate motor) with a candidate cursor: [`select`](Self::select) makes
/// candidate `i` current AND runs the per-candidate Safe Loop / blacklist precheck BEFORE its write
/// (returning `Err` to refuse it). The orchestrator then drives the SAME [`run_confirmed_f2_step`]
/// motor on the now-current candidate. Abstracted so [`run_confirmed_f2_multi_step`] is unit-testable
/// with a mock (no hardware). The real impl is windows-only.
pub trait F2MultiStepOps: F2Ops {
    /// Number of planned candidates available (descent order; safer/higher voltage first).
    fn candidate_count(&self) -> usize;
    /// Make candidate `i` current and re-check Safe Loop + blacklist immediately before its write.
    /// `Err(reason)` REFUSES this candidate (the descent stops, `Blacklisted`); `Ok` proceeds.
    fn select(&mut self, i: usize) -> Result<(), String>;
}

/// Drive the confirmed anchored multi-step descent. Runs at most `min(candidate_count, cap)`
/// candidates in order; for each: precheck via [`F2MultiStepOps::select`] (refusal → stop
/// `Blacklisted`), then the SAME validated [`run_confirmed_f2_step`] motor. CONTINUES only on a stable
/// `Validated` candidate (dwell stable + reset confirmed + boot flag cleared); STOPS immediately on any
/// other outcome and NEVER attempts a deeper candidate after a non-stable result. Returns a full
/// report. NEVER persists/applies/promotes a profile (no such op exists on the trait).
pub fn run_confirmed_f2_multi_step<O: F2MultiStepOps>(
    ops: &mut O,
    cap: usize,
) -> F2MultiStepReport {
    let planned = ops.candidate_count().min(cap);
    let mut steps: Vec<F2StepReport> = Vec::new();
    let mut last_good_index: Option<usize> = None;
    let mut stop_reason = if planned == 0 {
        F2MultiStopReason::NoMoreCandidates
    } else {
        F2MultiStopReason::CompletedAllPlanned
    };

    for i in 0..planned {
        // Per-candidate Safe Loop + blacklist precheck BEFORE arming / writing anything.
        if ops.select(i).is_err() {
            stop_reason = F2MultiStopReason::Blacklisted;
            break;
        }
        // The SAME validated single-candidate motor: arm → apply → verify → dwell → reset → clear.
        let report = run_confirmed_f2_step(ops);
        let outcome = report.outcome.clone();
        steps.push(report);
        match outcome {
            F2Outcome::Validated => {
                last_good_index = Some(i);
                // Continue to the next (lower-voltage) candidate only after a confirmed clean reset.
            }
            F2Outcome::VerifyFailed => {
                stop_reason = F2MultiStopReason::VerifierFailed;
                break;
            }
            F2Outcome::Unstable => {
                stop_reason = F2MultiStopReason::Unstable;
                break;
            }
            F2Outcome::DeviceLost => {
                stop_reason = F2MultiStopReason::DeviceLost;
                break;
            }
            F2Outcome::ClockDrop => {
                stop_reason = F2MultiStopReason::ClockDrop;
                break;
            }
            F2Outcome::ResetFailed => {
                stop_reason = F2MultiStopReason::ResetFailed;
                break;
            }
            F2Outcome::ArmFailed(_) => {
                stop_reason = F2MultiStopReason::ArmFailed;
                break;
            }
            F2Outcome::ApplyFailed(_) => {
                stop_reason = F2MultiStopReason::ApplyFailed;
                break;
            }
        }
    }

    F2MultiStepReport {
        planned,
        executed: steps.len(),
        steps,
        last_good_index,
        stop_reason,
    }
}

/// Canonical F2 intent for a candidate (target clock, target voltage bin, positive offset). Used to
/// arm the boot flag and to blacklist the point on a crash.
fn f2_intent(target_mhz: u32, cand: &PositiveOffsetPlan) -> TuningPoint {
    TuningPoint::from_axes([
        ("gpu_freq_mhz", target_mhz as i64),
        ("gpu_vf_bin_mv", cand.voltage_mv as i64),
        ("gpu_offset_mhz", cand.offset_mhz as i64),
    ])
}

/// Conservative blacklist match: refuse if the 3-axis F2 intent OR the 2-axis (freq, vf_bin) point
/// (matching build-frontier's region keys) falls in a blacklisted region.
fn candidate_blacklisted(record: &SafeLoopRecord, target_mhz: u32, cand: &PositiveOffsetPlan) -> bool {
    let f2 = f2_intent(target_mhz, cand);
    let f1_like = TuningPoint::from_axes([
        ("gpu_freq_mhz", target_mhz as i64),
        ("gpu_vf_bin_mv", cand.voltage_mv as i64),
    ]);
    record.is_blacklisted(&f2) || record.is_blacklisted(&f1_like)
}

/// Pure confirmed-mode preflight. Returns `Some(reason)` to REFUSE (fail closed) before any
/// hardware, or `None` to proceed. Refuses unless: `--steps 1` (single-step only); not in Safe Mode;
/// no boot flag already armed; `consecutive_crashes` below the abort threshold; a candidate exists;
/// the candidate is within all offset/floor/clock bounds (defensive re-check); the candidate intent
/// is not blacklisted.
pub fn confirmed_f2_refusal(
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
    steps: Option<usize>,
    candidate: Option<&PositiveOffsetPlan>,
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
) -> Option<String> {
    if steps != Some(1) {
        return Some(format!(
            "confirmed F2 is single-step only — pass --steps 1 (got --steps {steps:?})"
        ));
    }
    if record.safe_mode {
        return Some("Safe Mode is active".to_string());
    }
    if boot_flag_armed {
        return Some("a Safe Loop boot flag is already armed (prior run did not clear)".to_string());
    }
    if record.consecutive_crashes >= SAFE_MODE_CRASH_THRESHOLD {
        return Some(format!(
            "consecutive_crashes {} >= abort threshold {}",
            record.consecutive_crashes, SAFE_MODE_CRASH_THRESHOLD
        ));
    }
    let Some(cand) = candidate else {
        return Some("no valid candidate to confirm".to_string());
    };
    // Defensive re-validation against the bounds (the planner already enforced these).
    if cand.offset_mhz <= 0 {
        return Some(format!("candidate offset {} <= 0 (not a positive raise)", cand.offset_mhz));
    }
    if cand.offset_mhz > limits.abs_max_offset_mhz {
        return Some(format!(
            "candidate offset +{} exceeds the absolute cap +{}",
            cand.offset_mhz, limits.abs_max_offset_mhz
        ));
    }
    if cand.step_delta_mhz > limits.step_max_offset_mhz {
        return Some(format!(
            "candidate per-step +{} exceeds the per-step cap +{}",
            cand.step_delta_mhz, limits.step_max_offset_mhz
        ));
    }
    if cand.voltage_mv < limits.hw_floor_mv {
        return Some(format!(
            "candidate bin {} mV below the hardware floor {} mV",
            cand.voltage_mv, limits.hw_floor_mv
        ));
    }
    if cand.effective_mhz > limits.clock_ceiling_mhz {
        return Some(format!(
            "candidate clock {} MHz exceeds the clock ceiling {} MHz",
            cand.effective_mhz, limits.clock_ceiling_mhz
        ));
    }
    if candidate_blacklisted(record, target_mhz, cand) {
        return Some("the candidate intent is blacklisted".to_string());
    }
    None
}

/// Pure confirmed ANCHORED multi-step preflight (run-level gate). Returns `Some(reason)` to REFUSE
/// (fail closed) before any hardware, or `None` to proceed. Refuses unless: `--steps` is present and
/// within `1..=cap` (a larger request fails closed — the confirmed branch enforces its OWN cap
/// regardless of what the dry-run may preview); not in Safe Mode; no boot flag already armed;
/// `consecutive_crashes` below the abort threshold; and at least one anchored candidate exists. The
/// per-candidate bounds/blacklist re-checks happen at execution time (the writer re-validates bounds
/// and [`F2MultiStepOps::select`] re-checks Safe Loop + blacklist before each write).
pub fn confirmed_f2_multi_refusal(
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
    steps: Option<usize>,
    candidate_count: usize,
    cap: usize,
) -> Option<String> {
    let Some(n) = steps else {
        return Some(format!(
            "confirmed anchored multi-step requires --steps between 1 and {cap} (got none)"
        ));
    };
    if n < 1 {
        return Some(format!("confirmed anchored multi-step requires --steps >= 1 (got --steps {n})"));
    }
    if n > cap {
        return Some(format!(
            "confirmed anchored multi-step is capped at --steps {cap} (got --steps {n}) — fail closed"
        ));
    }
    if record.safe_mode {
        return Some("Safe Mode is active".to_string());
    }
    if boot_flag_armed {
        return Some("a Safe Loop boot flag is already armed (prior run did not clear)".to_string());
    }
    if record.consecutive_crashes >= SAFE_MODE_CRASH_THRESHOLD {
        return Some(format!(
            "consecutive_crashes {} >= abort threshold {}",
            record.consecutive_crashes, SAFE_MODE_CRASH_THRESHOLD
        ));
    }
    if candidate_count == 0 {
        return Some("no anchored candidates to confirm".to_string());
    }
    None
}

/// Format the confirmed single-step report as printable lines (pure + testable).
pub fn confirmed_report_lines(
    target_mhz: u32,
    candidate: &PositiveOffsetPlan,
    limits: &PositiveOffsetLimits,
    report: &F2StepReport,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== undervolt-probe CONFIRMED (F2 single-step) ===".to_string());
    out.push(format!(
        "candidate          : {} MHz @ {} mV  (+{} MHz offset -> {} MHz)",
        target_mhz, candidate.voltage_mv, candidate.offset_mhz, candidate.effective_mhz
    ));
    out.push(format!(
        "offset caps        : abs +{} MHz, per-step +{} MHz",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!("Safe Loop armed    : {}", report.armed));
    out.push(format!("offset applied     : {}", report.applied));
    out.push(format!("verifier result    : {:?}", report.verify));
    out.push(format!("dwell result       : {:?}", report.dwell));
    out.push(format!(
        "reset_to_stock     : {}",
        match report.reset_ok {
            Some(true) => "OK (confirmed cleared)",
            Some(false) => "FAILED/UNCONFIRMED",
            None => "not reached",
        }
    ));
    out.push(format!(
        "boot flag          : {}",
        if report.boot_flag_cleared {
            "cleared (clean reset)"
        } else {
            "RETAINED (startup recovery will handle it)"
        }
    ));
    out.push(format!("validated          : {}", report.validated));
    out.push(format!("blacklisted        : {}", report.blacklisted));
    out.push("profile            : none persisted, applied, or promoted".to_string());
    out.push(format!("outcome            : {:?}", report.outcome));
    out
}

/// Format the confirmed ANCHORED multi-step descent report as printable lines (pure + testable).
/// Shows the total planned/executed candidates, a per-candidate block (anchor mV/base/offset, plateau
/// caps, max flatten, verifier, dwell + avg/p5/watts, reset, boot flag, blacklisted, validated), the
/// final last-good candidate (if any), the stop reason, and the explicit no-persist/apply/promote
/// line. `candidates[i]` pairs with `report.steps[i]`.
pub fn confirmed_multi_report_lines(
    target_mhz: u32,
    candidates: &[AnchoredPositiveOffsetPlan],
    limits: &PositiveOffsetLimits,
    report: &F2MultiStepReport,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== undervolt-probe CONFIRMED (F2 anchored multi-step descent) ===".to_string());
    out.push(format!(
        "target             : {target_mhz} MHz (single target — same-target descent, never overclocks)"
    ));
    out.push(format!(
        "offset caps        : abs +{} MHz, per-step +{} MHz",
        limits.abs_max_offset_mhz, limits.step_max_offset_mhz
    ));
    out.push(format!("planned candidates : {}", report.planned));
    out.push(format!("executed candidates: {}", report.executed));
    for (i, step) in report.steps.iter().enumerate() {
        let cand = candidates.get(i);
        out.push(format!("--- candidate #{} ---", i + 1));
        match cand {
            Some(c) => {
                out.push(format!(
                    "  target           : {} MHz",
                    target_mhz
                ));
                out.push(format!(
                    "  anchor           : {} mV  base {} MHz  +{} MHz (step +{}) -> {} MHz",
                    c.anchor.voltage_mv,
                    c.anchor.base_mhz,
                    c.anchor.offset_mhz,
                    c.anchor.step_delta_mhz,
                    c.anchor.effective_mhz
                ));
                out.push(format!(
                    "  higher capped    : {} bin(s) (max flatten -{} MHz)",
                    c.capped_above_bins, c.max_negative_flatten_mhz
                ));
            }
            None => out.push("  (candidate plan unavailable)".to_string()),
        }
        out.push(format!("  verifier         : {:?}", step.verify));
        out.push(format!(
            "  dwell            : {:?} (avg {} MHz, p5 {} MHz, {} W)",
            step.dwell,
            step.avg_clock_mhz.map_or("n/a".to_string(), |v| v.to_string()),
            step.p5_clock_mhz.map_or("n/a".to_string(), |v| v.to_string()),
            step.power_w.map_or("n/a".to_string(), |v| v.to_string())
        ));
        out.push(format!(
            "  reset_to_stock   : {}",
            match step.reset_ok {
                Some(true) => "OK (confirmed cleared)",
                Some(false) => "FAILED/UNCONFIRMED",
                None => "not reached",
            }
        ));
        out.push(format!(
            "  boot flag        : {}",
            if step.boot_flag_cleared {
                "cleared (clean reset)"
            } else {
                "RETAINED (startup recovery will handle it)"
            }
        ));
        out.push(format!("  blacklisted      : {}", step.blacklisted));
        out.push(format!("  validated        : {}", step.validated));
        out.push(format!("  outcome          : {:?}", step.outcome));
    }
    out.push(format!(
        "last good candidate: {}",
        match report.last_good_index.and_then(|i| candidates.get(i).map(|c| (i, c))) {
            Some((i, c)) => format!(
                "#{} — {} MHz @ {} mV (+{} MHz)",
                i + 1,
                target_mhz,
                c.anchor.voltage_mv,
                c.anchor.offset_mhz
            ),
            None => "none (no candidate validated stably)".to_string(),
        }
    ));
    out.push(format!("stop reason        : {:?}", report.stop_reason));
    out.push("profile            : none persisted, applied, or promoted".to_string());
    out
}

/// Windows real executor for the confirmed F2 single step. Wires [`F2Ops`] to NVAPI (the bounded
/// positive-offset / anchored writer + offset readback), the validated load dwell + reset
/// (`gpu_power_sweep`), and the Safe Loop store. In `Anchored` mode it writes the full anchored curve
/// (`anchored` is `Some`) and verifies it with the anchored verifier; in `Simple` mode it writes/verifies
/// the single anchor bin. NOT invoked unless the operator passes `--confirm`.
#[cfg(windows)]
struct RealF2Ops<'a> {
    store: &'a SafeLoopStore,
    curve: Vec<(usize, u32, u32)>,
    /// The anchor bin (the raised bin) — both modes use this for arm/blacklist/single-bin verify.
    candidate: PositiveOffsetPlan,
    /// The full anchored curve plan, `Some` in [`UndervoltMode::Anchored`] (drives apply/verify/reset).
    anchored: Option<AnchoredPositiveOffsetPlan>,
    mode: UndervoltMode,
    limits: PositiveOffsetLimits,
    target_mhz: u32,
}

#[cfg(windows)]
impl F2Ops for RealF2Ops<'_> {
    fn arm_boot_flag(&mut self) -> Result<(), String> {
        let intent = f2_intent(self.target_mhz, &self.candidate);
        self.store
            .arm_boot_flag(&BootFlag::new(intent, "f2_undervolt_probe"))
            .map_err(|e| format!("arm_boot_flag: {e}"))
    }

    fn apply_positive_offset(&mut self) -> Result<(), String> {
        // Single-step → prev_offset = 0. Each writer re-validates every bound and fails closed.
        match (self.mode, &self.anchored) {
            (UndervoltMode::Anchored, Some(_)) => {
                // Anchored: write the full curve (anchor raise + plateau caps + elastic zeros). The
                // writer refuses any positive offset outside the anchor.
                nidavellir_gpu_nvapi::apply_bounded_anchored_positive_offset(
                    &self.curve,
                    self.candidate.index,
                    self.target_mhz,
                    0,
                    &self.limits,
                )
                .map(|_| ())
            }
            _ => nidavellir_gpu_nvapi::apply_bounded_positive_offset(
                &self.curve,
                self.candidate.index,
                self.target_mhz,
                0,
                &self.limits,
            )
            .map(|_| ()),
        }
    }

    fn verify(&mut self) -> PositiveOffsetVerification {
        // Offset readback is primary (idle GetStatus freq under-reports → pass freq = None).
        match (self.mode, &self.anchored) {
            (UndervoltMode::Anchored, Some(plan)) => {
                // Anchored: read back EVERY bin's offset and run the anchored verifier (anchor raised +
                // higher-voltage plateau capped + no stray positive offset). Map onto the state
                // machine's single-bin gate, logging the detailed anchored verdict.
                let observed: Vec<(usize, Option<i32>)> = plan
                    .entries
                    .iter()
                    .map(|e| {
                        (e.index, nidavellir_gpu_nvapi::vf_get_point_khz(e.index).map(|khz| khz / 1000))
                    })
                    .collect();
                let av = crate::gpu_verify::verify_anchored_positive_offset(
                    plan,
                    &observed,
                    F2_VERIFY_TOL_MHZ,
                );
                info!("undervolt-probe anchored verify: {av:?}");
                match av {
                    AnchoredOffsetVerification::AnchoredRaiseVerified => {
                        PositiveOffsetVerification::RaiseVerified
                    }
                    AnchoredOffsetVerification::AnchorRaiseIncomplete => {
                        PositiveOffsetVerification::RaiseIncomplete
                    }
                    AnchoredOffsetVerification::AnchorOverRaise
                    | AnchoredOffsetVerification::HigherBinAboveTarget
                    | AnchoredOffsetVerification::UnexpectedPositiveOffset => {
                        PositiveOffsetVerification::OverRaise
                    }
                    AnchoredOffsetVerification::Unverifiable => {
                        PositiveOffsetVerification::Unverifiable
                    }
                }
            }
            _ => {
                let observed =
                    nidavellir_gpu_nvapi::vf_get_point_khz(self.candidate.index).map(|khz| khz / 1000);
                crate::gpu_verify::verify_positive_offset(
                    self.candidate.offset_mhz,
                    self.candidate.effective_mhz,
                    observed,
                    None,
                    F2_VERIFY_TOL_MHZ,
                )
            }
        }
    }

    fn dwell(&mut self) -> F2DwellResult {
        let s = crate::gpu_power_sweep::single_load_dwell();
        let outcome = if s.crashed {
            F2DwellOutcome::DeviceLost
        } else if !s.stable {
            // silent_error or any non-stable/non-crash → conservatively unstable.
            F2DwellOutcome::Unstable
        } else if s.p5_clock_mhz + F2_CLOCK_DROP_TOL_MHZ < self.target_mhz {
            // Stable (no crash/error) but the sustained (p5) clock sagged below target − tol → the
            // undervolt did not hold the focus clock under load. Stop the descent (voltage too low).
            F2DwellOutcome::ClockDrop
        } else {
            F2DwellOutcome::Stable
        };
        info!(
            "undervolt-probe dwell: {outcome:?} avg_clock={} MHz p5={} MHz power={:.0} W silent_error={}",
            s.avg_clock_mhz, s.p5_clock_mhz, s.power_w, s.silent_error
        );
        F2DwellResult {
            outcome,
            avg_clock_mhz: s.avg_clock_mhz,
            p5_clock_mhz: s.p5_clock_mhz,
            power_w: s.power_w,
        }
    }

    fn reset_to_stock(&mut self) -> Result<(), String> {
        crate::gpu_power_sweep::reset_to_stock();
        // F2 must NEVER leave any offset applied: confirm EVERY bin the writer touched reads ~0 before
        // reporting success. In anchored mode that is every bin in the plan (anchor + caps + elastic);
        // in simple mode it is just the candidate bin. An unreadable or non-zero readback fails closed
        // (flag retained).
        let indices: Vec<usize> = match (self.mode, &self.anchored) {
            (UndervoltMode::Anchored, Some(plan)) => plan.entries.iter().map(|e| e.index).collect(),
            _ => vec![self.candidate.index],
        };
        for idx in indices {
            match nidavellir_gpu_nvapi::vf_get_point_khz(idx) {
                Some(khz) if khz.abs() <= F2_RESET_TOL_KHZ => {}
                Some(khz) => return Err(format!("reset readback offset {khz} kHz not cleared at idx {idx}")),
                None => {
                    return Err(format!("reset readback unavailable at idx {idx} — cannot confirm cleared"))
                }
            }
        }
        Ok(())
    }

    fn clear_boot_flag(&mut self) -> Result<(), String> {
        self.store.clear_boot_flag().map_err(|e| format!("clear_boot_flag: {e}"))
    }

    fn blacklist_point(&mut self) -> Result<(), String> {
        let mut rec = self.store.load_record();
        rec.blacklist.push(BlacklistRegion::around(
            f2_intent(self.target_mhz, &self.candidate),
            DEFAULT_BLACKLIST_RADIUS,
        ));
        rec.consecutive_crashes = rec.consecutive_crashes.saturating_add(1);
        self.store.save_record(&rec).map_err(|e| format!("save_record: {e}"))
    }
}

/// Windows real executor for the confirmed ANCHORED multi-step descent. Holds the descent's anchored
/// candidates (safer/higher voltage first) and a cursor; [`select`](F2MultiStepOps::select) builds a
/// per-candidate [`RealF2Ops`] (the SAME validated single-candidate motor glue) AFTER re-checking Safe
/// Loop state + the blacklist for that candidate's intent. Every [`F2Ops`] call delegates to the
/// current candidate's `RealF2Ops`. Anchored-only, single-target. NOT invoked unless the operator
/// passes `--confirm` with `--steps` ≥ 2 (single-step keeps the [`RealF2Ops`] path).
#[cfg(windows)]
struct RealF2MultiOps<'a> {
    store: &'a SafeLoopStore,
    curve: Vec<(usize, u32, u32)>,
    candidates: Vec<AnchoredPositiveOffsetPlan>,
    limits: PositiveOffsetLimits,
    target_mhz: u32,
    /// The per-candidate motor for the currently-selected candidate (`None` until first `select`).
    cur: Option<RealF2Ops<'a>>,
}

#[cfg(windows)]
impl F2Ops for RealF2MultiOps<'_> {
    fn arm_boot_flag(&mut self) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").arm_boot_flag()
    }
    fn apply_positive_offset(&mut self) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").apply_positive_offset()
    }
    fn verify(&mut self) -> PositiveOffsetVerification {
        self.cur.as_mut().expect("select before use").verify()
    }
    fn dwell(&mut self) -> F2DwellResult {
        self.cur.as_mut().expect("select before use").dwell()
    }
    fn reset_to_stock(&mut self) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").reset_to_stock()
    }
    fn clear_boot_flag(&mut self) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").clear_boot_flag()
    }
    fn blacklist_point(&mut self) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").blacklist_point()
    }
}

#[cfg(windows)]
impl F2MultiStepOps for RealF2MultiOps<'_> {
    fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    fn select(&mut self, i: usize) -> Result<(), String> {
        let plan = self
            .candidates
            .get(i)
            .ok_or_else(|| format!("no anchored candidate at index {i}"))?
            .clone();
        // Per-candidate Safe Loop + blacklist precheck IMMEDIATELY before this candidate's write.
        let rec = self.store.load_record();
        if rec.safe_mode {
            return Err("Safe Mode active before candidate write".to_string());
        }
        if self.store.is_boot_flag_armed() {
            return Err("a Safe Loop boot flag is already armed before candidate write".to_string());
        }
        if candidate_blacklisted(&rec, self.target_mhz, &plan.anchor) {
            return Err(format!("candidate {i} intent is blacklisted"));
        }
        let anchor = plan.anchor;
        self.cur = Some(RealF2Ops {
            store: self.store,
            curve: self.curve.clone(),
            candidate: anchor,
            anchored: Some(plan),
            mode: UndervoltMode::Anchored,
            limits: self.limits,
            target_mhz: self.target_mhz,
        });
        Ok(())
    }
}

/// Supervised console entry for the F2 `undervolt-probe`. WITHOUT `--confirm` it is a read-only
/// DRY-RUN: it reads the static VF base curve read-only, plans the bounded positive offsets, runs the
/// Safe Loop preflight read-only, and prints the plan. WITH `--confirm` it FAILS CLOSED — confirmed
/// hardware F2 is not implemented in this patch, and nothing is armed/applied/dwelled/written.
#[cfg(windows)]
pub fn run_undervolt_probe(store: &SafeLoopStore, confirm: bool, args: UndervoltArgs) {
    use nidavellir_gpu_nvapi as gpu;

    // Read-only: static VF-table base curve, sanity-filtered to plausible core points.
    let sane: Vec<(usize, u32, u32)> = gpu::read_vf_base_curve_modern()
        .into_iter()
        .filter(|&(_, mv, f)| is_f2_sane_point(mv, f))
        .collect();
    if sane.is_empty() {
        println!("undervolt-probe: no sane static VF base points available — fail closed (no hardware touched).");
        warn!("undervolt-probe: static VF base unavailable/non-sane — aborting before any plan");
        return;
    }
    let floor_mv = sane.iter().map(|&(_, mv, _)| mv).min().unwrap();
    let boost_top = sane.iter().map(|&(_, _, f)| f).max().unwrap();
    let focus_target = args.target_mhz.unwrap_or(boost_top);
    let max_steps = args.steps.unwrap_or(F2_DEFAULT_STEPS);
    // Clock ceiling = stock boost top: F2 may hold an existing clock at lower voltage but never
    // overclock above stock. The offset caps are the conservative constants (not CLI-widenable).
    let limits = PositiveOffsetLimits::conservative(floor_mv, boost_top);

    // Read-only Safe Loop state (shared by both modes; the dry-run NEVER mutates it).
    let record = store.load_record();
    let boot_flag_armed = store.is_boot_flag_armed();

    // ANCHORED is the default F2 path — a classic undervolt point that ALSO prevents boost above the
    // target. `--simple` falls back to the original single-bin positive-offset descent below.
    if args.mode == UndervoltMode::Anchored {
        run_anchored_undervolt_probe(
            store,
            confirm,
            &args,
            &sane,
            &limits,
            focus_target,
            &record,
            boot_flag_armed,
        );
        return;
    }

    let plan = plan_undervolt_probe(&sane, focus_target, args.start_mv, &limits, max_steps);

    // Read-only Safe Loop preflight over the planned points (axis keys match build-frontier's intent).
    let points: Vec<TuningPoint> = plan
        .points
        .iter()
        .map(|p| {
            TuningPoint::from_axes([
                ("gpu_freq_mhz", plan.focus_target_mhz as i64),
                ("gpu_vf_bin_mv", p.voltage_mv as i64),
            ])
        })
        .collect();
    let preflight = undervolt_preflight(&record, boot_flag_armed, &points);

    for line in undervolt_plan_lines(&plan, &limits, &preflight) {
        println!("{line}");
    }

    // Plan/verifier self-consistency: every planned point must verify as an intended raise using the
    // SAME verifier the future confirmed path will use — this catches planner/verifier drift without
    // any hardware. It does NOT verify hardware (there was no write).
    let all_self_verify = plan.points.iter().all(|p| {
        crate::gpu_verify::verify_positive_offset(
            p.offset_mhz,
            p.effective_mhz,
            Some(p.offset_mhz),
            Some(p.effective_mhz),
            F2_VERIFY_TOL_MHZ,
        ) == crate::gpu_verify::PositiveOffsetVerification::RaiseVerified
    });
    println!(
        "plan self-check    : {} planned point(s) verify as RaiseVerified (tol {} MHz): {}",
        plan.points.len(),
        F2_VERIFY_TOL_MHZ,
        all_self_verify
    );

    if confirm {
        // Confirmed F2 single step. The candidate is the FIRST planned point; confirmed mode requires
        // --steps 1 and re-runs the full fail-closed preflight before touching anything.
        let candidate = plan.points.first().copied();
        match confirmed_f2_refusal(
            &record,
            boot_flag_armed,
            args.steps,
            candidate.as_ref(),
            &limits,
            focus_target,
        ) {
            Some(reason) => {
                println!(
                    "undervolt-probe: --confirm REFUSED — {reason}. No Safe Loop arm, no apply, no \
                     dwell, no VF write performed."
                );
                warn!("undervolt-probe: --confirm refused: {reason} — no hardware touched");
            }
            None => {
                let cand = candidate.expect("refusal None guarantees a candidate");
                warn!(
                    "undervolt-probe: --confirm — executing ONE supervised F2 single step \
                     ({} MHz @ {} mV, +{} MHz) — can TDR/reboot.",
                    focus_target, cand.voltage_mv, cand.offset_mhz
                );
                let mut ops = RealF2Ops {
                    store,
                    curve: sane,
                    candidate: cand,
                    anchored: None,
                    mode: UndervoltMode::Simple,
                    limits,
                    target_mhz: focus_target,
                };
                let report = run_confirmed_f2_step(&mut ops);
                for line in confirmed_report_lines(focus_target, &cand, &limits, &report) {
                    println!("{line}");
                }
                info!("undervolt-probe: confirmed F2 single step (simple) outcome={:?}", report.outcome);
            }
        }
        return;
    }
    println!("(dry-run — pass `--steps 1 --confirm` for ONE supervised single step; nothing was written)");
    info!(
        "undervolt-probe: DRY-RUN (simple) — target={} MHz floor={} mV ceiling={} MHz steps={} candidates={} \
         preflight_safe={} — no Safe Loop arm, no apply, no dwell, no VF write.",
        focus_target, floor_mv, boost_top, max_steps, plan.points.len(), preflight.safe
    );
}

/// ANCHORED F2 probe (default mode). DRY-RUN by default: plans the classic undervolt point (raise the
/// anchor bin to target + cap the higher-voltage plateau to target + leave lower bins elastic), runs
/// the Safe Loop preflight read-only, prints the plan, and self-checks the planned curve with the SAME
/// anchored verifier the confirmed path uses. WITH `--confirm` it runs the fail-closed preflight then
/// ONE supervised single anchored step (single anchored curve; requires `--steps 1`; can TDR/reboot).
/// `--steps` ≥ 2 routes to the bounded same-target multi-step descent ([`run_anchored_multi_step`]).
/// Single-target only; never persists/applies/promotes a profile.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_anchored_undervolt_probe(
    store: &SafeLoopStore,
    confirm: bool,
    args: &UndervoltArgs,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    focus_target: u32,
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
) {
    // `--steps` ≥ 2 (anchored) → bounded same-target multi-step descent. `--steps` None or 1 keeps the
    // validated single anchored step below.
    if matches!(args.steps, Some(n) if n >= 2) {
        run_anchored_multi_step(store, confirm, args, sane, limits, focus_target, record, boot_flag_armed);
        return;
    }

    let probe = plan_anchored_undervolt(sane, focus_target, args.start_mv, limits);

    // Read-only Safe Loop preflight over every planned bin (anchor + caps + elastic). Axis keys match
    // build-frontier's intent so the blacklist applies consistently.
    let points: Vec<TuningPoint> = probe.plan.as_ref().map_or_else(Vec::new, |p| {
        p.entries
            .iter()
            .map(|e| {
                TuningPoint::from_axes([
                    ("gpu_freq_mhz", focus_target as i64),
                    ("gpu_vf_bin_mv", e.voltage_mv as i64),
                ])
            })
            .collect()
    });
    let preflight = undervolt_preflight(record, boot_flag_armed, &points);

    for line in anchored_plan_lines(&probe, limits, &preflight) {
        println!("{line}");
    }

    // Plan/verifier self-consistency: the planned curve must verify as AnchoredRaiseVerified using the
    // SAME verifier the confirmed path uses — catches planner/verifier drift without any hardware.
    if let Some(plan) = &probe.plan {
        let observed: Vec<(usize, Option<i32>)> =
            plan.entries.iter().map(|e| (e.index, Some(e.offset_mhz))).collect();
        let v = crate::gpu_verify::verify_anchored_positive_offset(plan, &observed, F2_VERIFY_TOL_MHZ);
        println!("plan self-check    : anchored plan verifies as {v:?} (tol {F2_VERIFY_TOL_MHZ} MHz)");
    }

    if confirm {
        // Confirmed anchored single step. The candidate is the anchor bin; confirmed mode requires
        // --steps 1 and re-runs the full fail-closed preflight before touching anything.
        let candidate = probe.plan.as_ref().map(|p| p.anchor);
        match confirmed_f2_refusal(
            record,
            boot_flag_armed,
            args.steps,
            candidate.as_ref(),
            limits,
            focus_target,
        ) {
            Some(reason) => {
                println!(
                    "undervolt-probe: --confirm REFUSED — {reason}. No Safe Loop arm, no apply, no \
                     dwell, no VF write performed."
                );
                warn!("undervolt-probe: --confirm refused (anchored): {reason} — no hardware touched");
            }
            None => {
                let plan = probe.plan.expect("refusal None guarantees a plan");
                let anchor = plan.anchor;
                warn!(
                    "undervolt-probe: --confirm — executing ONE supervised ANCHORED F2 step \
                     ({} MHz @ {} mV, +{} MHz anchor, {} plateau cap(s)) — can TDR/reboot.",
                    focus_target, anchor.voltage_mv, anchor.offset_mhz, plan.capped_above_bins
                );
                let mut ops = RealF2Ops {
                    store,
                    curve: sane.to_vec(),
                    candidate: anchor,
                    anchored: Some(plan.clone()),
                    mode: UndervoltMode::Anchored,
                    limits: *limits,
                    target_mhz: focus_target,
                };
                let report = run_confirmed_f2_step(&mut ops);
                for line in confirmed_report_lines(focus_target, &anchor, limits, &report) {
                    println!("{line}");
                }
                info!("undervolt-probe: confirmed ANCHORED F2 single step outcome={:?}", report.outcome);
            }
        }
        return;
    }
    println!(
        "(dry-run — pass `--steps 1 --confirm` for ONE supervised anchored single step; nothing was written)"
    );
    info!(
        "undervolt-probe: DRY-RUN (anchored) — target={} MHz floor={} mV ceiling={} MHz anchor_mv={:?} \
         has_plan={} preflight_safe={} — no Safe Loop arm, no apply, no dwell, no VF write.",
        focus_target, limits.hw_floor_mv, limits.clock_ceiling_mhz, probe.anchor_mv,
        probe.plan.is_some(), preflight.safe
    );
}

/// ANCHORED F2 same-target MULTI-STEP descent (`--steps` ≥ 2). DRY-RUN by default: plans the bounded
/// descent (anchored candidates at the SAME target, safer/higher voltage first), runs the Safe Loop
/// preflight read-only over every candidate's bins, and prints the plan + confirmed stop semantics +
/// the no-op line. WITH `--confirm` it runs the fail-closed run-level preflight ([`confirmed_f2_multi_refusal`],
/// enforcing the [`F2_CONFIRMED_MAX_STEPS`] cap) then drives [`run_confirmed_f2_multi_step`] over the
/// real per-candidate motor: each candidate arms Safe Loop → applies → verifies → dwells → resets →
/// clears, and the descent STOPS at the first non-stable candidate. Single-target, anchored-only,
/// capped; never persists/applies/promotes a profile.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_anchored_multi_step(
    store: &SafeLoopStore,
    confirm: bool,
    args: &UndervoltArgs,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    focus_target: u32,
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
) {
    // Dry-run previews up to the REQUESTED steps; the confirmed branch enforces its own cap below.
    let max_steps = args.steps.unwrap_or(F2_DEFAULT_STEPS);
    let descent = plan_anchored_undervolt_descent(sane, focus_target, args.start_mv, limits, max_steps);

    // Read-only Safe Loop preflight over every planned bin of every candidate (anchor + caps + elastic).
    let points: Vec<TuningPoint> = descent
        .candidates
        .iter()
        .flat_map(|c| {
            c.entries.iter().map(|e| {
                TuningPoint::from_axes([
                    ("gpu_freq_mhz", focus_target as i64),
                    ("gpu_vf_bin_mv", e.voltage_mv as i64),
                ])
            })
        })
        .collect();
    let preflight = undervolt_preflight(record, boot_flag_armed, &points);

    for line in anchored_descent_plan_lines(&descent, limits, &preflight, F2_CONFIRMED_MAX_STEPS) {
        println!("{line}");
    }

    if confirm {
        match confirmed_f2_multi_refusal(
            record,
            boot_flag_armed,
            args.steps,
            descent.candidates.len(),
            F2_CONFIRMED_MAX_STEPS,
        ) {
            Some(reason) => {
                println!(
                    "undervolt-probe: --confirm REFUSED — {reason}. No Safe Loop arm, no apply, no \
                     dwell, no VF write performed."
                );
                warn!("undervolt-probe: --confirm refused (anchored multi-step): {reason} — no hardware touched");
            }
            None => {
                warn!(
                    "undervolt-probe: --confirm — executing up to {} ANCHORED candidate(s) at {} MHz \
                     (descending voltage; stops at first non-stable) — can TDR/reboot.",
                    F2_CONFIRMED_MAX_STEPS.min(descent.candidates.len()),
                    focus_target
                );
                let mut ops = RealF2MultiOps {
                    store,
                    curve: sane.to_vec(),
                    candidates: descent.candidates.clone(),
                    limits: *limits,
                    target_mhz: focus_target,
                    cur: None,
                };
                let report = run_confirmed_f2_multi_step(&mut ops, F2_CONFIRMED_MAX_STEPS);
                for line in confirmed_multi_report_lines(focus_target, &descent.candidates, limits, &report) {
                    println!("{line}");
                }
                info!(
                    "undervolt-probe: confirmed ANCHORED multi-step — planned={} executed={} last_good={:?} stop={:?}",
                    report.planned, report.executed, report.last_good_index, report.stop_reason
                );
            }
        }
        return;
    }
    println!(
        "(dry-run — pass `--steps N --confirm` (N up to {F2_CONFIRMED_MAX_STEPS}) for a supervised \
         anchored descent; nothing was written)"
    );
    info!(
        "undervolt-probe: DRY-RUN (anchored multi-step) — target={} MHz candidates={} requested_steps={} \
         confirmed_cap={} preflight_safe={} — no Safe Loop arm, no apply, no dwell, no VF write.",
        focus_target, descent.candidates.len(), max_steps, F2_CONFIRMED_MAX_STEPS, preflight.safe
    );
}

/// Non-Windows stub — F2 undervolt-probe is Windows-only (NVAPI/NVML).
#[cfg(not(windows))]
pub fn run_undervolt_probe(_store: &SafeLoopStore, _confirm: bool, _args: UndervoltArgs) {
    tracing::warn!("undervolt-probe is Windows-only");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidavellir_core::safe_loop::BlacklistRegion;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    // (index, voltage_mv, base_freq_mhz): boost-top bin at 1062/1755; lower bins below the target.
    fn t_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1700), (1, 900, 1725), (2, 950, 1740), (3, 1000, 1748), (4, 1062, 1755)]
    }

    fn pt(freq: u32, mv: u32) -> TuningPoint {
        TuningPoint::from_axes([("gpu_freq_mhz", freq as i64), ("gpu_vf_bin_mv", mv as i64)])
    }

    // ── parse_undervolt_args ──────────────────────────────────────────────────────────────────
    #[test]
    fn parse_undervolt_args_reads_flags_and_fails_closed() {
        let a = parse_undervolt_args(&os(&[
            "undervolt-probe", "--target-mhz", "1755", "--start-mv", "1000", "--steps", "4",
        ]))
        .unwrap();
        assert_eq!(a.target_mhz, Some(1755));
        assert_eq!(a.start_mv, Some(1000));
        assert_eq!(a.steps, Some(4));
        // Defaults when absent — and the DEFAULT mode is anchored.
        let d = parse_undervolt_args(&os(&["undervolt-probe"])).unwrap();
        assert_eq!((d.target_mhz, d.start_mv, d.steps), (None, None, None));
        assert_eq!(d.mode, UndervoltMode::Anchored);
        // Missing / non-numeric values fail closed.
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--target-mhz"])).is_err());
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--steps", "x"])).is_err());
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--start-mv", "abc"])).is_err());
    }

    #[test]
    fn parse_undervolt_args_reads_mode_flags() {
        // --simple selects the original single-bin mode; --anchored is explicit (and the default).
        assert_eq!(
            parse_undervolt_args(&os(&["undervolt-probe", "--simple"])).unwrap().mode,
            UndervoltMode::Simple
        );
        assert_eq!(
            parse_undervolt_args(&os(&["undervolt-probe", "--anchored"])).unwrap().mode,
            UndervoltMode::Anchored
        );
        // Last mode flag wins.
        assert_eq!(
            parse_undervolt_args(&os(&["undervolt-probe", "--simple", "--anchored"])).unwrap().mode,
            UndervoltMode::Anchored
        );
    }

    // ── plan_undervolt_probe (pure search; no hardware, no write) ─────────────────────────────
    #[test]
    fn dry_run_plan_plans_without_writing() {
        // target = boost top 1755; floor 850; ceiling 1755; conservative caps (+30 abs / +15 step).
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        let plan = plan_undervolt_probe(&t_base(), 1755, None, &limits, 4);
        // The 1062 mV bin (base == target) is skipped; lower bins get bounded positive offsets.
        assert!(plan.skipped_above_target >= 1);
        // 1000 (+7), 950 (+15), 900 (+30) hold; 850 would need +55 > abs cap → descent stops.
        assert_eq!(plan.points.len(), 3);
        assert_eq!(plan.points.iter().map(|p| p.voltage_mv).collect::<Vec<_>>(), vec![1000, 950, 900]);
        for p in &plan.points {
            assert_eq!(p.effective_mhz, 1755);
            assert!(p.offset_mhz > 0 && p.offset_mhz <= limits.abs_max_offset_mhz);
            assert!(p.step_delta_mhz <= limits.step_max_offset_mhz);
        }
        // The descent stopped on the absolute-cap bound (the 850 mV bin needs +55).
        assert!(plan.stop_reason.as_deref().unwrap_or_default().contains("absolute cap"));
    }

    #[test]
    fn dry_run_plan_respects_step_budget() {
        // target 1755: 1000 (+7) and 950 (+15) are planned, then the 2-step budget halts the descent
        // before the 900 mV bin (which would be the 3rd candidate).
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        let plan = plan_undervolt_probe(&t_base(), 1755, None, &limits, 2);
        assert_eq!(plan.points.len(), 2);
        assert_eq!(plan.points.iter().map(|p| p.voltage_mv).collect::<Vec<_>>(), vec![1000, 950]);
        assert!(plan.stop_reason.as_deref().unwrap_or_default().contains("step budget"));
    }

    // ── dry-run output / no-op semantics ──────────────────────────────────────────────────────
    #[test]
    fn dry_run_plan_lines_state_no_write_semantics() {
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        let plan = plan_undervolt_probe(&t_base(), 1755, None, &limits, 4);
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = undervolt_plan_lines(&plan, &limits, &pf).join("\n");
        // Explicit no-op / no-write semantics.
        assert!(text.contains("no Safe Loop arm"));
        assert!(text.contains("no apply"));
        assert!(text.contains("no dwell"));
        assert!(text.contains("no VF write"));
        // Required plan content.
        assert!(text.contains("focus target"));
        assert!(text.contains("offset caps"));
        assert!(text.contains("voltage floor"));
        assert!(text.contains("clock ceiling"));
        assert!(text.contains("reset_to_stock"));
        assert!(text.contains("Safe Loop preflight"));
    }

    // ── anchored probe (plan_anchored_undervolt / select_anchor_bin / anchored_plan_lines) ────
    // Anchor-focused base: lower bins below target, higher bins above target so the plateau caps
    // engage. (idx, mV, base): 850/1700, 900/1740, 950/1770, 1000/1800, 1062/1845.
    fn a_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1700), (1, 900, 1740), (2, 950, 1770), (3, 1000, 1800), (4, 1062, 1845)]
    }

    #[test]
    fn select_anchor_bin_picks_highest_below_target() {
        // target 1755: bins below it are 850(1700) and 900(1740) → highest is the 900 mV bin (idx 1).
        assert_eq!(select_anchor_bin(&a_base(), 1755, None), Some(1));
        // --start-mv 875 ignores the 900 mV bin → the 850 mV bin (idx 0) becomes the anchor.
        assert_eq!(select_anchor_bin(&a_base(), 1755, Some(875)), Some(0));
        // Target at/below every base → nothing to anchor.
        assert_eq!(select_anchor_bin(&a_base(), 1600, None), None);
    }

    #[test]
    fn anchored_probe_raises_anchor_caps_plateau_and_keeps_lower_elastic() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let probe = plan_anchored_undervolt(&a_base(), 1755, None, &limits);
        assert_eq!(probe.anchor_mv, Some(900));
        let plan = probe.plan.expect("a valid anchored plan");
        // Anchor 900 mV raised +15 → 1755.
        assert_eq!((plan.anchor.voltage_mv, plan.anchor.offset_mhz, plan.anchor.effective_mhz), (900, 15, 1755));
        // Plateau (950/1000/1062) capped DOWN to target; lower bin (850) elastic.
        assert_eq!(plan.capped_above_bins, 3);
        assert_eq!(plan.elastic_below_bins, 1);
        // Exactly one positive offset, and no bin sits above target.
        assert_eq!(plan.entries.iter().filter(|e| e.offset_mhz > 0).count(), 1);
        assert!(plan.entries.iter().all(|e| e.effective_mhz <= 1755));
    }

    #[test]
    fn anchored_probe_single_candidate_only() {
        // The anchored probe yields exactly ONE anchored curve (single candidate), never a descent list.
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let probe = plan_anchored_undervolt(&a_base(), 1755, None, &limits);
        let plan = probe.plan.unwrap();
        assert_eq!(plan.max_positive_offset_mhz, plan.anchor.offset_mhz);
        assert_eq!(plan.entries.iter().filter(|e| e.role == AnchoredBinRole::Anchor).count(), 1);
    }

    #[test]
    fn anchored_probe_reports_note_when_no_candidate() {
        // Target at/below every base → no anchor, a note, and no plan (never a silent empty write).
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let probe = plan_anchored_undervolt(&a_base(), 1600, None, &limits);
        assert!(probe.plan.is_none());
        assert!(probe.note.is_some());
        assert!(probe.anchor_mv.is_none());
    }

    #[test]
    fn anchored_plan_lines_state_anchored_mode_and_no_write() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let probe = plan_anchored_undervolt(&a_base(), 1755, None, &limits);
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = anchored_plan_lines(&probe, &limits, &pf).join("\n");
        // Mode + the anchored guarantee + the explicit no-op/no-write semantics.
        assert!(text.contains("ANCHORED"));
        assert!(text.contains("anchored mode prevents boost above the target"));
        assert!(text.contains("no Safe Loop arm"));
        assert!(text.contains("no apply"));
        assert!(text.contains("no dwell"));
        assert!(text.contains("no VF write"));
        // Required anchored plan content.
        assert!(text.contains("anchor bin"));
        assert!(text.contains("max positive offset"));
        assert!(text.contains("max neg flatten"));
        assert!(text.contains("higher-voltage bins"));
        assert!(text.contains("clock ceiling"));
        assert!(text.contains("voltage floor"));
    }

    #[test]
    fn anchored_confirmed_branch_is_single_step_only() {
        // The anchor IS a PositiveOffsetPlan candidate → the confirmed preflight refuses --steps != 1
        // and allows --steps 1, exactly like the simple path (single anchored curve = single step).
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let plan = plan_anchored_undervolt(&a_base(), 1755, None, &limits).plan.unwrap();
        let anchor = plan.anchor;
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(3), Some(&anchor), &limits, 1755)
            .unwrap()
            .contains("single-step only"));
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), Some(&anchor), &limits, 1755)
            .is_none());
    }

    // ── undervolt_preflight (pure; refuses unsafe state) ──────────────────────────────────────
    #[test]
    fn preflight_refuses_unsafe_state() {
        let pts = vec![pt(1755, 900)];
        // Safe Mode active → refuse.
        let mut rec = SafeLoopRecord::default();
        rec.safe_mode = true;
        let v = undervolt_preflight(&rec, false, &pts);
        assert!(!v.safe && v.safe_mode);
        // Boot flag already armed → refuse.
        let v2 = undervolt_preflight(&SafeLoopRecord::default(), true, &pts);
        assert!(!v2.safe && v2.boot_flag_armed);
        // A planned point inside a blacklisted region → refuse.
        let mut rec3 = SafeLoopRecord::default();
        rec3.blacklist.push(BlacklistRegion::around(pt(1755, 900), 5));
        let v3 = undervolt_preflight(&rec3, false, &pts);
        assert!(!v3.safe);
        assert_eq!(v3.blacklisted_points, 1);
        // Clean state → safe.
        let v4 = undervolt_preflight(&SafeLoopRecord::default(), false, &pts);
        assert!(v4.safe && v4.reasons.is_empty());
    }

    // ── confirmed F2 single-step: usage, preflight, fail-closed state machine (mock; no HW) ────
    #[test]
    fn usage_lists_flags_and_confirm_warning() {
        let u = undervolt_usage();
        assert!(u.contains("Usage"));
        assert!(u.contains("--target-mhz"));
        assert!(u.contains("--start-mv"));
        assert!(u.contains("--steps"));
        assert!(u.contains("--simple"));
        assert!(u.contains("--anchored"));
        assert!(u.contains("--confirm"));
        assert!(u.contains("-h, --help"));
        assert!(u.to_lowercase().contains("operator"));
        assert!(u.to_uppercase().contains("ANCHORED"));
        assert!(u.contains("WARNING"));
        // Text only — it does not produce a plan / candidate listing.
        assert!(!u.contains("candidate bins"));
    }

    fn cand() -> PositiveOffsetPlan {
        // A valid bounded candidate reachable from prev_offset 0 (1000 mV base 1748, target 1755 → +7).
        plan_bounded_positive_offset(&t_base(), 3, 1755, 0, &cand_limits()).unwrap()
    }
    fn cand_limits() -> PositiveOffsetLimits {
        PositiveOffsetLimits::conservative(850, 1900)
    }

    #[test]
    fn confirmed_refuses_when_steps_not_one() {
        let c = cand();
        let r = confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(3), Some(&c), &cand_limits(), 1755);
        assert!(r.unwrap().contains("single-step only"));
        // Unset steps is also refused.
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, None, Some(&c), &cand_limits(), 1755).is_some());
        // --steps 1 with a clean record → allowed.
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), Some(&c), &cand_limits(), 1755).is_none());
    }

    #[test]
    fn confirmed_refuses_when_no_candidate() {
        let r = confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), None, &cand_limits(), 1755);
        assert!(r.unwrap().contains("no valid candidate"));
    }

    #[test]
    fn confirmed_refuses_safe_mode() {
        let mut rec = SafeLoopRecord::default();
        rec.safe_mode = true;
        let c = cand();
        assert!(confirmed_f2_refusal(&rec, false, Some(1), Some(&c), &cand_limits(), 1755)
            .unwrap()
            .contains("Safe Mode"));
    }

    #[test]
    fn confirmed_refuses_armed_boot_flag() {
        let c = cand();
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), true, Some(1), Some(&c), &cand_limits(), 1755)
            .unwrap()
            .contains("boot flag"));
    }

    #[test]
    fn confirmed_refuses_blacklisted_intent() {
        let c = cand();
        // Blacklist the 2-axis (freq, vf_bin) point — confirmed preflight must still catch it.
        let mut rec = SafeLoopRecord::default();
        rec.blacklist.push(BlacklistRegion::around(pt(1755, c.voltage_mv), 2));
        assert!(confirmed_f2_refusal(&rec, false, Some(1), Some(&c), &cand_limits(), 1755)
            .unwrap()
            .contains("blacklisted"));
    }

    #[test]
    fn confirmed_refuses_excessive_consecutive_crashes() {
        let mut rec = SafeLoopRecord::default();
        rec.consecutive_crashes = SAFE_MODE_CRASH_THRESHOLD; // at the abort threshold
        let c = cand();
        assert!(confirmed_f2_refusal(&rec, false, Some(1), Some(&c), &cand_limits(), 1755)
            .unwrap()
            .contains("consecutive_crashes"));
        // Below threshold → allowed.
        rec.consecutive_crashes = SAFE_MODE_CRASH_THRESHOLD - 1;
        assert!(confirmed_f2_refusal(&rec, false, Some(1), Some(&c), &cand_limits(), 1755).is_none());
    }

    // Mock F2Ops with a call log + configurable per-op results.
    struct MockOps {
        log: Vec<&'static str>,
        arm: Result<(), String>,
        apply: Result<(), String>,
        verify: PositiveOffsetVerification,
        dwell: F2DwellOutcome,
        reset: Result<(), String>,
        clear: Result<(), String>,
        blacklist: Result<(), String>,
    }
    impl MockOps {
        fn happy() -> Self {
            MockOps {
                log: Vec::new(),
                arm: Ok(()),
                apply: Ok(()),
                verify: PositiveOffsetVerification::RaiseVerified,
                dwell: F2DwellOutcome::Stable,
                reset: Ok(()),
                clear: Ok(()),
                blacklist: Ok(()),
            }
        }
    }
    impl F2Ops for MockOps {
        fn arm_boot_flag(&mut self) -> Result<(), String> { self.log.push("arm"); self.arm.clone() }
        fn apply_positive_offset(&mut self) -> Result<(), String> { self.log.push("apply"); self.apply.clone() }
        fn verify(&mut self) -> PositiveOffsetVerification { self.log.push("verify"); self.verify }
        fn dwell(&mut self) -> F2DwellResult {
            self.log.push("dwell");
            // Dummy headline stats; the single-step state machine only branches on `outcome`.
            F2DwellResult { outcome: self.dwell, avg_clock_mhz: 1815, p5_clock_mhz: 1815, power_w: 183.0 }
        }
        fn reset_to_stock(&mut self) -> Result<(), String> { self.log.push("reset"); self.reset.clone() }
        fn clear_boot_flag(&mut self) -> Result<(), String> { self.log.push("clear"); self.clear.clone() }
        fn blacklist_point(&mut self) -> Result<(), String> { self.log.push("blacklist"); self.blacklist.clone() }
    }

    #[test]
    fn confirmed_success_path_order_and_state() {
        let mut ops = MockOps::happy();
        let r = run_confirmed_f2_step(&mut ops);
        // Exact sequence: arm BEFORE write, then verify, dwell, reset, clear.
        assert_eq!(ops.log, vec!["arm", "apply", "verify", "dwell", "reset", "clear"]);
        assert_eq!(r.outcome, F2Outcome::Validated);
        assert!(r.armed && r.applied && r.validated && r.boot_flag_cleared);
        assert_eq!(r.reset_ok, Some(true));
        assert!(!r.blacklisted);
    }

    #[test]
    fn confirmed_verify_fail_resets_no_validate() {
        let mut ops = MockOps::happy();
        ops.verify = PositiveOffsetVerification::RaiseIncomplete;
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(ops.log, vec!["arm", "apply", "verify", "reset", "clear"]); // never dwells
        assert_eq!(r.outcome, F2Outcome::VerifyFailed);
        assert!(!r.validated);
        assert!(r.boot_flag_cleared);
        assert_eq!(r.reset_ok, Some(true));
    }

    #[test]
    fn confirmed_device_lost_retains_flag_and_blacklists() {
        let mut ops = MockOps::happy();
        ops.dwell = F2DwellOutcome::DeviceLost;
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(r.outcome, F2Outcome::DeviceLost);
        assert!(!r.validated);
        assert!(r.blacklisted);
        assert!(!r.boot_flag_cleared); // RETAINED on device loss
        assert!(!ops.log.contains(&"clear"));
        assert!(ops.log.contains(&"blacklist"));
    }

    #[test]
    fn confirmed_unstable_blacklists_and_clears_after_reset() {
        let mut ops = MockOps::happy();
        ops.dwell = F2DwellOutcome::Unstable;
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(r.outcome, F2Outcome::Unstable);
        assert!(!r.validated);
        assert!(r.blacklisted);
        assert!(r.boot_flag_cleared); // reset ok, no device loss → safe to clear
    }

    #[test]
    fn confirmed_reset_failure_retains_flag_fail_closed() {
        let mut ops = MockOps::happy();
        ops.reset = Err("reset readback offset 15000 kHz not cleared".to_string());
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(r.outcome, F2Outcome::ResetFailed);
        assert!(!r.boot_flag_cleared);
        assert!(!r.validated);
        assert_eq!(r.reset_ok, Some(false));
        assert!(!ops.log.contains(&"clear"));
    }

    #[test]
    fn confirmed_arm_failure_does_not_write() {
        let mut ops = MockOps::happy();
        ops.arm = Err("arm_boot_flag: io".to_string());
        let r = run_confirmed_f2_step(&mut ops);
        assert!(matches!(r.outcome, F2Outcome::ArmFailed(_)));
        assert!(!r.armed && !r.applied);
        assert!(!ops.log.contains(&"apply"));
        assert!(!ops.log.contains(&"verify"));
        assert!(!ops.log.contains(&"dwell"));
    }

    #[test]
    fn confirmed_has_no_persist_apply_or_promote_op() {
        // The whole success sequence is exactly arm→apply(offset)→verify→dwell→reset→clear; the trait
        // exposes NO profile persistence / apply / promotion operation, so a confirmed step cannot
        // persist, apply, or promote a profile. ("apply" is the bounded VF-offset write, not a profile.)
        let mut ops = MockOps::happy();
        let _ = run_confirmed_f2_step(&mut ops);
        for step in &ops.log {
            assert!(matches!(*step, "arm" | "apply" | "verify" | "dwell" | "reset" | "clear" | "blacklist"));
        }
        assert!(!ops.log.iter().any(|s| s.contains("persist") || s.contains("promote")));
    }

    #[test]
    fn confirmed_clock_drop_resets_no_validate_no_blacklist() {
        // A STABLE-but-sagging dwell (ClockDrop) is not a crash/instability: reset, clear on a clean
        // reset, never validate, and never blacklist (single-step motor; the descent stops upstream).
        let mut ops = MockOps::happy();
        ops.dwell = F2DwellOutcome::ClockDrop;
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(r.outcome, F2Outcome::ClockDrop);
        assert!(!r.validated);
        assert!(!r.blacklisted);
        assert!(r.boot_flag_cleared); // reset ok, no device loss → safe to clear
        assert_eq!(r.reset_ok, Some(true));
        assert!(!ops.log.contains(&"blacklist"));
    }

    #[test]
    fn single_step_gate_unchanged_by_multi_step() {
        // The validated single-step refusal is untouched: --steps 1 allowed, --steps 3 still refused as
        // "single-step only" (multi-step is a SEPARATE gate, `confirmed_f2_multi_refusal`).
        let c = cand();
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), Some(&c), &cand_limits(), 1755).is_none());
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(3), Some(&c), &cand_limits(), 1755)
            .unwrap()
            .contains("single-step only"));
    }

    // ── F2 anchored MULTI-STEP descent (pure planner + orchestrator; no hardware) ──────────────
    // Descent fixture: four bins below target 1755 with bounded raises (+7/+15/+30, then +45 > abs
    // cap), plus a 1062 mV bin above target so the plateau cap engages. (idx, mV, base_mhz).
    fn d_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1710), (1, 900, 1725), (2, 950, 1740), (3, 1000, 1748), (4, 1062, 1800)]
    }

    #[test]
    fn descent_plans_multiple_anchored_candidates_descending_voltage() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let d = plan_anchored_undervolt_descent(&d_base(), 1755, None, &limits, 4);
        // The 1062 mV bin (base above target) is skipped; 1000/950/900 anchor; 850 needs +45 > abs cap.
        assert!(d.skipped_above_target >= 1);
        assert_eq!(d.candidates.len(), 3);
        let anchors: Vec<u32> = d.candidates.iter().map(|c| c.anchor.voltage_mv).collect();
        assert_eq!(anchors, vec![1000, 950, 900]); // safer/higher voltage first
        let offsets: Vec<i32> = d.candidates.iter().map(|c| c.anchor.offset_mhz).collect();
        assert_eq!(offsets, vec![7, 15, 30]);
        // Every candidate holds the SAME target with no bin above it and exactly one positive offset.
        for c in &d.candidates {
            assert_eq!(c.anchor.effective_mhz, 1755);
            assert!(c.entries.iter().all(|e| e.effective_mhz <= 1755));
            assert_eq!(c.entries.iter().filter(|e| e.offset_mhz > 0).count(), 1);
        }
        // The descent stopped on the absolute-cap bound at the 850 mV bin (needs +45).
        assert!(d.stop_reason.as_deref().unwrap_or_default().contains("absolute cap"));
    }

    #[test]
    fn descent_respects_step_budget() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let d = plan_anchored_undervolt_descent(&d_base(), 1755, None, &limits, 2);
        assert_eq!(d.candidates.len(), 2);
        assert_eq!(
            d.candidates.iter().map(|c| c.anchor.voltage_mv).collect::<Vec<_>>(),
            vec![1000, 950]
        );
        assert!(d.stop_reason.as_deref().unwrap_or_default().contains("step budget"));
    }

    #[test]
    fn descent_plan_lines_show_anchored_cap_and_no_write() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let d = plan_anchored_undervolt_descent(&d_base(), 1755, None, &limits, 3);
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = anchored_descent_plan_lines(&d, &limits, &pf, F2_CONFIRMED_MAX_STEPS).join("\n");
        assert!(text.contains("ANCHORED"));
        assert!(text.contains("confirmed cap"));
        assert!(text.contains("F2_CONFIRMED_MAX_STEPS=3"));
        assert!(text.contains("anchor")); // per-candidate anchor lines
        assert!(text.contains("capped DOWN"));
        assert!(text.contains("anchored mode prevents boost above the target"));
        assert!(text.contains("confirmed stop")); // exact stop semantics for confirmed mode
        // Explicit no-op / no-write semantics.
        assert!(text.contains("no Safe Loop arm"));
        assert!(text.contains("no apply"));
        assert!(text.contains("no dwell"));
        assert!(text.contains("no VF write"));
    }

    // ── confirmed multi-step refusal (run-level gate) ─────────────────────────────────────────
    #[test]
    fn confirmed_multi_refusal_enforces_cap_and_state() {
        let rec = SafeLoopRecord::default();
        // No --steps → refuse.
        assert!(confirmed_f2_multi_refusal(&rec, false, None, 3, 3).is_some());
        // Above the cap → refuse (fail closed), regardless of available candidates.
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(4), 3, 3).unwrap().contains("capped"));
        // Within the cap (1..=3) with candidates + a clean record → allowed.
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(1), 1, 3).is_none());
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(2), 2, 3).is_none());
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(3), 3, 3).is_none());
        // No candidates → refuse.
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(2), 0, 3)
            .unwrap()
            .contains("no anchored candidates"));
        // Safe Mode / armed boot flag / crash threshold → refuse.
        let mut sm = SafeLoopRecord::default();
        sm.safe_mode = true;
        assert!(confirmed_f2_multi_refusal(&sm, false, Some(2), 2, 3).unwrap().contains("Safe Mode"));
        assert!(confirmed_f2_multi_refusal(&rec, true, Some(2), 2, 3).unwrap().contains("boot flag"));
        let mut cc = SafeLoopRecord::default();
        cc.consecutive_crashes = SAFE_MODE_CRASH_THRESHOLD;
        assert!(confirmed_f2_multi_refusal(&cc, false, Some(2), 2, 3)
            .unwrap()
            .contains("consecutive_crashes"));
    }

    // ── multi-step orchestrator (mock; drives the REAL per-candidate motor; no hardware) ───────
    #[derive(Clone)]
    struct CandScript {
        precheck: Result<(), String>,
        arm: Result<(), String>,
        apply: Result<(), String>,
        verify: PositiveOffsetVerification,
        dwell: F2DwellOutcome,
        reset: Result<(), String>,
        clear: Result<(), String>,
        blacklist: Result<(), String>,
    }
    impl CandScript {
        fn stable() -> Self {
            CandScript {
                precheck: Ok(()),
                arm: Ok(()),
                apply: Ok(()),
                verify: PositiveOffsetVerification::RaiseVerified,
                dwell: F2DwellOutcome::Stable,
                reset: Ok(()),
                clear: Ok(()),
                blacklist: Ok(()),
            }
        }
        fn with_dwell(mut self, d: F2DwellOutcome) -> Self {
            self.dwell = d;
            self
        }
    }
    struct MockMultiOps {
        scripts: Vec<CandScript>,
        cur: usize,
        log: Vec<String>,
    }
    impl MockMultiOps {
        fn new(scripts: Vec<CandScript>) -> Self {
            MockMultiOps { scripts, cur: 0, log: Vec::new() }
        }
        fn s(&self) -> &CandScript {
            &self.scripts[self.cur]
        }
    }
    impl F2Ops for MockMultiOps {
        fn arm_boot_flag(&mut self) -> Result<(), String> { self.log.push(format!("arm{}", self.cur)); self.s().arm.clone() }
        fn apply_positive_offset(&mut self) -> Result<(), String> { self.log.push(format!("apply{}", self.cur)); self.s().apply.clone() }
        fn verify(&mut self) -> PositiveOffsetVerification { self.log.push(format!("verify{}", self.cur)); self.s().verify }
        fn dwell(&mut self) -> F2DwellResult {
            self.log.push(format!("dwell{}", self.cur));
            F2DwellResult { outcome: self.s().dwell, avg_clock_mhz: 1815, p5_clock_mhz: 1815, power_w: 183.0 }
        }
        fn reset_to_stock(&mut self) -> Result<(), String> { self.log.push(format!("reset{}", self.cur)); self.s().reset.clone() }
        fn clear_boot_flag(&mut self) -> Result<(), String> { self.log.push(format!("clear{}", self.cur)); self.s().clear.clone() }
        fn blacklist_point(&mut self) -> Result<(), String> { self.log.push(format!("blacklist{}", self.cur)); self.s().blacklist.clone() }
    }
    impl F2MultiStepOps for MockMultiOps {
        fn candidate_count(&self) -> usize { self.scripts.len() }
        fn select(&mut self, i: usize) -> Result<(), String> {
            self.cur = i;
            self.log.push(format!("select{i}"));
            self.scripts[i].precheck.clone()
        }
    }

    #[test]
    fn multi_step_runs_in_order_and_completes_all() {
        let mut ops = MockMultiOps::new(vec![CandScript::stable(), CandScript::stable(), CandScript::stable()]);
        let r = run_confirmed_f2_multi_step(&mut ops, F2_CONFIRMED_MAX_STEPS);
        assert_eq!(r.stop_reason, F2MultiStopReason::CompletedAllPlanned);
        assert_eq!((r.planned, r.executed), (3, 3));
        assert_eq!(r.last_good_index, Some(2));
        // Exact per-candidate sequence, safer/higher voltage (index 0) first.
        assert_eq!(
            ops.log,
            vec![
                "select0", "arm0", "apply0", "verify0", "dwell0", "reset0", "clear0",
                "select1", "arm1", "apply1", "verify1", "dwell1", "reset1", "clear1",
                "select2", "arm2", "apply2", "verify2", "dwell2", "reset2", "clear2",
            ]
        );
        // Reset + boot-flag clear after EVERY executed candidate; all validated.
        for s in &r.steps {
            assert!(s.validated);
            assert_eq!(s.reset_ok, Some(true));
            assert!(s.boot_flag_cleared);
        }
    }

    #[test]
    fn multi_step_enforces_cap_when_more_candidates_planned() {
        // Five planned candidates but the cap is 3 → only 3 execute.
        let mut ops = MockMultiOps::new(vec![CandScript::stable(); 5]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!((r.planned, r.executed), (3, 3));
        assert_eq!(r.stop_reason, F2MultiStopReason::CompletedAllPlanned);
        assert!(!ops.log.iter().any(|l| l == "select3" || l == "select4"));
    }

    #[test]
    fn multi_step_stops_after_verifier_fail() {
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable(),
            CandScript { verify: PositiveOffsetVerification::RaiseIncomplete, ..CandScript::stable() },
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::VerifierFailed);
        assert_eq!(r.executed, 2);
        assert_eq!(r.last_good_index, Some(0)); // only candidate 0 validated
        assert!(!ops.log.iter().any(|l| l == "select2")); // never reaches candidate 2
    }

    #[test]
    fn multi_step_stops_after_unstable() {
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable(),
            CandScript::stable().with_dwell(F2DwellOutcome::Unstable),
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::Unstable);
        assert_eq!(r.executed, 2);
        assert_eq!(r.last_good_index, Some(0));
        assert!(!ops.log.iter().any(|l| l == "select2"));
        // The unstable candidate reset and was blacklisted but never validated.
        assert!(r.steps[1].blacklisted);
        assert!(!r.steps[1].validated);
    }

    #[test]
    fn multi_step_stops_after_device_lost() {
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable().with_dwell(F2DwellOutcome::DeviceLost),
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::DeviceLost);
        assert_eq!(r.executed, 1);
        assert_eq!(r.last_good_index, None);
        assert!(!ops.log.iter().any(|l| l == "select1")); // descent never continues after device loss
        assert!(!r.steps[0].boot_flag_cleared); // boot flag RETAINED for startup recovery
        assert!(r.steps[0].blacklisted);
    }

    #[test]
    fn multi_step_stops_after_clock_drop() {
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable(),
            CandScript::stable().with_dwell(F2DwellOutcome::ClockDrop),
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::ClockDrop);
        assert_eq!(r.executed, 2);
        assert_eq!(r.last_good_index, Some(0));
        assert!(!ops.log.iter().any(|l| l == "select2"));
    }

    #[test]
    fn multi_step_reset_failure_retains_flag_and_stops() {
        let mut ops = MockMultiOps::new(vec![
            CandScript { reset: Err("readback not cleared".to_string()), ..CandScript::stable() },
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::ResetFailed);
        assert_eq!(r.executed, 1);
        assert_eq!(r.last_good_index, None);
        assert_eq!(r.steps[0].reset_ok, Some(false));
        assert!(!r.steps[0].boot_flag_cleared); // fail closed — flag retained
        assert!(!ops.log.iter().any(|l| l == "select1"));
    }

    #[test]
    fn multi_step_blacklist_precheck_stops_before_write() {
        // Candidate 0 runs; candidate 1's precheck refuses → stop Blacklisted, candidate 1 never writes.
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable(),
            CandScript { precheck: Err("blacklisted".to_string()), ..CandScript::stable() },
            CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::Blacklisted);
        assert_eq!(r.executed, 1); // only candidate 0 ran the motor
        assert_eq!(r.last_good_index, Some(0));
        assert!(ops.log.iter().any(|l| l == "select1")); // precheck happened
        assert!(!ops.log.iter().any(|l| l == "arm1")); // but no write for candidate 1
    }

    #[test]
    fn multi_step_last_good_is_last_stable_only() {
        // Two stable then a verifier fail → last_good is candidate 1 (the last stable one), not 2.
        let mut ops = MockMultiOps::new(vec![
            CandScript::stable(),
            CandScript::stable(),
            CandScript { verify: PositiveOffsetVerification::RaiseIncomplete, ..CandScript::stable() },
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, 3);
        assert_eq!(r.stop_reason, F2MultiStopReason::VerifierFailed);
        assert_eq!(r.last_good_index, Some(1));
    }

    #[test]
    fn multi_step_has_no_persist_apply_or_promote_op() {
        let mut ops = MockMultiOps::new(vec![CandScript::stable(), CandScript::stable()]);
        let _ = run_confirmed_f2_multi_step(&mut ops, 3);
        assert!(!ops.log.iter().any(|s| s.contains("persist") || s.contains("promote")));
        // Every logged op is a known safe per-candidate primitive (or the select cursor).
        for s in &ops.log {
            let base: String = s.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
            assert!(matches!(
                base.as_str(),
                "select" | "arm" | "apply" | "verify" | "dwell" | "reset" | "clear" | "blacklist"
            ));
        }
    }
}
