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
//!   with `--confirm` — the first real CONFIRMED single-step hardware branch
//!   ([`run_confirmed_f2_step`] over the [`F2Ops`] trait): arm Safe Loop → apply ONE bounded positive
//!   offset → verify → dwell once → `reset_to_stock` on EVERY exit path → clear the boot flag ONLY
//!   after a confirmed reset. Single-target, single-step only.
//!
//! Safety invariants: no profile apply/persist/promotion, no multi-target loop, no autonomous
//! crash-seeking, no power-limit/TDP/clock-lock change; the dry-run reads Safe Loop state READ-ONLY
//! (it never arms the boot flag, mutates the record, applies, dwells, or writes VF); and the
//! confirmed branch never leaves a positive offset applied after exit (a reset that cannot be
//! confirmed fails closed and retains the boot flag for startup recovery).

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

/// Tolerance (MHz) for the verifier (one boost bin) — used by the dry-run self-check and the
/// confirmed-mode post-write verify.
#[cfg(windows)]
const F2_VERIFY_TOL_MHZ: u32 = 15;

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

/// Help/usage text for `undervolt-probe`. Pure — printing it reads no hardware, plans nothing, and
/// mutates nothing. Includes an explicit `--confirm` hardware warning.
pub fn undervolt_usage() -> String {
    [
        "Usage: nidavellir-service undervolt-probe [OPTIONS]",
        "",
        "F2 true-undervolt probe. The default ANCHORED mode plans a classic undervolt point: it raises",
        "the chosen (lower-voltage) anchor bin to the focus clock AND caps every higher-voltage bin to",
        "the same clock so the GPU cannot boost above the target. Dry-run by default; with --confirm it",
        "executes ONE supervised single anchored step.",
        "",
        "Options:",
        "  --target-mhz <MHz>   Focus target clock (default: stock boost top; never overclocks).",
        "  --start-mv <mV>      Highest voltage bin to anchor at (default: curve top).",
        "  --steps <N>          Candidate bins to plan in --simple mode (default: 4). Confirmed mode",
        "                       REQUIRES --steps 1.",
        "  --anchored           Plan the anchored classic undervolt point (DEFAULT).",
        "  --simple             Plan the original single-bin positive-offset descent (boost above the",
        "                       target is NOT prevented; for comparison/diagnostics only).",
        "  --confirm            Execute ONE supervised single step (see WARNING). Default: dry-run.",
        "  -h, --help           Print this help and exit (no hardware read, no plan, no mutation).",
        "",
        "WARNING: --confirm MAY WRITE a bounded positive VF offset and run a load dwell. It can TDR",
        "         or reboot the machine and REQUIRES an operator present and able to reboot. It is",
        "         single-step only (one candidate), arms Safe Loop before the write, resets to stock",
        "         on every exit path, and never persists, applies, or promotes a profile.",
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
    /// Dwell completed cleanly.
    Stable,
    /// Instability / silent error under load (no device loss).
    Unstable,
    /// TDR / crash / device-lost during the dwell.
    DeviceLost,
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
    /// Dwell / measure once under load.
    fn dwell(&mut self) -> F2DwellOutcome;
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
    r.dwell = Some(d);
    match d {
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

    fn dwell(&mut self) -> F2DwellOutcome {
        let s = crate::gpu_power_sweep::single_load_dwell();
        let outcome = if s.crashed {
            F2DwellOutcome::DeviceLost
        } else if s.stable {
            F2DwellOutcome::Stable
        } else {
            // silent_error or any non-stable/non-crash → conservatively unstable.
            F2DwellOutcome::Unstable
        };
        info!(
            "undervolt-probe dwell: {outcome:?} avg_clock={} MHz p5={} MHz power={:.0} W silent_error={}",
            s.avg_clock_mhz, s.p5_clock_mhz, s.power_w, s.silent_error
        );
        outcome
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
/// Single-target, single-step only; never persists/applies/promotes a profile.
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
        fn dwell(&mut self) -> F2DwellOutcome { self.log.push("dwell"); self.dwell }
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
}
