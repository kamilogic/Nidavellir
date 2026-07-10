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
//! - a same-target ANCHORED MULTI-STEP descent ([`plan_anchored_undervolt_descent`] +
//!   [`run_confirmed_f2_multi_step`] over the [`F2MultiStepOps`] trait): `--steps N`
//!   executes the requested sequence of anchored candidates at ONE target (safer/higher voltage → lower
//!   voltage), running the SAME per-step motor and STOPPING at the first non-stable candidate; and
//! - an explicit MANUAL-PRIOR development/known-GPU shortcut ([`plan_manual_prior_undervolt`] +
//!   `run_manual_prior_undervolt_probe`, gated by [`confirmed_manual_prior_refusal`]): `--manual-prior`
//!   anchors at an operator-provided `--start-mv` using a SEPARATE larger bounded offset cap
//!   ([`F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ`]) to validate a KNOWN point fast. It is OPT-IN, never
//!   the default, requires `--start-mv`, and is single-step under `--confirm`.
//!
//! Safety invariants: no profile apply/persist/promotion, no MULTI-TARGET automation (the multi-step
//! descent stays on a single target), no autonomous crash-seeking, no power-limit/TDP/clock-lock
//! change; discovery depth is bounded only by the real VF table / requested manual steps and the first
//! terminal detection; the MANUAL-PRIOR larger offset cap is OPT-IN ONLY (it never changes the
//! default/autonomous discovery caps) and still fail-closed (an offset above it is REFUSED, never
//! clamped, and the stock clock ceiling still caps the effective clock); the dry-run reads Safe Loop
//! state READ-ONLY (it never arms the boot flag, mutates the record, applies, dwells, or writes VF);
//! and the confirmed branch never leaves a positive offset applied after exit (a reset that cannot be
//! confirmed fails closed and retains the boot flag for startup recovery).

use std::ffi::OsString;

use nidavellir_core::f2_observation::{
    F2QualificationCoverage, F2QualificationPattern, F2QualificationStrength,
    F2QualificationVerdict,
};
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
use nidavellir_gpu_stress::{RenderGoldens, VfQualifierPattern};
#[cfg(windows)]
use tracing::{info, warn};

#[cfg(windows)]
use crate::gpu_verify::AnchoredOffsetVerification;
use crate::gpu_verify::PositiveOffsetVerification;

/// Default number of descent steps (candidate bins) when `--steps` is omitted. Small by design —
/// F2 v1 is a single focus target with a small bounded raise.
const F2_DEFAULT_STEPS: usize = 4;

/// Hard upper bound on `--validation-passes` for the `--auto-sweep` confidence opt-in. A request above
/// this is REFUSED (fail closed), never clamped silently — extra validations are real hardware time
/// (arm→apply→verify→dwell→reset per pass), so the total is bounded so it can never run unbounded.
#[cfg(windows)]
const F2_MAX_VALIDATION_PASSES: usize = 20;

/// AUTO-SWEEP traverses the complete physical candidate domain. `usize::MAX` is only the planner's
/// "no arbitrary budget" sentinel; the real VF table, hardware floor, and first terminal detection
/// still bound execution.
#[cfg(windows)]
pub(crate) const F2_SWEEP_DRYRUN_BUDGET: usize = usize::MAX;

/// Hard cap (MHz) on the bounded POSITIVE offset for the explicit MANUAL-PRIOR development path ONLY.
/// The default/autonomous discovery keeps the conservative +30 absolute / +15 per-step caps and NEVER
/// sees this value. Manual-prior is an opt-in shortcut to validate a KNOWN manual point (e.g. 1800 MHz
/// @ 875 mV, which on the test GPU needs ~+210 MHz from a 1590 MHz base): the cap is large enough to
/// admit such a known raise yet still a HARD, fail-closed bound — an offset above it is REFUSED, never
/// clamped — and the stock clock ceiling still caps the effective clock so this can never overclock.
const F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ: i32 = 250;

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

/// v13: every F2 dwell runs under an absolute NVML max-clock ceiling at the focus target, so the
/// sustained p95 can never sit above target. One boost bin of slack absorbs clock-counter
/// quantization; beyond it the ceiling did not hold (driver refusal / NVML failure) and the dwell
/// evidence does not describe the labeled point.
#[cfg(windows)]
const F2_CLOCK_CEILING_TOL_MHZ: u32 = 15;

/// An adjacent PowerRender p99 step larger than both limits is remeasured at the exact same bin.
/// The absolute floor catches the observed whole-dwell underload, while the relative limit scales
/// with other board-power classes.
#[cfg(windows)]
const POWER_P99_RECHECK_ABS_W: f32 = 8.0;
#[cfg(windows)]
const POWER_P99_RECHECK_REL: f32 = 0.05;
/// Initial dwell plus at most two reset-clean repeats at the same physical bin.
#[cfg(windows)]
pub(crate) const POWER_P99_MAX_ATTEMPTS: usize = 3;
/// Adjacent p5 values must describe the same sustained-clock regime before power monotonicity is
/// compared. One boost bin mirrors the verifier tolerance.
#[cfg(windows)]
const POWER_P99_EQUIVALENT_P5_TOL_MHZ: u32 = 15;

/// Adaptive discovery may skip only across a confirmed power-bound region. Four physical bins are
/// the largest requested stride, and the actual scheduler additionally enforces this voltage span
/// plus the writer's positive-offset step cap from the last reset-clean candidate.
#[cfg(windows)]
const F2_ADAPTIVE_MAX_STRIDE_BINS: usize = 4;
#[cfg(windows)]
const F2_ADAPTIVE_MAX_VOLTAGE_DROP_MV: u32 = 25;

/// Relative sustained-clock margin allowed between equivalent v8 qualification passes. A
/// candidate whose heavy-phase p5 falls farther than this below the median of prior stable
/// candidates at the same target/pattern has reached the voltage-margin cliff even if it did not
/// crash. This is policy, not a hardware limit.
#[cfg(windows)]
const MARGIN_DROP_TOL_MHZ: u32 = 30;

/// Number of additional attempts after an inconclusive v8 qualification dwell. Coverage
/// ambiguity is not instability: retry the same physical point, then skip only this clock.
#[cfg(windows)]
const INCONCLUSIVE_RETRY_BUDGET: usize = 2;

/// Proven Standard dwell duration. The live Forge modes may select another duration, but the CLI
/// paths keep this baseline unless they opt in explicitly.
#[cfg(windows)]
const F2_STANDARD_DWELL_MS: u64 = 15_000;

/// v14 candidate-only endurance soak: ONE continuous WORST-REALISTIC dwell (~20 min) run only at the
/// exact Apply point for the 3 profile candidates — deliberately harsher than a real game (sustained
/// max-power + cap-slam + droop transients) so a PASS means real games are safe with margin. ~20 min
/// mirrors the continuous Overwatch loop that TDR'd Godforge 1920@918; long enough to reach the
/// thermal saturation the reset-between 5-min patterns never reach.
#[cfg(windows)]
pub(crate) const F2_ENDURANCE_QUALIFICATION_DWELL_MS: u64 = 1_200_000;

/// v15 candidate-only transition shock (~8 min): true-idle → heavy-slam cycles at the exact Apply
/// point, reproducing the game/benchmark-LAUNCH transition behind the observed in-game BusReset TDR
/// cascade. Runs BEFORE the Endurance soak — a launch-fragile point fails in ~8 min instead of
/// wasting the 20-min soak.
#[cfg(windows)]
pub(crate) const F2_TRANSITION_SHOCK_DWELL_MS: u64 = 480_000;

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
/// selects the original single-bin mode. `manual_prior` (`--manual-prior`) opts into the explicit
/// development/known-GPU shortcut (anchored, larger bounded cap; requires `--start-mv`); it defaults
/// to `false` so default/autonomous behavior is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndervoltArgs {
    pub target_mhz: Option<u32>,
    pub start_mv: Option<u32>,
    pub steps: Option<usize>,
    pub mode: UndervoltMode,
    /// Opt-in manual-prior mode (`--manual-prior`). NEVER the default; requires an explicit `--start-mv`.
    pub manual_prior: bool,
    /// Opt-in autonomous same-target sweep (`--auto-sweep`): discover the minimum stable voltage for one
    /// `--target-mhz` via the official progressive anchored descent, recording observations. NEVER the
    /// default; dry-run unless `--confirm`.
    pub auto_sweep: bool,
    /// Opt-in multi-target ladder sweep (`--ladder-sweep`): run an autonomous target sweep for each of
    /// `--targets`, in order, using lower targets' results only as conservative priors. NEVER the default.
    pub ladder_sweep: bool,
    /// Target clocks (MHz) for `--ladder-sweep` (`--targets 1800,1815,1830`). Empty unless provided.
    pub targets: Vec<u32>,
    /// Confidence opt-in for `--auto-sweep` (`--validation-passes N`, default 1): how many TOTAL validated
    /// passes the deepest discovered point gets in one session. 1 = today's behavior (one validation). A
    /// value > 1 re-validates only that single deepest point up to `N-1` additional times (each a full
    /// arm→apply→verify→dwell→reset), accumulating confidence. Bounded by [`F2_MAX_VALIDATION_PASSES`].
    pub validation_passes: usize,
}

impl Default for UndervoltArgs {
    fn default() -> Self {
        Self {
            target_mhz: None,
            start_mv: None,
            steps: None,
            mode: UndervoltMode::default(),
            manual_prior: false,
            auto_sweep: false,
            ladder_sweep: false,
            targets: Vec::new(),
            // Confidence opt-in defaults to ONE validated pass (today's behavior).
            validation_passes: 1,
        }
    }
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
                    Some(v.parse().map_err(|_| format!("--target-mhz: invalid number '{v}'"))?,
                );
                i += 2;
            }
            "--start-mv" => {
                let v = strs.get(i + 1).ok_or_else(|| "--start-mv needs a value".to_string())?;
                out.start_mv =
                    Some(v.parse().map_err(|_| format!("--start-mv: invalid number '{v}'"))?,
                );
                i += 2;
            }
            "--steps" => {
                let v = strs.get(i + 1).ok_or_else(|| "--steps needs a value".to_string())?;
                out.steps = Some(v.parse().map_err(|_| format!("--steps: invalid number '{v}'"))?,
                );
                i += 2;
            }
            "--validation-passes" => {
                let v = strs.get(i + 1).ok_or_else(|| "--validation-passes needs a value".to_string())?;
                out.validation_passes =
                    v.parse().map_err(|_| format!("--validation-passes: invalid number '{v}'"))?;
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
            "--manual-prior" => {
                out.manual_prior = true;
                i += 1;
            }
            "--auto-sweep" => {
                out.auto_sweep = true;
                i += 1;
            }
            "--ladder-sweep" => {
                out.ladder_sweep = true;
                i += 1;
            }
            "--targets" => {
                let v = strs.get(i + 1).ok_or_else(|| "--targets needs a value".to_string())?;
                let mut targets = Vec::new();
                for t in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    targets.push(t.parse().map_err(|_| format!("--targets: invalid number '{t}'"))?,
                    );
                }
                out.targets = targets;
                i += 2;
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
        match plan_bounded_positive_offset(static_base_curve, idx, focus_target_mhz, prev_offset, limits,
        ) {
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

/// Resolve a persisted F2 apply anchor against the live static table. Apply/reapply must use the
/// exact validated VF-table voltage; silently falling to a lower bin would deepen the undervolt
/// beyond the forged point. If the table changed and the exact anchor is unavailable, fail closed.
fn select_exact_apply_anchor_bin(
    static_base_curve: &[(usize, u32, u32)],
    target_mhz: u32,
    anchor_mv: u32,
) -> Option<usize> {
    static_base_curve
        .iter()
        .find(|&&(_, mv, base)| mv == anchor_mv && base < target_mhz)
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
    match plan_bounded_anchored_positive_offset(static_base_curve, anchor_idx, focus_target_mhz, 0, limits,
    )
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

/// The result of planning a MANUAL-PRIOR anchored undervolt point: the operator's explicit prior
/// (`requested_start_mv`), the REAL VF bin it resolved to (at/below the requested mV), that bin's base
/// clock, the required positive offset to reach the target, the manual-prior offset cap, whether the
/// point is within bounds, and the underlying [`AnchoredProbePlan`]. Single-target, single anchored
/// point. A PLAN only — no hardware action. Manual-prior is an explicit development/known-GPU shortcut,
/// NOT the default autonomous discovery path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPriorPlan {
    pub focus_target_mhz: u32,
    /// The voltage the operator asked to anchor at (`--start-mv`).
    pub requested_start_mv: u32,
    /// The real VF bin index selected (highest bin at/below `requested_start_mv` whose base < target).
    pub selected_idx: Option<usize>,
    /// The selected bin's actual voltage (mV) — may differ from `requested_start_mv` if it is not exact.
    pub selected_mv: Option<u32>,
    /// The selected bin's static base clock (MHz).
    pub base_mhz: Option<u32>,
    /// The positive offset the target needs at the selected bin (`target - base`); may exceed the cap.
    pub required_offset_mhz: Option<i32>,
    /// The manual-prior absolute offset cap actually in force (MHz).
    pub manual_cap_mhz: i32,
    /// True iff the underlying anchored plan was produced (offset within cap, bin real/above floor, etc.).
    pub within_bounds: bool,
    /// The underlying anchored plan (raise the anchor + cap the plateau + elastic below), or `None`.
    pub probe: AnchoredProbePlan,
    /// Refusal/explanation when no anchored plan was produced (over cap / below floor / nothing to anchor).
    pub note: Option<String>,
}

/// Pure MANUAL-PRIOR anchored planner: resolve the operator's explicit `requested_start_mv` to the
/// nearest REAL VF bin at/below it whose base is below `focus_target_mhz` (via [`select_anchor_bin`]),
/// then build the bounded anchored curve at that bin using the MANUAL-PRIOR limits (a SEPARATE, larger
/// offset cap — the default/autonomous discovery never sees it). Reuses [`plan_anchored_undervolt`], so
/// it inherits EVERY anchored fail-closed rule (real-bin, hardware floor, clock ceiling, monotone
/// sanity, no positive offset outside the anchor) and only the offset cap differs. Also surfaces the
/// selected bin, its base, and the required offset even when the plan is REFUSED (so the dry-run can
/// report required offset vs cap). Single-target, single anchored point. No hardware — returns a plan.
pub fn plan_manual_prior_undervolt(
    static_base_curve: &[(usize, u32, u32)],
    focus_target_mhz: u32,
    requested_start_mv: u32,
    manual_limits: &PositiveOffsetLimits,
) -> ManualPriorPlan {
    let selected_idx = select_anchor_bin(static_base_curve, focus_target_mhz, Some(requested_start_mv),
    );
    let (selected_mv, base_mhz, required_offset_mhz) = match selected_idx {
        Some(idx) => static_base_curve
            .iter()
            .find(|(i, _, _)| *i == idx)
            .map(|&(_, mv, base)| {
                (Some(mv), Some(base), Some(focus_target_mhz as i32 - base as i32),
                )
            })
            .unwrap_or((None, None, None)),
        None => (None, None, None),
    };
    // Reuse the anchored planner with the MANUAL-PRIOR limits; it fails closed (never clamps) on an
    // offset above the manual cap, a below-floor bin, a non-monotone curve, or a missing anchor.
    let probe = plan_anchored_undervolt(static_base_curve, focus_target_mhz, Some(requested_start_mv), manual_limits,
    );
    let within_bounds = probe.plan.is_some();
    let note = probe.note.clone();
    ManualPriorPlan {
        focus_target_mhz,
        requested_start_mv,
        selected_idx,
        selected_mv,
        base_mhz,
        required_offset_mhz,
        manual_cap_mhz: manual_limits.abs_max_offset_mhz,
        within_bounds,
        probe,
        note,
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

/// The per-step baseline offset for candidate `i` of a CONFIRMED chained same-target descent.
///
/// The confirmed multi-step motor ([`run_confirmed_f2_multi_step`]) reaches candidate `i` ONLY after
/// candidate `i-1` returned `Validated` (it stops at the first non-stable outcome), so the prior
/// candidate's offset is a point freshly validated on THIS hardware THIS run, and is the correct
/// per-step reference for candidate `i`'s single write-from-stock — the planner already chained the
/// candidates so that adjacent offsets differ by at most the per-step cap. Candidate 0 has no prior
/// candidate this run, so it uses the cross-run `baseline_offset` (the deepest prior VALIDATED
/// same-target observation's offset, or `0` when none — unchanged first-run behavior). The ABSOLUTE
/// offset cap still bounds every candidate's absolute offset independently inside the writer; this only
/// moves the per-step reference from stock `+0` to the last validated point. Pure.
pub fn chained_prev_offset(
    candidates: &[AnchoredPositiveOffsetPlan],
    i: usize,
    baseline_offset: i32,
) -> i32 {
    match i.checked_sub(1).and_then(|p| candidates.get(p)) {
        Some(prev) => prev.anchor.offset_mhz,
        None => baseline_offset,
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
        reasons.push("a Safe Loop boot flag is already armed (prior run did not clear) — refuse".to_string(),
        );
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
        out.push("candidate bins     : none (no bin needs a bounded positive raise to hold the target)".to_string(),
        );
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
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock on every exit path".to_string(),
    );
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
    out.push("=== undervolt-probe PLAN (F2 anchored true-undervolt, dry-run preview) ===".to_string(),
    );
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
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock on every exit path".to_string(),
    );
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
        "confirmed span     : {confirmed_cap} planned physical candidate(s); no arbitrary step cap \
         (stops at first terminal detection or the real VF floor)"
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
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock after EVERY candidate".to_string(),
    );
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
            .to_string());
    out
}

/// Format the MANUAL-PRIOR anchored dry-run plan as printable lines (pure + testable). Shows the
/// ANCHORED + MANUAL-PRIOR mode, the explicit manual-prior warning, the target, the requested
/// `--start-mv`, the REAL selected VF bin + its base, the required positive offset, the manual-prior
/// offset cap (and that the default discovery cap is unchanged), the anchored caps/elastic/flatten
/// (when within bounds) or the refusal (required offset vs cap / below floor / nothing to anchor), the
/// Safe Loop preflight, the reset_to_stock requirement, the confirmed single-step requirement, and the
/// explicit no-op (no-write) + no-persist/apply/promote lines.
pub fn manual_prior_plan_lines(
    plan: &ManualPriorPlan,
    manual_limits: &PositiveOffsetLimits,
    preflight: &PreflightVerdict,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("=== undervolt-probe PLAN (F2 ANCHORED + MANUAL-PRIOR, dry-run preview) ===".to_string(),
    );
    out.push(
        "mode               : ANCHORED + MANUAL-PRIOR (explicit operator prior — NOT default discovery)"
            .to_string(),
    );
    out.push(
        "manual-prior       : uses user-provided prior; not the default unknown-GPU discovery path"
            .to_string(),
    );
    out.push(format!(
        "focus target       : {} MHz (single target; manual-prior holds a known clock — never overclocks)",
        plan.focus_target_mhz
    ));
    out.push(format!(
        "requested start-mv : {} mV (explicit; REQUIRED for manual-prior)",
        plan.requested_start_mv
    ));
    match (plan.selected_mv, plan.base_mhz, plan.required_offset_mhz) {
        (Some(mv), Some(base), Some(off)) => {
            out.push(format!(
                "selected anchor    : {mv} mV (real VF bin at/below requested) base {base} MHz"
            ));
            out.push(format!("required offset    : {off:+} MHz (target - base)"));
        }
        _ => {
            out.push(
                "selected anchor    : none — no real VF bin at/below the requested start-mv is below target"
                    .to_string(),
            );
            out.push("required offset    : n/a".to_string());
        }
    }
    out.push(format!(
        "manual offset cap  : +{} MHz (F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ; DEFAULT discovery cap \
         stays +{} — unaffected)",
        manual_limits.abs_max_offset_mhz, nidavellir_gpu_nvapi::POS_OFFSET_MAX_MHZ
    ));
    out.push(format!(
        "voltage floor      : {} mV (the anchor never goes below it)",
        manual_limits.hw_floor_mv
    ));
    out.push(format!(
        "clock ceiling      : {} MHz (a planned clock above it is rejected)",
        manual_limits.clock_ceiling_mhz
    ));
    match &plan.probe.plan {
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
                "higher-voltage bins: {} capped DOWN to target, {} already at/below target (never raised)",
                p.capped_above_bins, p.above_already_ok_bins
            ));
            out.push(format!(
                "lower-voltage bins : {} left elastic (offset 0, never raised)",
                p.elastic_below_bins
            ));
            out.push("within bounds      : YES (required offset within the manual-prior cap)".to_string(),
            );
        }
        None => {
            out.push(format!(
                "within bounds      : NO — {}",
                plan.note.clone().unwrap_or_else(|| "no anchored plan produced".to_string())
            ));
            out.push(
                "refusal            : no write (required offset exceeds the manual-prior cap, or the \
                 bin is invalid / below floor / nothing to anchor) — never clamped"
                    .to_string(),
            );
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
    out.push("reset_to_stock     : a confirmed run MUST reset the GPU to stock on every exit path".to_string(),
    );
    out.push(
        "anchored guarantee : anchored mode prevents boost above the target during this probe"
            .to_string(),
    );
    out.push(
        "confirmed mode     : single-step only (requires --start-mv + --steps 1 + --manual-prior + --confirm)"
            .to_string(),
    );
    out.push("no-op (dry-run)    : no Safe Loop arm, no apply, no dwell, no VF write".to_string());
    out.push("profile            : none persisted, applied, or promoted".to_string());
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
        "  --steps <N>          Explicit ANCHORED candidates to attempt at the same target, descending",
        "                       voltage. There is no hidden step cap; N is the operator's requested",
        "                       boundary. --steps 1 keeps the validated single-step path. --simple uses N as the",
        "                       descent budget (default: 4) and REQUIRES --steps 1 under --confirm.",
        "  --anchored           Plan the anchored classic undervolt point(s) (DEFAULT).",
        "  --simple             Plan the original single-bin positive-offset descent (boost above the",
        "                       target is NOT prevented; for comparison/diagnostics only).",
        "  --manual-prior       Explicit DEVELOPMENT / known-GPU shortcut: anchor at an operator-provided",
        "                       --start-mv with a SEPARATE larger bounded offset cap",
        "                       (F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ). NOT the default and NOT for",
        "                       unknown GPUs; REQUIRES --start-mv; confirmed mode is single-step (--steps",
        "                       1). The default/autonomous discovery cap (+30) is unaffected.",
        "  --auto-sweep         Autonomous same-target sweep: discover the minimum stable voltage for one",
        "                       --target-mhz via the OFFICIAL progressive anchored descent (conservative",
        "                       caps), recording an observation per candidate. Walks every real VF bin",
        "                       until the first terminal detection or physical floor; ignores --steps;",
        "                       NOT manual-prior. Dry-run plans + previews the learned frontier.",
        "  --ladder-sweep       Autonomous MULTI-target sweep over --targets, in order. Lower targets'",
        "                       results are used only as conservative descent FLOORS for higher targets",
        "                       (never assumed to hold the higher clock). Stops the ladder on a safety",
        "                       failure. Dry-run plans + reports the learned frontier + classifier bridge.",
        "  --targets <a,b,..>   Comma-separated target clocks (MHz) for --ladder-sweep.",
        "  --confirm            Execute supervised anchored candidate(s) (see WARNING). Default: dry-run.",
        "  -h, --help           Print this help and exit (no hardware read, no plan, no mutation).",
        "",
        "Explicit --steps runs should start small; autonomous discovery owns its full physical sweep.",
        "Default behavior is autonomous progressive anchored discovery; manual-prior is opt-in only.",
        "",
        "WARNING: --confirm MAY WRITE bounded positive VF offsets and run load dwells. It can TDR or",
        "         reboot the machine and REQUIRES an operator present and able to reboot. Anchored",
        "         autonomous discovery has no arbitrary candidate-count cap at a target;",
        "         manual-prior confirmed mode runs ONE anchored point at the explicit --start-mv. Both",
        "         arm Safe Loop before each write, reset to stock after EVERY candidate, stop at the",
        "         first non-stable candidate, and never persist, apply, or promote a profile.",
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
    /// A known-answer workload detected a silent compute error (no device loss).
    SilentError,
    /// Instability under load without a classified silent compute error (no device loss).
    Unstable,
    /// TDR / crash / device-lost during the dwell.
    DeviceLost,
    /// Dwell did not crash or error, but the sustained (p5) clock sagged below target − tolerance —
    /// the undervolt could not hold the focus clock under load (voltage too low for this clock).
    ClockDrop,
    /// Dwell reset-clean, but the qualification coverage was too weak to accept or reject the point.
    Inconclusive,
}

/// One confirmed dwell's outcome plus its headline measurements. The measurements are carried so the
/// multi-step report can show avg/p5 clock + watts per candidate and so the real dwell can classify a
/// [`F2DwellOutcome::ClockDrop`] from p5. Not used in any `Eq` comparison (carries an `f32`).
#[derive(Debug, Clone, PartialEq)]
pub struct F2DwellResult {
    pub outcome: F2DwellOutcome,
    pub avg_clock_mhz: u32,
    pub p5_clock_mhz: u32,
    pub p95_clock_mhz: u32,
    pub power_w: f32,
    pub max_power_w: f32,
    pub power_p99_w: Option<f32>,
    pub power_capped_frac: f32,
    pub max_temp_c: Option<f32>,
    pub thermal_throttled: bool,
    pub measured_voltage_min_mv: Option<u32>,
    pub measured_voltage_avg_mv: Option<u32>,
    pub measured_voltage_max_mv: Option<u32>,
    pub measured_voltage_sample_count: u32,
    pub render_frames: Option<u64>,
    pub render_fps: Option<f64>,
    pub duration_ms: u64,
    pub sample_count: u32,
    pub qualification_coverage: Option<F2QualificationCoverage>,
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
    /// The dwell reported a silent compute error (no device loss).
    SilentError,
    /// The dwell reported instability without a classified silent error (no device loss).
    Unstable,
    /// The dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// Live-Forge reclassification of a reset-clean pre-sustain `ClockDrop` while still power-bound.
    PowerBoundClockDrop,
    /// The dwell stayed up but the sustained (p5) clock sagged below target − tolerance.
    ClockDrop,
    /// Reset-clean qualification attempt with insufficient coverage; neither good nor bad evidence.
    Inconclusive,
    /// `reset_to_stock` could not be confirmed — boot flag RETAINED, fail closed.
    ResetFailed,
    /// Dwell stable, reset confirmed, boot flag cleared.
    Validated,
}

/// Decision made by the live F2 discovery loop after one reset-clean candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F2DiscoveryDecision {
    /// Try the next lower real voltage bin for this same target.
    ContinueVoltage,
    /// This target held for the first time; remember it as sustainable and continue descending.
    MarkSustainableAndContinue,
    /// This target never held and is no longer power-bound, so move to the next lower real clock.
    NextClockUnsustainable,
    /// The target had already held and this candidate found its lower-voltage boundary.
    BoundaryFound,
    /// A normal terminal failure occurred before this target ever held.
    NextClockAfterFailure,
    /// Hardware/recovery state is not trustworthy; abort the whole forge.
    AbortForge,
}

/// True when the dwell was effectively at the board power limit. The direct power ratio is the
/// primary signal requested by F2 (99–100% of the cap); the sampled cap flag is only a fallback when
/// the driver did not expose a usable numeric limit.
pub fn f2_near_power_limit(
    power_p99_w: Option<f32>,
    power_limit_w: Option<f32>,
    power_capped_frac: Option<f32>,
) -> bool {
    let Some(power_p99) = power_p99_w.filter(|power| power.is_finite() && *power > 0.0) else {
        return false;
    };
    match power_limit_w {
        Some(limit) if limit.is_finite() && limit > 0.0 => {
            power_p99 / limit >= 0.99
        }
        _ => power_capped_frac.is_some_and(|f| f.is_finite() && f >= 0.5),
    }
}

#[cfg(windows)]
fn f2_power_p99_pair_consistent(a: f32, b: f32) -> bool {
    let tolerance = POWER_P99_RECHECK_ABS_W.max(a.max(b) * POWER_P99_RECHECK_REL);
    (a - b).abs() <= tolerance
}

#[cfg(windows)]
fn f2_power_p99_requires_recheck(
    previous: Option<(f32, u32)>,
    report: &F2StepReport,
) -> bool {
    if !f2_power_measurement_usable(report) {
        return false;
    }
    let Some((previous_p99, previous_p5)) = previous else {
        return false;
    };
    let (Some(current_p99), Some(current_p5)) = (report.power_p99_w, report.p5_clock_mhz) else {
        return false;
    };
    previous_p99.is_finite()
        && previous_p99 > 0.0
        && current_p99.is_finite()
        && current_p99 > 0.0
        && previous_p5.abs_diff(current_p5) <= POWER_P99_EQUIVALENT_P5_TOL_MHZ
        && !f2_power_p99_pair_consistent(previous_p99, current_p99)
}

#[cfg(windows)]
fn f2_power_measurement_usable(report: &F2StepReport) -> bool {
    matches!(report.outcome, F2Outcome::Validated | F2Outcome::ClockDrop)
        && report.reset_ok == Some(true)
        && !report.thermal_throttled
        && report
            .power_p99_w
            .is_some_and(|power| power.is_finite() && power > 0.0)
}

#[cfg(windows)]
fn f2_power_attempts_have_consistent_pair(reports: &[F2StepReport]) -> bool {
    let powers: Vec<f32> = reports
        .iter()
        .filter(|report| f2_power_measurement_usable(report))
        .filter_map(|report| report.power_p99_w)
        .collect();
    powers.iter().enumerate().any(|(i, a)| {
        powers
            .iter()
            .skip(i + 1)
            .any(|b| f2_power_p99_pair_consistent(*a, *b))
    })
}

/// Confirm a normal single dwell, or require a consistent pair after an anomalous adjacent-bin
/// step. The returned power is deliberately the highest measured p99 in the accepted group: neither
/// interpolation nor a synthetic monotonic correction is allowed.
#[cfg(windows)]
fn f2_confirm_power_attempts(reports: &mut [F2StepReport], rechecked: bool) -> Option<f32> {
    let hard_failure = reports.iter().any(|report| {
        matches!(
            report.outcome,
            F2Outcome::DeviceLost
                | F2Outcome::ResetFailed
                | F2Outcome::ArmFailed(_)
                | F2Outcome::ApplyFailed(_)
                | F2Outcome::VerifyFailed
                | F2Outcome::SilentError
                | F2Outcome::Unstable
        )
    });
    let usable: Vec<(usize, f32)> = reports
        .iter()
        .enumerate()
        .filter(|(_, report)| f2_power_measurement_usable(report))
        .filter_map(|(index, report)| report.power_p99_w.map(|power| (index, power)))
        .collect();
    let consensus = if hard_failure {
        false
    } else if rechecked {
        f2_power_attempts_have_consistent_pair(reports)
    } else {
        usable.len() == 1
    };
    let attempt_count = reports.len() as u32;
    if !consensus {
        for report in reports.iter_mut().filter(|report| f2_power_measurement_usable(report)) {
            report.outcome = F2Outcome::Inconclusive;
            report.power_p99_confirmed = false;
            report.power_p99_attempts = attempt_count;
        }
        return None;
    }
    let conservative_p99 = usable
        .iter()
        .map(|(_, power)| *power)
        .fold(f32::NEG_INFINITY, f32::max);
    for (index, _) in usable {
        reports[index].power_p99_confirmed = true;
        reports[index].power_p99_attempts = attempt_count;
    }
    Some(conservative_p99)
}

#[cfg(windows)]
fn f2_aggregate_power_attempts(
    reports: &[F2StepReport],
    conservative_p99: Option<f32>,
) -> F2StepReport {
    let mut aggregate = reports[0].clone();
    if let Some(hard) = reports.iter().find(|report| {
        matches!(
            report.outcome,
            F2Outcome::DeviceLost
                | F2Outcome::ResetFailed
                | F2Outcome::ArmFailed(_)
                | F2Outcome::ApplyFailed(_)
                | F2Outcome::VerifyFailed
                | F2Outcome::SilentError
                | F2Outcome::Unstable
        )
    }) {
        aggregate.outcome = hard.outcome.clone();
    } else if conservative_p99.is_none() {
        aggregate.outcome = F2Outcome::Inconclusive;
    } else if reports
        .iter()
        .any(|report| matches!(report.outcome, F2Outcome::ClockDrop))
    {
        aggregate.outcome = F2Outcome::ClockDrop;
    } else {
        aggregate.outcome = F2Outcome::Validated;
    }
    aggregate.power_p99_w = conservative_p99;
    aggregate.power_p99_confirmed = conservative_p99.is_some();
    aggregate.power_p99_attempts = reports.len() as u32;
    aggregate.p5_clock_mhz = reports.iter().filter_map(|r| r.p5_clock_mhz).min();
    aggregate.avg_clock_mhz = reports.iter().filter_map(|r| r.avg_clock_mhz).min();
    aggregate.power_w = reports.iter().filter_map(|r| r.power_w).max();
    aggregate.max_power_w = reports.iter().filter_map(|r| r.max_power_w).max();
    aggregate.power_capped_frac = reports
        .iter()
        .filter_map(|r| r.power_capped_frac)
        .reduce(f32::max);
    aggregate.max_temp_c = reports.iter().filter_map(|r| r.max_temp_c).reduce(f32::max);
    aggregate.thermal_throttled = reports.iter().any(|r| r.thermal_throttled);
    aggregate
}

fn f2_power_bound_clock_drop(outcome: &F2Outcome, near_power_limit: bool) -> F2Outcome {
    if matches!(outcome, F2Outcome::ClockDrop) && near_power_limit {
        F2Outcome::PowerBoundClockDrop
    } else {
        outcome.clone()
    }
}

/// Pure F2 transition matrix. A pre-sustain `ClockDrop` is not automatically a voltage boundary:
/// while the card remains at 99–100% of its power cap, lowering voltage may free enough headroom for
/// that same target to become sustainable. Once off-cap, the same drop means the target is not viable.
/// After a target has held once, the first drop/error is its discovered boundary.
pub fn f2_discovery_decision(
    outcome: &F2Outcome,
    had_sustainable_point: bool,
    near_power_limit: bool,
) -> F2DiscoveryDecision {
    match outcome {
        F2Outcome::Validated if had_sustainable_point => F2DiscoveryDecision::ContinueVoltage,
        F2Outcome::Validated => F2DiscoveryDecision::MarkSustainableAndContinue,
        F2Outcome::PowerBoundClockDrop => F2DiscoveryDecision::ContinueVoltage,
        F2Outcome::ClockDrop if near_power_limit => F2DiscoveryDecision::ContinueVoltage,
        F2Outcome::ClockDrop if had_sustainable_point => F2DiscoveryDecision::BoundaryFound,
        F2Outcome::ClockDrop => F2DiscoveryDecision::NextClockUnsustainable,
        F2Outcome::SilentError | F2Outcome::Unstable if had_sustainable_point => {
            F2DiscoveryDecision::BoundaryFound
        }
        F2Outcome::SilentError | F2Outcome::Unstable => F2DiscoveryDecision::NextClockAfterFailure,
        F2Outcome::Inconclusive => F2DiscoveryDecision::NextClockAfterFailure,
        F2Outcome::DeviceLost
        | F2Outcome::ResetFailed
        | F2Outcome::ArmFailed(_)
        | F2Outcome::ApplyFailed(_)
        | F2Outcome::VerifyFailed => F2DiscoveryDecision::AbortForge,
    }
}

#[cfg(windows)]
fn f2_adaptive_power_bound_next_index(
    candidates: &[AnchoredPositiveOffsetPlan],
    current_index: usize,
    target_mhz: u32,
    p5_clock_mhz: Option<u32>,
    reference_offset_mhz: i32,
    step_max_offset_mhz: i32,
) -> usize {
    let deficit_mhz = target_mhz.saturating_sub(p5_clock_mhz.unwrap_or(target_mhz));
    let requested_stride = if deficit_mhz >= 90 {
        F2_ADAPTIVE_MAX_STRIDE_BINS
    } else if deficit_mhz >= 45 {
        2
    } else {
        1
    };
    let Some(current) = candidates.get(current_index) else {
        return current_index;
    };
    for stride in (1..=requested_stride).rev() {
        let next_index = current_index.saturating_add(stride);
        let Some(next) = candidates.get(next_index) else {
            continue;
        };
        let voltage_drop_mv = current
            .anchor
            .voltage_mv
            .saturating_sub(next.anchor.voltage_mv);
        let offset_step_mhz = next.anchor.offset_mhz.saturating_sub(reference_offset_mhz);
        if voltage_drop_mv <= F2_ADAPTIVE_MAX_VOLTAGE_DROP_MV
            && offset_step_mhz <= step_max_offset_mhz
        {
            return next_index;
        }
    }
    current_index.saturating_add(1).min(candidates.len())
}

#[cfg(windows)]
fn f2_recovery_midpoint(shallower_safe_index: usize, deeper_failed_index: usize) -> Option<usize> {
    let gap = deeper_failed_index.saturating_sub(shallower_safe_index);
    (gap > 1).then_some(shallower_safe_index + gap / 2)
}

#[cfg(windows)]
fn f2_reset_clean_discovery_failure(
    decision: F2DiscoveryDecision,
    report: &F2StepReport,
) -> bool {
    report.reset_ok == Some(true)
        && report.boot_flag_cleared
        && matches!(
            decision,
            F2DiscoveryDecision::NextClockUnsustainable
                | F2DiscoveryDecision::BoundaryFound
                | F2DiscoveryDecision::NextClockAfterFailure
        )
}

fn f2_outcome_retains_boot_flag(outcome: &F2Outcome) -> bool {
    matches!(outcome, F2Outcome::DeviceLost | F2Outcome::ResetFailed)
}

fn resume_f2_candidates(
    candidates: &mut Vec<AnchoredPositiveOffsetPlan>,
    prior_good_mv: Option<u32>,
    prior_bad_mv: Option<u32>,
    prior_power_bound_mv: Option<u32>,
) -> Option<u32> {
    if let Some(bad_mv) = prior_bad_mv {
        if prior_good_mv.is_some() {
            candidates.clear();
            return None;
        }
        // A failed warm-start with no validated point does not prove the clock unsustainable.
        // Retry the still-unknown higher-voltage candidates while never touching the failed/deeper
        // region again.
        candidates.retain(|candidate| candidate.anchor.voltage_mv > bad_mv);
        return Some(bad_mv);
    }
    let resume_below = prior_good_mv.into_iter().chain(prior_power_bound_mv).min();
    if let Some(mv) = resume_below {
        candidates.retain(|candidate| candidate.anchor.voltage_mv < mv);
    }
    resume_below
}

fn f2_observation_matches_current_candidate(
    candidates: &[AnchoredPositiveOffsetPlan],
    anchor_mv: u32,
    base_mhz: u32,
    offset_mhz: i32,
) -> bool {
    candidates.iter().any(|candidate| {
        candidate.anchor.voltage_mv == anchor_mv
            && candidate.anchor.base_mhz == base_mhz
            && candidate.anchor.offset_mhz == offset_mhz
    })
}

/// Structured report of a confirmed single step (also drives the printed output and the tests).
#[derive(Debug, Clone, PartialEq)]
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
    pub p95_clock_mhz: Option<u32>,
    pub power_w: Option<u32>,
    pub max_power_w: Option<u32>,
    pub power_p99_w: Option<f32>,
    /// True only after discovery's v4 p99 consistency gate accepts this measurement group.
    pub power_p99_confirmed: bool,
    pub power_p99_attempts: u32,
    pub power_capped_frac: Option<f32>,
    pub max_temp_c: Option<f32>,
    pub thermal_throttled: bool,
    pub measured_voltage_min_mv: Option<u32>,
    pub measured_voltage_avg_mv: Option<u32>,
    pub measured_voltage_max_mv: Option<u32>,
    pub measured_voltage_sample_count: u32,
    pub render_frames: Option<u64>,
    pub render_fps: Option<f64>,
    pub dwell_duration_ms: Option<u64>,
    pub sample_count: Option<u32>,
    pub qualification_coverage: Option<F2QualificationCoverage>,
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
    /// Record the failed point in the Safe Loop blacklist. A device loss retains the boot flag so
    /// startup recovery can account it exactly once; reset-clean instability is boundary knowledge.
    fn blacklist_point(&mut self, counts_as_crash: bool) -> Result<(), String>;
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
        p95_clock_mhz: None,
        power_w: None,
        max_power_w: None,
        power_p99_w: None,
        power_p99_confirmed: false,
        power_p99_attempts: 0,
        power_capped_frac: None,
        max_temp_c: None,
        thermal_throttled: false,
        measured_voltage_min_mv: None,
        measured_voltage_avg_mv: None,
        measured_voltage_max_mv: None,
        measured_voltage_sample_count: 0,
        render_frames: None,
        render_fps: None,
        dwell_duration_ms: None,
        sample_count: None,
        qualification_coverage: None,
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
    r.p95_clock_mhz = Some(d.p95_clock_mhz);
    r.power_w = Some(d.power_w.round() as u32);
    r.max_power_w = Some(d.max_power_w.round() as u32);
    r.power_p99_w = d.power_p99_w;
    r.power_capped_frac = Some(d.power_capped_frac);
    r.max_temp_c = d.max_temp_c;
    r.thermal_throttled = d.thermal_throttled;
    r.measured_voltage_min_mv = d.measured_voltage_min_mv;
    r.measured_voltage_avg_mv = d.measured_voltage_avg_mv;
    r.measured_voltage_max_mv = d.measured_voltage_max_mv;
    r.measured_voltage_sample_count = d.measured_voltage_sample_count;
    r.render_frames = d.render_frames;
    r.render_fps = d.render_fps;
    r.dwell_duration_ms = Some(d.duration_ms);
    r.sample_count = Some(d.sample_count);
    r.qualification_coverage = d.qualification_coverage.clone();
    match d.outcome {
        F2DwellOutcome::DeviceLost => {
            // Crash / TDR: best-effort reset, record the blacklist, and RETAIN the boot flag — a
            // reboot may be imminent, so startup recovery must still fire. Never validate.
            r.reset_ok = Some(ops.reset_to_stock().is_ok());
            r.blacklisted = ops.blacklist_point(true).is_ok();
            r.outcome = F2Outcome::DeviceLost;
            r
        }
        F2DwellOutcome::SilentError => {
            r.outcome = F2Outcome::SilentError;
            finish_after_write(ops, r, true)
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
        F2DwellOutcome::Inconclusive => {
            r.outcome = F2Outcome::Inconclusive;
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
        r.blacklisted = ops.blacklist_point(false).is_ok();
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

// ── F2 confirmed ANCHORED multi-step descent: same-target orchestration ─────────────────────────
// The single-step state machine above proves ONE anchored point. The multi-step descent
// executes the planned sequence of anchored candidates at the SAME target, from safer/higher voltage to
// lower voltage, running the SAME validated per-candidate motor ([`run_confirmed_f2_step`]) for each.
// It STOPS at the first non-stable candidate and never attempts a deeper (lower-voltage) candidate
// after any non-stable result. Single-target and anchored-only;
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
    SilentError,
    /// A candidate's dwell reported instability without a classified silent error.
    Unstable,
    /// A candidate's dwell reported a crash / TDR / device loss.
    DeviceLost,
    /// A candidate held but its sustained (p5) clock sagged below target − tolerance.
    ClockDrop,
    /// A candidate reset cleanly, but qualification coverage was too weak to accept/reject it.
    Inconclusive,
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
#[derive(Debug, Clone, PartialEq)]
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
            F2Outcome::SilentError => {
                stop_reason = F2MultiStopReason::SilentError;
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
            F2Outcome::PowerBoundClockDrop => {
                // Produced only by the integrated live-Forge policy after the single-step motor.
                // If supplied to this legacy bounded runner, continue to the next voltage candidate.
            }
            F2Outcome::ClockDrop => {
                stop_reason = F2MultiStopReason::ClockDrop;
                break;
            }
            F2Outcome::Inconclusive => {
                stop_reason = F2MultiStopReason::Inconclusive;
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

/// CONFIDENCE OPT-IN: re-validate the SINGLE deepest already-validated candidate (`deepest_index`) up to
/// `extra_passes` additional times, using the SAME validated per-candidate motor ([`run_confirmed_f2_step`])
/// — each pass is a full select → arm → apply → verify → dwell → reset → clear (reset after EVERY pass,
/// identical safety to the normal descent). STOPS immediately on ANY non-`Validated` / safety failure (a
/// point that just failed is never hammered again). Returns ONE report per executed extra pass (in order),
/// so the caller can record ONE observation per pass (accumulating `validations_at_best`). `extra_passes`
/// is the caller-clamped, BOUNDED number of EXTRA re-validations (`validation_passes - 1`); 0 ⇒ no-op,
/// preserving today's single-validation behavior exactly. NEVER persists/applies/promotes a profile.
pub fn run_confirmed_f2_extra_validations<O: F2MultiStepOps>(
    ops: &mut O,
    deepest_index: usize,
    extra_passes: usize,
) -> Vec<F2StepReport> {
    let mut reports: Vec<F2StepReport> = Vec::with_capacity(extra_passes);
    for _ in 0..extra_passes {
        // Per-pass Safe Loop + blacklist precheck BEFORE arming / writing (refusal stops further passes).
        if ops.select(deepest_index).is_err() {
            break;
        }
        let report = run_confirmed_f2_step(ops);
        let validated = matches!(report.outcome, F2Outcome::Validated);
        reports.push(report);
        // Stop the moment a re-validation is anything other than a clean Validated pass.
        if !validated {
            break;
        }
    }
    reports
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
fn candidate_blacklisted(record: &SafeLoopRecord, target_mhz: u32, cand: &PositiveOffsetPlan,
) -> bool {
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
        return Some("a Safe Loop boot flag is already armed (prior run did not clear)".to_string(),
        );
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

/// Pure confirmed MANUAL-PRIOR preflight. Returns `Some(reason)` to REFUSE (fail closed) before any
/// hardware, or `None` to proceed. Manual-prior is opt-in and single-step: it REQUIRES an explicit
/// `--start-mv`, then delegates to [`confirmed_f2_refusal`] with the MANUAL-PRIOR limits — so it
/// inherits the `--steps 1` single-step gate, the Safe Mode / armed-flag / crash-threshold gates, the
/// candidate-present check, the defensive offset/floor/clock bound re-checks (against the larger
/// manual-prior cap), and the blacklist check. It NEVER relaxes any of those — only the offset cap
/// differs from default discovery.
pub fn confirmed_manual_prior_refusal(
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
    start_mv: Option<u32>,
    steps: Option<usize>,
    candidate: Option<&PositiveOffsetPlan>,
    manual_limits: &PositiveOffsetLimits,
    target_mhz: u32,
) -> Option<String> {
    if start_mv.is_none() {
        return Some("manual-prior requires an explicit --start-mv".to_string());
    }
    confirmed_f2_refusal(record, boot_flag_armed, steps, candidate, manual_limits, target_mhz,
    )
}

/// Pure confirmed ANCHORED multi-step preflight (run-level gate). Returns `Some(reason)` to REFUSE
/// (fail closed) before any hardware, or `None` to proceed. Refuses unless: `--steps` is present and
/// within `1..=cap` (a larger request fails closed — the confirmed branch enforces its OWN cap
/// regardless of what the dry-run may preview); not in Safe Mode; no boot flag already armed;
/// and at least one anchored candidate exists. Actual crash/TDR recovery is represented by Safe Mode
/// or an armed boot flag; reset-clean instability boundaries must not poison later clocks in the same
/// supervised run. The
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
        return Some("a Safe Loop boot flag is already armed (prior run did not clear)".to_string(),
        );
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum F2StressPurpose {
    PowerDiscovery,
    V8Qualification(F2QualificationPattern, RenderGoldens),
    ApplyQualification(F2QualificationPattern, RenderGoldens),
}

#[cfg(windows)]
impl F2StressPurpose {
    fn is_qualification(self) -> bool {
        !matches!(self, F2StressPurpose::PowerDiscovery)
    }

    fn qualifier_pattern(self) -> Option<VfQualifierPattern> {
        match self {
            F2StressPurpose::PowerDiscovery => None,
            F2StressPurpose::V8Qualification(F2QualificationPattern::A, _) => {
                Some(VfQualifierPattern::Fsgl3A)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::B, _) => {
                Some(VfQualifierPattern::Fsgl3B)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::HighFps, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::HighFps, _) => {
                Some(VfQualifierPattern::V8HighFps)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::Texture, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::Texture, _) => {
                Some(VfQualifierPattern::V8Texture)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::Transitions, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::Transitions, _) => {
                Some(VfQualifierPattern::V8Transitions)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::Memory, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::Memory, _) => {
                Some(VfQualifierPattern::V8Memory)
            }
            F2StressPurpose::ApplyQualification(F2QualificationPattern::A, _) => {
                Some(VfQualifierPattern::Fsgl3A)
            }
            F2StressPurpose::ApplyQualification(F2QualificationPattern::B, _) => {
                Some(VfQualifierPattern::Fsgl3B)
            }
            // v14/v15 candidate-only gates are exact-Apply only; the V8Qualification arms never
            // fire in practice but are mapped so the match stays exhaustive.
            F2StressPurpose::V8Qualification(F2QualificationPattern::Endurance, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::Endurance, _) => {
                Some(VfQualifierPattern::Endurance)
            }
            F2StressPurpose::V8Qualification(F2QualificationPattern::TransitionShock, _)
            | F2StressPurpose::ApplyQualification(F2QualificationPattern::TransitionShock, _) => {
                Some(VfQualifierPattern::TransitionShock)
            }
        }
    }

    fn render_goldens(self) -> Option<RenderGoldens> {
        match self {
            F2StressPurpose::V8Qualification(_, goldens) => Some(goldens),
            F2StressPurpose::ApplyQualification(_, goldens) => Some(goldens),
            F2StressPurpose::PowerDiscovery => None,
        }
    }
}

#[cfg(windows)]
fn classify_f2_stress_dwell(
    s: &crate::gpu_power_sweep::SingleDwell,
    target_mhz: u32,
    purpose: F2StressPurpose,
) -> F2DwellOutcome {
    if s.cancelled {
        F2DwellOutcome::Inconclusive
    } else if s.crashed {
        F2DwellOutcome::DeviceLost
    } else if s.silent_error {
        F2DwellOutcome::SilentError
    } else if !s.stable {
        F2DwellOutcome::Unstable
    } else if s.p95_clock_mhz > target_mhz + F2_CLOCK_CEILING_TOL_MHZ {
        // v13: the dwell ran under an absolute NVML max-clock ceiling at `target`, so a sustained
        // p95 above target means the ceiling did not hold (driver refusal / silent NVML failure).
        // The evidence describes a different (higher) point than the label — never Stable and never
        // boundary knowledge. The GPU itself did nothing wrong, so this is Inconclusive, not Unstable.
        F2DwellOutcome::Inconclusive
    } else if purpose == F2StressPurpose::PowerDiscovery && s.thermal_throttled {
        // Thermal slowdown corrupts the V↔W power calibration regardless of clock (a throttled
        // sample draws less than the point's real steady-state power), so discovery evidence is
        // inconclusive whenever the card thermally slowed.
        F2DwellOutcome::Inconclusive
    } else if matches!(
        purpose,
        // v15: TransitionShock is EXEMPT from the p5-sag thermal disqualifier — its dwell is
        // deliberately ~60% true-idle (10-30 s gaps), so p5 is an idle clock BY DESIGN and says
        // nothing about whether the card backed off the qualified point. Any NVML throttle flag
        // (routine at ~70 °C during exact-Apply) would otherwise misclassify EVERY shock dwell as
        // Inconclusive and refuse the candidate at the end of a full run. The shock carries its
        // own detectors instead: the post-idle slam wall-time stall (Unstable) + golden checksum
        // (SilentError). All continuous patterns keep the held-clock rule below unchanged.
        F2StressPurpose::ApplyQualification(pattern, _)
            if pattern != F2QualificationPattern::TransitionShock
    ) && s.thermal_throttled
        && s.p5_clock_mhz + F2_CLOCK_DROP_TOL_MHZ < target_mhz
    {
        // Exact-Apply qualification: a thermal-slowdown flag only invalidates the proof when the
        // slowdown actually backed the card OFF the qualified point — i.e. the sustained clock (p5)
        // sagged below target beyond tolerance. When the card HELD >= target despite the flag (e.g.
        // a momentary memory-junction hotspot at a cool core temp), the hard VF point was still
        // exercised, so the throttle is not disqualifying and the dwell falls through to the normal
        // coverage/stability verdict. Power discovery keeps the stricter rule above.
        F2DwellOutcome::Inconclusive
    } else if purpose == F2StressPurpose::PowerDiscovery && s.power_p99_w.is_none() {
        // Discovery cannot make a power-bound decision or calibrate an applied bin without p99.
        F2DwellOutcome::Inconclusive
    } else if purpose.is_qualification()
        && s.qualification_coverage
            .as_ref()
            .is_some_and(|coverage| coverage.verdict == F2QualificationVerdict::Inconclusive)
    {
        F2DwellOutcome::Inconclusive
    } else if purpose.is_qualification() && s.prehang_stall_detected {
        // The sensor sampler starved mid-dwell (the pre-hang signature recorded since v6, now a
        // verdict): the GPU stopped answering while under the qualifier. The bin is bad — fail
        // it here instead of letting a deeper bin reach the driver TDR watchdog.
        F2DwellOutcome::Unstable
    } else if purpose == F2StressPurpose::PowerDiscovery
        && s.p5_clock_mhz + F2_CLOCK_DROP_TOL_MHZ < target_mhz
    {
        F2DwellOutcome::ClockDrop
    } else {
        F2DwellOutcome::Stable
    }
}

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
    /// The per-step reference offset for THIS candidate's bounded write (the last validated offset, or 0
    /// for a fresh single-step). The writer enforces the per-step cap on `offset - prev_offset_mhz` and
    /// the absolute cap on `offset` regardless. A single-step / manual-prior run always passes 0; only
    /// the confirmed chained target-sweep motor advances it (see [`chained_prev_offset`]).
    prev_offset_mhz: i32,
    dwell_ms: u64,
    stress_purpose: F2StressPurpose,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
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
        // The per-step cap is enforced on `offset - prev_offset_mhz` (the last validated point, or 0 for
        // a single-step run); the absolute cap is enforced on `offset` regardless. Each writer
        // re-validates every bound and fails closed.
        let prev = self.prev_offset_mhz;
        match (self.mode, &self.anchored) {
            (UndervoltMode::Anchored, Some(_)) => {
                // Anchored: write the full curve (anchor raise + plateau caps + elastic zeros). The
                // writer refuses any positive offset outside the anchor.
                nidavellir_gpu_nvapi::apply_bounded_anchored_positive_offset(
                    &self.curve,
                    self.candidate.index,
                    self.target_mhz,
                    prev,
                    &self.limits,
                )
                .map(|_| ())?
            }
            _ => nidavellir_gpu_nvapi::apply_bounded_positive_offset(
                &self.curve,
                self.candidate.index,
                self.target_mhz,
                prev,
                &self.limits,
            )
            .map(|_| ())?,
        }
        // v13: absolute clock ceiling at the focus target — the VF-curve plateau caps are offsets
        // relative to a base curve the driver shifts with temperature, so only an NVML locked-clocks
        // max makes the measured point BE the labeled point (p95 == target). Failure fails the apply
        // closed: the step motor resets, and the shared reset releases the ceiling.
        nidavellir_core::nvml_gpu::lock_core_clock_max_mhz(self.target_mhz)
            .map_err(|e| format!("v13 clock ceiling ({} MHz) failed after VF write: {e}", self.target_mhz))
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
                        (e.index, nidavellir_gpu_nvapi::vf_get_point_khz(e.index).map(|khz| khz / 1000),
                        )
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
        let s = match self.stress_purpose {
            F2StressPurpose::PowerDiscovery => {
                crate::gpu_power_sweep::single_load_dwell_with_cancel(
                    self.dwell_ms,
                    self.cancel,
                )
            }
            purpose => {
                crate::gpu_power_sweep::single_qualifier_dwell_with_cancel(
                    self.dwell_ms,
                    self.target_mhz,
                    purpose
                        .qualifier_pattern()
                        .expect("qualification purpose has a pattern"),
                    purpose
                        .render_goldens()
                        .expect("v8 qualification purpose has stock goldens"),
                    self.cancel,
                )
            }
        };
        // Mixed qualification telemetry is intentionally excluded from ClockDrop classification:
        // its light phases would make aggregate p5 unsuitable as a sustained-clock boundary.
        let outcome = classify_f2_stress_dwell(&s, self.target_mhz, self.stress_purpose);
        if s.prehang_stall_detected {
            warn!(
                "undervolt-probe dwell: pre-hang telemetry observed an NVML valid-sample stall >= {} ms; reset action remains disabled pending hardware calibration",
                crate::gpu_power_sweep::PREHANG_STALL_MS
            );
        }
        info!(
            "undervolt-probe dwell: {outcome:?} avg_clock={} MHz p5={} MHz p95={} MHz \
             power_avg={:.0} W power_p99={:?} W power_peak={:.0} W max_temp={:?} C \
             thermal_throttled={} silent_error={}",
            s.avg_clock_mhz,
            s.p5_clock_mhz,
            s.p95_clock_mhz,
            s.power_w,
            s.power_p99_w,
            s.max_power_w,
            s.max_temp_c,
            s.thermal_throttled,
            s.silent_error
        );
        F2DwellResult {
            outcome,
            avg_clock_mhz: s.avg_clock_mhz,
            p5_clock_mhz: s.p5_clock_mhz,
            p95_clock_mhz: s.p95_clock_mhz,
            power_w: s.power_w,
            max_power_w: s.max_power_w,
            power_p99_w: s.power_p99_w,
            power_capped_frac: s.power_capped_frac,
            max_temp_c: s.max_temp_c,
            thermal_throttled: s.thermal_throttled,
            measured_voltage_min_mv: s.volt_min_mv,
            measured_voltage_avg_mv: s.volt_avg_mv,
            measured_voltage_max_mv: s.volt_max_mv,
            measured_voltage_sample_count: s.volt_sample_count,
            render_frames: s.render_frames,
            render_fps: s.render_fps,
            duration_ms: s.duration_ms,
            sample_count: s.sample_count,
            qualification_coverage: s.qualification_coverage,
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
                Some(khz) => {
                    return Err(format!("reset readback offset {khz} kHz not cleared at idx {idx}"))
                }
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

    fn blacklist_point(&mut self, _counts_as_crash: bool) -> Result<(), String> {
        let mut rec = self.store.load_record();
        let intent = f2_intent(self.target_mhz, &self.candidate);
        if !rec.is_blacklisted(&intent) {
            rec.blacklist
                .push(BlacklistRegion::around(intent, DEFAULT_BLACKLIST_RADIUS));
        }
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
    /// The cross-run resume baseline offset for candidate 0 (the deepest prior VALIDATED same-target
    /// observation's offset, or 0 when none). Candidate `i>0` chains off candidate `i-1` instead (it is
    /// only reached after `i-1` validated). See [`chained_prev_offset`].
    baseline_offset_mhz: i32,
    /// Live adaptive discovery can select a non-adjacent candidate. In that case the writer must be
    /// bounded against the last candidate that actually completed reset-clean, never the skipped
    /// plan entry. Other multi-step callers keep the original adjacent chaining.
    prev_offset_override_mhz: Option<i32>,
    dwell_ms: u64,
    stress_purpose: F2StressPurpose,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
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
    fn blacklist_point(&mut self, counts_as_crash: bool) -> Result<(), String> {
        self.cur.as_mut().expect("select before use").blacklist_point(counts_as_crash)
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
            return Err("a Safe Loop boot flag is already armed before candidate write".to_string(),
            );
        }
        if candidate_blacklisted(&rec, self.target_mhz, &plan.anchor) {
            return Err(format!("candidate {i} intent is blacklisted"));
        }
        // Ordinary chained descent uses candidate i-1. Adaptive live discovery supplies the offset
        // of the last candidate that actually completed reset-clean, so skipped plan entries can
        // never relax the per-step writer bound. The absolute cap still applies independently.
        let prev_offset_mhz = self.prev_offset_override_mhz.unwrap_or_else(|| {
            chained_prev_offset(&self.candidates, i, self.baseline_offset_mhz)
        });
        let anchor = plan.anchor;
        self.cur = Some(RealF2Ops {
            store: self.store,
            curve: self.curve.clone(),
            candidate: anchor,
            anchored: Some(plan),
            mode: UndervoltMode::Anchored,
            limits: self.limits,
            target_mhz: self.target_mhz,
            prev_offset_mhz,
            dwell_ms: self.dwell_ms,
            stress_purpose: self.stress_purpose,
            cancel: self.cancel,
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
    let min_base = sane.iter().map(|&(_, _, f)| f).min().unwrap();
    let live_curve = gpu::read_vf_curve_modern();
    let boost_top = match crate::gpu_power_sweep::f2_stock_clock_ceiling(&live_curve) {
        Ok(clock) => clock,
        Err(e) => {
            println!(
                "undervolt-probe: stock clock domain unavailable ({e}) — fail closed (no hardware touched)."
            );
            return;
        }
    };
    let focus_target = args.target_mhz.unwrap_or(boost_top);
    let max_steps = args.steps.unwrap_or(F2_DEFAULT_STEPS);
    // Clock ceiling = stock boost top: F2 may hold an existing clock at lower voltage but never
    // overclock above stock. The offset caps are the conservative constants (not CLI-widenable).
    let limits = PositiveOffsetLimits::conservative(floor_mv, boost_top);
    let discovery_limits =
        PositiveOffsetLimits::hardware_frontier(floor_mv, boost_top, min_base);

    // Read-only Safe Loop state (shared by both modes; the dry-run NEVER mutates it).
    let record = store.load_record();
    let boot_flag_armed = store.is_boot_flag_armed();

    // MANUAL-PRIOR (explicit development / known-GPU shortcut): opt-in, anchored single point at an
    // operator-provided --start-mv with a SEPARATE larger bounded offset cap. NEVER the default;
    // branches BEFORE the autonomous anchored/simple dispatch so it cannot change default discovery
    // behavior or its conservative caps. Anchored-only (any --simple is ignored under --manual-prior).
    if args.manual_prior {
        run_manual_prior_undervolt_probe(
            store, confirm, &args, &sane, floor_mv, boost_top, focus_target, &record, boot_flag_armed,
        );
        return;
    }

    // LADDER-SWEEP (autonomous multi-target sweep): opt-in; runs a target sweep per `--targets` in order
    // with conservative priors. NEVER the default. Branches before auto-sweep / the plain dispatch.
    if args.ladder_sweep {
        run_anchored_ladder_sweep(store, confirm, &args, &sane, &discovery_limits);
        return;
    }

    // AUTO-SWEEP (autonomous same-target minimum-stable-voltage discovery): opt-in; uses the OFFICIAL
    // progressive anchored descent + observation learning. NEVER the default; branches before the plain
    // anchored/simple dispatch so default behavior is unchanged. Autonomous discovery uses the same
    // hardware-derived physical envelope as the live Forge: no arbitrary +30/+210 or +15 progression
    // ceiling, while the effective target remains capped at the stock live clock domain.
    if args.auto_sweep {
        run_anchored_target_sweep(
            store,
            confirm,
            &args,
            &sane,
            &discovery_limits,
            focus_target,
            &record,
            boot_flag_armed,
        );
        return;
    }

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
                    prev_offset_mhz: 0, // single-step: per-step measured from stock (unchanged)
                    dwell_ms: F2_STANDARD_DWELL_MS,
                    stress_purpose: F2StressPurpose::PowerDiscovery,
                    cancel: None,
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
        run_anchored_multi_step(store, confirm, args, sane, limits, focus_target, record, boot_flag_armed,
        );
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
                    prev_offset_mhz: 0, // single-step anchored: per-step measured from stock (unchanged)
                    dwell_ms: F2_STANDARD_DWELL_MS,
                    stress_purpose: F2StressPurpose::PowerDiscovery,
                    cancel: None,
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
/// with the requested physical candidate count) then drives [`run_confirmed_f2_multi_step`] over the
/// real per-candidate motor: each candidate arms Safe Loop → applies → verifies → dwells → resets →
/// clears, and the descent STOPS at the first non-stable candidate. Single-target, anchored-only,
/// never persists/applies/promotes a profile.
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
    // Explicit --steps is the operator-selected boundary; there is no additional hidden cap.
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

    for line in anchored_descent_plan_lines(&descent, limits, &preflight, max_steps) {
        println!("{line}");
    }

    if confirm {
        match confirmed_f2_multi_refusal(
            record,
            boot_flag_armed,
            args.steps,
            descent.candidates.len(),
            max_steps,
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
                    descent.candidates.len(),
                    focus_target
                );
                let mut ops = RealF2MultiOps {
                    store,
                    curve: sane.to_vec(),
                    candidates: descent.candidates.clone(),
                    limits: *limits,
                    target_mhz: focus_target,
                    // Explicit --steps descent: no cross-run observation resume (within-run advancement
                    // still chains each candidate off the prior validated one via `select`).
                    baseline_offset_mhz: 0,
                    prev_offset_override_mhz: None,
                    dwell_ms: F2_STANDARD_DWELL_MS,
                    stress_purpose: F2StressPurpose::PowerDiscovery,
                    cancel: None,
                    cur: None,
                };
                let report = run_confirmed_f2_multi_step(&mut ops, max_steps);
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
        "(dry-run — pass `--steps N --confirm` for a supervised \
         anchored descent; nothing was written)"
    );
    info!(
        "undervolt-probe: DRY-RUN (anchored multi-step) — target={} MHz candidates={} requested_steps={} \
         preflight_safe={} — no Safe Loop arm, no apply, no dwell, no VF write.",
        focus_target, descent.candidates.len(), max_steps, preflight.safe
    );
}

/// AUTO-SWEEP: autonomous same-target minimum-stable-voltage discovery (`--auto-sweep`). Uses the
/// OFFICIAL progressive anchored descent (conservative caps) — NOT manual-prior. DRY-RUN by default:
/// plans the descent, previews the current learned frontier (READ-ONLY), and prints where observations
/// would be recorded — no Safe Loop arm / apply / dwell / VF write / observation. WITH `--confirm` it
/// runs the full physical-bin descent, records ONE observation per
/// executed candidate via [`crate::gpu_f2_sweep::record_target_sweep`], and reports the discovered
/// last-good / first-bad / bracket. Single-target; autonomous (ignores `--steps`); never
/// persists/applies/promotes a profile.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_anchored_target_sweep(
    store: &SafeLoopStore,
    confirm: bool,
    args: &UndervoltArgs,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    focus_target: u32,
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
) {
    use nidavellir_core::f2_observation::{
        new_run_id, now_rfc3339, validated_descent_baseline, F2ObsMode, F2ObservationStore,
    };

    let descent =
        plan_anchored_undervolt_descent(sane, focus_target, args.start_mv, limits, F2_SWEEP_DRYRUN_BUDGET,
    );

    // This GPU's identity (read-only). Scopes the resume baseline to this card and tags observations.
    let gpu_key = nidavellir_gpu_nvapi::read_curve().ok().map(|c| c.name);

    // Read-only Safe Loop preflight over every planned bin (anchor + caps + elastic).
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

    // Read-only learned-frontier preview from the observation store (NEVER written in the dry-run).
    let obs_store = F2ObservationStore::system();
    let obs_path = obs_store.path().display().to_string();
    let frontier_preview = crate::gpu_f2_sweep::frontier_preview_for(&obs_store, focus_target);

    // Observation-aware chained descent: the deepest prior VALIDATED same-target/same-GPU point is the
    // per-step baseline for candidate 0 (the descent resumes from it instead of stock +0). Read-only.
    let target_obs = obs_store.query_by_target(focus_target);
    let baseline_obs = validated_descent_baseline(&target_obs, focus_target, gpu_key.as_deref());
    let baseline_offset_mhz = baseline_obs.map(|o| o.offset_mhz).unwrap_or(0);

    for line in crate::gpu_f2_sweep::target_sweep_plan_lines(
        focus_target,
        &descent,
        limits,
        descent.candidates.len(),
        &obs_path,
        frontier_preview.as_ref(),
        baseline_obs.map(|o| (o.anchor_mv, o.offset_mhz)),
        preflight.safe,
    ) {
        println!("{line}");
    }
    println!(
        "validation passes  : {} (extra confidence-building re-validations of the deepest point; default 1)",
        args.validation_passes
    );

    if confirm {
        // Confidence opt-in is BOUNDED: a request above the hard cap is REFUSED (fail closed), never
        // clamped — extra validations are real arm→apply→verify→dwell→reset hardware time per pass.
        if args.validation_passes > F2_MAX_VALIDATION_PASSES {
            println!(
                "undervolt-probe: --confirm REFUSED — --validation-passes {} exceeds the hard cap {} \
                 (fail closed). No Safe Loop arm, no apply, no dwell, no VF write, no observation recorded.",
                args.validation_passes, F2_MAX_VALIDATION_PASSES
            );
            warn!(
                "undervolt-probe: --confirm refused (auto-sweep): validation-passes {} > cap {} — no hardware touched",
                args.validation_passes, F2_MAX_VALIDATION_PASSES
            );
            return;
        }
        // Autonomous discovery ignores --steps and owns the complete physical candidate list. The
        // shared refusal still enforces Safe Mode / armed flag / crash threshold / has-candidates.
        let candidate_count = descent.candidates.len();
        match confirmed_f2_multi_refusal(
            record,
            boot_flag_armed,
            Some(candidate_count),
            candidate_count,
            candidate_count,
        ) {
            Some(reason) => {
                println!(
                    "undervolt-probe: --confirm REFUSED — {reason}. No Safe Loop arm, no apply, no dwell, \
                     no VF write, no observation recorded."
                );
                warn!("undervolt-probe: --confirm refused (auto-sweep): {reason} — no hardware touched");
            }
            None => {
                warn!(
                    "undervolt-probe: --confirm — executing autonomous TARGET SWEEP at {} MHz (up to {} \
                     candidate(s), descending; stops at first non-stable) — can TDR/reboot.",
                    focus_target,
                    candidate_count
                );
                let mut ops = RealF2MultiOps {
                    store,
                    curve: sane.to_vec(),
                    candidates: descent.candidates.clone(),
                    limits: *limits,
                    target_mhz: focus_target,
                    baseline_offset_mhz, // resume from the deepest prior validated same-target point
                    prev_offset_override_mhz: None,
                    dwell_ms: F2_STANDARD_DWELL_MS,
                    stress_purpose: F2StressPurpose::PowerDiscovery,
                    cancel: None,
                    cur: None,
                };
                let report = run_confirmed_f2_multi_step(&mut ops, candidate_count);
                for line in confirmed_multi_report_lines(focus_target, &descent.candidates, limits, &report) {
                    println!("{line}");
                }
                // Record ONE observation per executed candidate (confirmed path ONLY), then report the
                // discovered minimum-stable-voltage bracket. No profile is persisted/applied/promoted.
                let ctx = crate::gpu_f2_sweep::ObsContext {
                    run_id: new_run_id("f2-target-sweep"),
                    timestamp: now_rfc3339(),
                    gpu_key: gpu_key.clone(),
                    evidence_kind: nidavellir_core::f2_observation::F2EvidenceKind::Discovery,
                    discovery_contract_version: Some(
                        nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
                    ),
                    qualification_contract_version: None,
                    qualification_coverage: None,
                    mode: F2ObsMode::TargetSweep,
                    requested_start_mv: args.start_mv,
                    positive_offset_cap_mhz: limits.abs_max_offset_mhz,
                };
                let summary =
                    crate::gpu_f2_sweep::record_target_sweep(&ctx, focus_target, &descent, &report, &obs_store,
                );
                println!("=== TARGET SWEEP result ===");
                println!("executed/recorded  : {}/{}", summary.executed, summary.recorded);
                println!("last_good (min V)  : {:?} mV", summary.last_good_mv);
                println!("first_bad          : {:?} mV", summary.first_bad_mv);
                println!("bracket            : {:?}", summary.bracket);
                println!("stop reason        : {}", summary.stop_reason);
                println!("frontier updated   : {}", summary.frontier_updated);
                println!("ended safe (reset) : {}", summary.safe);
                println!("profile            : none persisted, applied, or promoted");
                info!(
                    "undervolt-probe: confirmed TARGET SWEEP — target={} last_good={:?} first_bad={:?} \
                     recorded={} safe={}",
                    focus_target, summary.last_good_mv, summary.first_bad_mv, summary.recorded, summary.safe
                );

                // CONFIDENCE OPT-IN (--validation-passes > 1): re-validate ONLY the deepest validated
                // candidate up to `validation_passes - 1` extra times. Each pass is a full, reset-clean
                // arm→apply→verify→dwell→reset on the SAME point (no new/deeper point is tried), recording
                // ONE observation per pass so `validations_at_best` accumulates and raises the frontier
                // confidence. Stops immediately on any non-Validated / safety failure. No-op when the flag
                // is absent (default 1) or the descent produced no validated point.
                let extra_passes = args.validation_passes.saturating_sub(1);
                if extra_passes > 0 {
                    if let Some(deepest_index) = report.last_good_index {
                        if let Some(deepest) = descent.candidates.get(deepest_index).cloned() {
                            println!(
                                "=== confidence re-validation: deepest point ({} mV, +{} MHz) × up to {} extra pass(es) ===",
                                deepest.anchor.voltage_mv, deepest.anchor.offset_mhz, extra_passes
                            );
                            // Reproduce the deepest point exactly: a single-candidate motor whose baseline
                            // is the offset the deepest candidate chained from during the descent.
                            let deepest_baseline =
                                chained_prev_offset(&descent.candidates, deepest_index, baseline_offset_mhz,
                            );
                            let mut rev_ops = RealF2MultiOps {
                                store,
                                curve: sane.to_vec(),
                                candidates: vec![deepest.clone()],
                                limits: *limits,
                                target_mhz: focus_target,
                                baseline_offset_mhz: deepest_baseline,
                                prev_offset_override_mhz: None,
                                dwell_ms: F2_STANDARD_DWELL_MS,
                                stress_purpose: F2StressPurpose::PowerDiscovery,
                                cancel: None,
                                cur: None,
                            };
                            let extra_reports =
                                run_confirmed_f2_extra_validations(&mut rev_ops, 0, extra_passes);
                            // Record ONE observation per executed extra pass at the deepest point.
                            let single = AnchoredDescentPlan {
                                focus_target_mhz: focus_target,
                                start_mv: descent.start_mv,
                                max_steps: 1,
                                candidates: vec![deepest],
                                stop_reason: None,
                                skipped_above_target: 0,
                            };
                            let mut extra_recorded = 0usize;
                            for (pass, step) in extra_reports.iter().enumerate() {
                                let one = F2MultiStepReport {
                                    planned: 1,
                                    executed: 1,
                                    steps: vec![step.clone()],
                                    last_good_index: matches!(step.outcome, F2Outcome::Validated)
                                        .then_some(0),
                                    stop_reason: F2MultiStopReason::CompletedAllPlanned,
                                };
                                let rev_ctx = crate::gpu_f2_sweep::ObsContext {
                                    run_id: new_run_id("f2-target-sweep-reval"),
                                    timestamp: now_rfc3339(),
                                    gpu_key: gpu_key.clone(),
                                    evidence_kind:
                                        nidavellir_core::f2_observation::F2EvidenceKind::Discovery,
                                    discovery_contract_version: Some(
                                        nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
                                    ),
                                    qualification_contract_version: None,
                                    qualification_coverage: None,
                                    mode: F2ObsMode::TargetSweep,
                                    requested_start_mv: args.start_mv,
                                    positive_offset_cap_mhz: limits.abs_max_offset_mhz,
                                };
                                let rev_summary = crate::gpu_f2_sweep::record_target_sweep(
                                    &rev_ctx, focus_target, &single, &one, &obs_store,
                                );
                                extra_recorded += rev_summary.recorded;
                                println!(
                                    "  pass {:>2}: outcome {:?}, safe {}",
                                    pass + 1,
                                    step.outcome,
                                    rev_summary.safe
                                );
                            }
                            let final_frontier =
                                crate::gpu_f2_sweep::frontier_preview_for(&obs_store, focus_target);
                            println!(
                                "re-validations     : {}/{} extra pass(es) executed/recorded; frontier confidence now {:?}",
                                extra_reports.len(),
                                extra_passes,
                                final_frontier.map(|e| e.confidence)
                            );
                            println!("profile            : none persisted, applied, or promoted");
                            info!(
                                "undervolt-probe: confirmed TARGET SWEEP re-validation — target={} deepest_idx={} \
                                 extra_passes_requested={} executed={} recorded={}",
                                focus_target, deepest_index, extra_passes, extra_reports.len(), extra_recorded
                            );
                        }
                    }
                }
            }
        }
        return;
    }
    println!(
        "(dry-run — pass `--target-mhz {focus_target} --auto-sweep --confirm` for a physically bounded autonomous \
         sweep; nothing was written)"
    );
    info!(
        "undervolt-probe: DRY-RUN (auto-sweep) — target={} MHz candidates={} preflight_safe={} — no Safe \
         Loop arm, no apply, no dwell, no VF write, no observation.",
        focus_target, descent.candidates.len(), preflight.safe
    );
}

/// LADDER-SWEEP: autonomous multi-target minimum-stable-voltage discovery (`--ladder-sweep --targets
/// a,b,c`). Runs an autonomous target sweep for each target IN ORDER, using a lower target's discovered
/// minimum only as a conservative descent FLOOR for higher targets (never assuming the lower voltage
/// holds the higher clock). DRY-RUN by default: plans each target's descent, prints the conservative-prior
/// policy + the current learned frontier + the classifier-bridge preview — no Safe Loop arm / apply /
/// dwell / VF write / observation. WITH `--confirm` it runs each target's physically bounded sweep sequentially,
/// records observations, and STOPS the whole ladder on a safety failure. Never persists/applies/promotes
/// a profile.
#[cfg(windows)]
fn run_anchored_ladder_sweep(
    store: &SafeLoopStore,
    confirm: bool,
    args: &UndervoltArgs,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
) {
    use nidavellir_core::f2_observation::{
        last_good_for_target, new_run_id, now_rfc3339, F2ObsMode, F2ObservationStore,
    };

    if args.targets.is_empty() {
        println!(
            "undervolt-probe: --ladder-sweep REQUIRES --targets (e.g. --targets 1800,1815,1830). \
             No hardware touched."
        );
        warn!("undervolt-probe: --ladder-sweep without --targets — refused, no hardware touched");
        return;
    }

    // Safe Loop state is re-read per target inside the confirmed loop (a prior target may change it).
    let obs_store = F2ObservationStore::system();
    let obs_path = obs_store.path().display().to_string();
    let base_floor = limits.hw_floor_mv;

    // Dry-run plan: per target, a conservative descent floor from the prior target's KNOWN last-good
    // (read-only from the store) — a FLOOR, never an assumption that the prior voltage holds.
    let mut plans = Vec::new();
    for (idx, &target) in args.targets.iter().enumerate() {
        let prev_target = idx.checked_sub(1).map(|j| args.targets[j]);
        let prior = prev_target.and_then(|prev_t| {
            last_good_for_target(&obs_store.query_by_target(prev_t), prev_t).map(|o| o.anchor_mv)
        });
        // Direction-aware: descending uses the prior last-good as a START ceiling (full base floor);
        // ascending/first uses it as a conservative FLOOR (today's behavior).
        let (start_mv, floor_mv) = crate::gpu_f2_sweep::ladder_target_descent_bounds(
            base_floor, prior, target, prev_target, args.start_mv,
        );
        let target_limits = PositiveOffsetLimits { hw_floor_mv: floor_mv, ..*limits };
        let descent =
            plan_anchored_undervolt_descent(sane, target, start_mv, &target_limits, F2_SWEEP_DRYRUN_BUDGET,
        );
        plans.push(crate::gpu_f2_sweep::ladder_target_plan(target, floor_mv, prior, &descent,
        ));
    }
    let max_planned_candidates = plans.iter().map(|plan| plan.candidate_count).max().unwrap_or(0);
    for line in crate::gpu_f2_sweep::ladder_plan_lines(&plans, max_planned_candidates, &obs_path) {
        println!("{line}");
    }
    // Learned frontier report + the read-only classifier-bridge preview (no profile applied/persisted).
    let frontier = obs_store.learned_frontier();
    for line in crate::gpu_f2_sweep::frontier_report_lines(&frontier) {
        println!("{line}");
    }
    for line in crate::gpu_power_sweep::classify_f2_frontier_summary(&frontier) {
        println!("{line}");
    }

    if confirm {
        // CONFIRMED ladder (bounded; stops on a safety failure). Each target runs its own autonomous
        // sweep with a conservative floor from the PREVIOUS target's discovered last-good; a normal bad
        // candidate stops only that target, but a safety failure stops the whole ladder.
        warn!(
            "undervolt-probe: --confirm — executing autonomous LADDER SWEEP over {:?} (per-target physical frontier; \
             stops on safety failure) — can TDR/reboot.",
            args.targets
        );
        let run_id = new_run_id("f2-ladder");
        let mut prev_good: Option<u32> = None;
        for (idx, &target) in args.targets.iter().enumerate() {
            let rec = store.load_record();
            let armed = store.is_boot_flag_armed();
            let prev_target = idx.checked_sub(1).map(|j| args.targets[j]);
            let prior = prev_good.or_else(|| {
                prev_target.and_then(|prev_t| {
                    last_good_for_target(&obs_store.query_by_target(prev_t), prev_t).map(|o| o.anchor_mv)
                })
            });
            // Direction-aware: descending uses the prior last-good as a START ceiling (full base floor);
            // ascending/first uses it as a conservative FLOOR (today's behavior).
            let (start_mv, floor_mv) = crate::gpu_f2_sweep::ladder_target_descent_bounds(
                base_floor, prior, target, prev_target, args.start_mv,
            );
            let target_limits = PositiveOffsetLimits { hw_floor_mv: floor_mv, ..*limits };
            let descent = plan_anchored_undervolt_descent(
                sane,
                target,
                start_mv,
                &target_limits,
                F2_SWEEP_DRYRUN_BUDGET,
            );
            let candidate_count = descent.candidates.len();
            match confirmed_f2_multi_refusal(
                &rec,
                armed,
                Some(candidate_count),
                candidate_count,
                candidate_count,
            ) {
                Some(reason) => {
                    println!("ladder: target {target} MHz REFUSED — {reason}; stopping ladder.");
                    warn!("undervolt-probe: ladder stopped at {target} MHz — refused: {reason}");
                    break;
                }
                None => {
                    let mut ops = RealF2MultiOps {
                        store,
                        curve: sane.to_vec(),
                        candidates: descent.candidates.clone(),
                        limits: target_limits,
                        target_mhz: target,
                        // Ladder keeps its conservative per-target voltage-FLOOR policy (no cross-run
                        // offset resume); within-run advancement still chains each candidate via `select`.
                        baseline_offset_mhz: 0,
                        prev_offset_override_mhz: None,
                        dwell_ms: F2_STANDARD_DWELL_MS,
                        stress_purpose: F2StressPurpose::PowerDiscovery,
                        cancel: None,
                        cur: None,
                    };
                    let report = run_confirmed_f2_multi_step(&mut ops, candidate_count);
                    let ctx = crate::gpu_f2_sweep::ObsContext {
                        run_id: run_id.clone(),
                        timestamp: now_rfc3339(),
                        gpu_key: nidavellir_gpu_nvapi::read_curve().ok().map(|c| c.name),
                        evidence_kind:
                            nidavellir_core::f2_observation::F2EvidenceKind::Discovery,
                        discovery_contract_version: Some(
                            nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
                        ),
                        qualification_contract_version: None,
                        qualification_coverage: None,
                        mode: F2ObsMode::LadderSweep,
                        requested_start_mv: args.start_mv,
                        positive_offset_cap_mhz: target_limits.abs_max_offset_mhz,
                    };
                    let summary =
                        crate::gpu_f2_sweep::record_target_sweep(&ctx, target, &descent, &report, &obs_store,
                    );
                    println!(
                        "ladder: target {} MHz → last_good {:?} mV, first_bad {:?} mV, safe {}, stop {}",
                        target, summary.last_good_mv, summary.first_bad_mv, summary.safe, summary.stop_reason
                    );
                    if !crate::gpu_f2_sweep::ladder_should_continue(summary.safe) {
                        println!("ladder: target {target} MHz ended UNSAFE — stopping ladder (no further targets).");
                        warn!("undervolt-probe: ladder stopped at {target} MHz — unsafe end");
                        break;
                    }
                    prev_good = summary.last_good_mv;
                }
            }
        }
        println!("=== F2 LADDER result — final learned frontier ===");
        for line in crate::gpu_f2_sweep::frontier_report_lines(&obs_store.learned_frontier()) {
            println!("{line}");
        }
        println!("profile            : none persisted, applied, or promoted");
        return;
    }
    println!(
        "(dry-run — pass `--ladder-sweep --targets {} --confirm` for a physically bounded autonomous ladder; nothing was written)",
        args.targets.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",")
    );
    info!(
        "undervolt-probe: DRY-RUN (ladder-sweep) — targets={:?} — no Safe Loop arm, no apply, no dwell, \
         no VF write, no observation.",
        args.targets
    );
}

/// Read-only F2 forge inputs (see [`f2_forge_inputs`]). The static VF base curve sanity-filtered to
/// plausible core points, the hardware voltage floor (lowest sane bin), the stock boost top (highest
/// live core clock = the clock ceiling), and the hardware-derived positive-offset envelope.
#[cfg(windows)]
pub(crate) struct F2ForgeInputs {
    pub sane_base_curve: Vec<(usize, u32, u32)>,
    #[allow(dead_code)] // the clock ceiling lives inside `limits`; exposed for caller diagnostics
    pub boost_top_mhz: u32,
    pub limits: PositiveOffsetLimits,
}

/// Read-only F2 forge inputs derived from the static VF base plus the caller's validated live stock
/// clock ceiling. The offset envelope spans the physical curve, while the effective target can never
/// exceed that stock ceiling. Returns `None` when no sane base points exist. No Safe Loop arm, apply,
/// dwell, or VF write.
#[cfg(windows)]
pub(crate) fn f2_forge_inputs(clock_ceiling_mhz: u32) -> Option<F2ForgeInputs> {
    use nidavellir_gpu_nvapi as gpu;
    let sane: Vec<(usize, u32, u32)> = gpu::read_vf_base_curve_modern()
        .into_iter()
        .filter(|&(_, mv, f)| is_f2_sane_point(mv, f))
        .collect();
    if sane.is_empty() {
        warn!("f2-forge: no sane static VF base points available — fail closed (no hardware touched)");
        return None;
    }
    let floor_mv = sane.iter().map(|&(_, mv, _)| mv).min().unwrap();
    let min_base = sane.iter().map(|&(_, _, f)| f).min().unwrap();
    // The Forge must traverse the complete real VF domain. Its offset envelope is therefore derived
    // from the hardware curve itself, while the effective target remains capped at stock boost top.
    let limits =
        PositiveOffsetLimits::hardware_frontier(floor_mv, clock_ceiling_mhz, min_base);
    Some(F2ForgeInputs { sane_base_curve: sane,
        boost_top_mhz: clock_ceiling_mhz, limits,
    })
}

/// Apply ONE F2 anchored undervolt point to hardware and LEAVE it applied — the APPLY path (not a probe).
/// Reads the live static VF base curve, plans+writes the bounded anchored offset for `target_mhz` anchored
/// at the exact validated `anchor_mv` bin, then verifies the write. FAIL-CLOSED: a missing anchor, a writer
/// rejection, or any non-`AnchoredRaiseVerified` verdict resets to stock (confirming every touched bin
/// reads ~0) and returns `Err` WITHOUT leaving an offset applied. On success the anchored curve stays
/// resident (that is the apply). Reuses the SAME primitives as the confirmed motor ([`RealF2Ops`]) with
/// `prev_offset = 0`. Owns NO Safe Loop arm / persist — the caller
/// ([`crate::gpu_apply::apply_and_persist_undervolt`]) owns the boot-flag + persistence lifecycle.
#[cfg(windows)]
pub(crate) fn apply_anchored_undervolt(target_mhz: u32, anchor_mv: u32) -> Result<(), String> {
    use nidavellir_gpu_nvapi as gpu;

    // Profile switching must derive the stock live ceiling, not the currently applied/capped curve.
    // The caller already armed Safe Loop, so a reset failure remains recoverable and fails closed.
    gpu::reset_all().map_err(|e| format!("F2 apply: pre-apply stock reset failed: {e}"))?;

    // Read-only static VF base, sanity-filtered — identical envelope to the forge motor.
    let sane: Vec<(usize, u32, u32)> = gpu::read_vf_base_curve_modern()
        .into_iter()
        .filter(|&(_, mv, f)| is_f2_sane_point(mv, f))
        .collect();
    if sane.is_empty() {
        return Err("F2 apply: no sane static VF base points — fail closed (no write)".into());
    }
    let floor_mv = sane.iter().map(|&(_, mv, _)| mv).min().unwrap();
    let live_curve = gpu::read_vf_curve_modern();
    let boost_top = crate::gpu_power_sweep::f2_stock_clock_ceiling(&live_curve)
        .map_err(|e| format!("F2 apply: stock clock domain unavailable: {e}"))?;
    let min_base = sane.iter().map(|&(_, _, f)| f).min().unwrap();
    // Apply the exact already-validated Forge point under the same hardware-derived envelope used
    // during discovery. The target is still capped at stock boost top and the exact anchor is required.
    let limits = PositiveOffsetLimits::hardware_frontier(floor_mv, boost_top, min_base);

    let Some(anchor_idx) = select_exact_apply_anchor_bin(&sane, target_mhz, anchor_mv) else {
        return Err(format!(
            "F2 apply: exact validated VF bin {anchor_mv} mV is unavailable or cannot raise to {target_mhz} MHz — fail closed (no write)"
        ));
    };

    // Fail-closed reset helper: reset to stock, then CONFIRM every given bin reads ~0 (a positive offset
    // must NEVER survive an error — including a mid-loop PARTIAL write). Returns `reason` on a confirmed
    // clean reset, or a more-severe reset-not-confirmed message if any bin cannot be confirmed cleared.
    let reset_and_confirm = |indices: &[usize], reason: String| -> String {
        crate::gpu_power_sweep::reset_to_stock();
        for &idx in indices {
            match gpu::vf_get_point_khz(idx) {
                Some(khz) if khz.abs() <= F2_RESET_TOL_KHZ => {}
                Some(khz) => {
                    return format!("{reason}; reset readback {khz} kHz NOT cleared at idx {idx}")
                }
                None => {
                    return format!(
                        "{reason}; reset readback unavailable at idx {idx} — cannot confirm cleared"
                    )
                }
            }
        }
        reason
    };

    // Plan + write the anchored curve (anchor raise + plateau caps + elastic below). prev_offset = 0: a
    // fresh single-point apply. A writer rejection may have PARTIALLY written, so reset + confirm EVERY
    // sane bin (the full set the writer could have touched) cleared before returning Err.
    let plan = match gpu::apply_bounded_anchored_positive_offset(&sane, anchor_idx, target_mhz, 0, &limits,
    )
    {
        Ok(p) => p,
        Err(e) => {
            let all: Vec<usize> = sane.iter().map(|&(i, _, _)| i).collect();
            return Err(reset_and_confirm(&all, format!("F2 apply: anchored write rejected ({e})"),
            ));
        }
    };

    // Verify: read back every touched bin's offset and run the anchored verifier (offset readback is
    // primary — idle GetStatus under-reports). Only AnchoredRaiseVerified leaves the curve applied.
    let observed: Vec<(usize, Option<i32>)> = plan
        .entries
        .iter()
        .map(|e| {
            (e.index, gpu::vf_get_point_khz(e.index).map(|khz| khz / 1000),
            )
        })
        .collect();
    let verdict = crate::gpu_verify::verify_anchored_positive_offset(&plan, &observed, F2_VERIFY_TOL_MHZ);
    if verdict == AnchoredOffsetVerification::AnchoredRaiseVerified {
        // v13: absolute clock ceiling at the applied target — the plateau caps alone shift with the
        // driver's thermal curve compensation, so without the ceiling the delivered regime exceeds
        // the validated point. Fail-closed: no ceiling ⇒ no apply (undo the verified curve too).
        if let Err(e) = nidavellir_core::nvml_gpu::lock_core_clock_max_mhz(target_mhz) {
            warn!("F2 apply: v13 clock ceiling failed ({e}) — resetting to stock (fail closed)");
            let touched: Vec<usize> = plan.entries.iter().map(|e| e.index).collect();
            return Err(reset_and_confirm(
                &touched,
                format!("F2 apply: v13 clock ceiling ({target_mhz} MHz) failed ({e})"),
            ));
        }
        info!("F2 apply: anchored undervolt verified ({target_mhz} MHz @ {anchor_mv} mV bin) + clock ceiling {target_mhz} MHz");
        return Ok(());
    }

    // Fail-closed: undo the write and confirm every touched bin cleared before returning the error.
    warn!("F2 apply: anchored verify {verdict:?} — resetting to stock (fail closed)");
    let touched: Vec<usize> = plan.entries.iter().map(|e| e.index).collect();
    Err(reset_and_confirm(&touched, format!("F2 apply: anchored verify failed ({verdict:?})"),
    ))
}

/// Result of one live-Forge target's complete physical-bin discovery.
#[cfg(windows)]
pub(crate) struct F2ClockDiscoverySummary {
    pub sustainable: bool,
    pub last_good_mv: Option<u32>,
    pub first_bad_mv: Option<u32>,
    pub next_clock_start_mv: Option<u32>,
    pub conservative_start_mv: Option<u32>,
    pub warm_start_rejected: bool,
    pub executed_steps: usize,
    pub completed: bool,
    pub aborted: bool,
    /// A crash/device-loss or unconfirmed reset must survive the outer belt-and-suspenders reset so
    /// startup recovery can still account for the interrupted hardware run.
    pub retain_boot_flag: bool,
    pub stop_reason: String,
    pub logs: Vec<String>,
}

#[cfg(windows)]
pub(crate) struct F2ClockDiscoveryProgress {
    pub target_mhz: u32,
    pub planned_steps: usize,
    pub unpruned_steps: usize,
    pub anchor_mv: Option<u32>,
    pub outcome: Option<String>,
    pub line: String,
}

/// Result of filling one missing exact-Apply-bin PowerRender measurement after the qualified
/// frontier is complete. This step never promotes stability; it only contributes current-contract
/// power telemetry. The distinct exact-Apply v8 gate runs after synthesis.
#[cfg(windows)]
pub(crate) struct F2PowerCalibrationSummary {
    pub confirmed: bool,
    pub executed_steps: usize,
    pub aborted: bool,
    pub retain_boot_flag: bool,
    pub stop_reason: String,
    pub logs: Vec<String>,
}

/// Result of the long v8 three-pattern gate at the exact post-margin Apply pair selected for a profile.
/// A reset-clean rejection is local to this candidate and lets synthesis choose another point; hard
/// device/reset/write failures still abort the Forge.
#[cfg(windows)]
pub(crate) struct F2ApplyQualificationSummary {
    pub qualified: bool,
    pub executed_steps: usize,
    pub aborted: bool,
    pub cancelled: bool,
    pub retain_boot_flag: bool,
    pub stop_reason: String,
    pub logs: Vec<String>,
}

#[cfg(windows)]
fn plan_f2_power_calibration_candidate(
    sane: &[(usize, u32, u32)],
    target_mhz: u32,
    apply_mv: u32,
    reference_offset_mhz: i32,
    limits: &PositiveOffsetLimits,
) -> Result<AnchoredPositiveOffsetPlan, String> {
    let anchor_index = select_exact_apply_anchor_bin(sane, target_mhz, apply_mv)
        .ok_or_else(|| format!("{target_mhz} MHz @ {apply_mv} mV is not an exact valid F2 anchor"))?;
    plan_bounded_anchored_positive_offset(
        sane,
        anchor_index,
        target_mhz,
        reference_offset_mhz,
        limits,
    )
}

fn f2_conservative_next_clock_start(
    power_bound_clock_drops: &[u32],
    validated_voltages: &[u32],
) -> Option<u32> {
    power_bound_clock_drops
        .iter()
        .copied()
        .min()
        .or_else(|| validated_voltages.iter().copied().max())
}

fn f2_optimized_next_clock_start(
    planned_voltages: &[u32],
    last_good_mv: Option<u32>,
    conservative_start_mv: Option<u32>,
) -> Option<u32> {
    last_good_mv
        .and_then(|good| {
            planned_voltages
                .iter()
                .copied()
                .filter(|mv| *mv > good)
                .min()
                .or(Some(good))
        })
        .or(conservative_start_mv)
}

/// Result of running the full N-pass FailureSeekingGameLoop qualification on ONE anchored candidate.
#[cfg(windows)]
enum F2QualificationOutcome {
    /// All `qualification_passes` passes validated reset-clean.
    Qualified,
    /// A reset-clean instability — the qualifier rejected this point. Carries the failing outcome's
    /// debug string (purely for the stop reason / logs).
    Rejected(String),
    /// Reset-clean but coverage too weak to accept or reject after the retry budget.
    Inconclusive,
    /// Stop was requested mid-qualification.
    Cancelled,
    /// A hard failure (device lost / reset failed / arm / apply / verify / precheck / persist). The
    /// caller must abort the forge; `retain_boot_flag` follows the usual DeviceLost/ResetFailed rule.
    Aborted { stop_reason: String, retain_boot_flag: bool },
}

#[cfg(windows)]
fn f2_should_qualify_discovery_candidate(
    outcome: &F2Outcome,
    near_power_limit: bool,
    qualification_passes: usize,
) -> bool {
    matches!(outcome, F2Outcome::Validated)
        && !near_power_limit
        && qualification_passes > 0
}

#[cfg(windows)]
impl F2QualificationOutcome {
    /// Whether qualification reached a reset-clean terminal result for this clock. A rejection is
    /// local evidence against the current target; it must not abort discovery of lower clocks.
    fn completes_clock(&self) -> bool {
        matches!(
            self,
            Self::Qualified | Self::Rejected(_) | Self::Inconclusive
        )
    }
}

#[cfg(windows)]
#[derive(Default)]
struct F2QualificationMarginHistory {
    pattern_a: Vec<u32>,
    pattern_b: Vec<u32>,
    high_fps: Vec<u32>,
    texture: Vec<u32>,
    transitions: Vec<u32>,
    memory: Vec<u32>,
}

#[cfg(windows)]
impl F2QualificationMarginHistory {
    fn values(&self, pattern: F2QualificationPattern) -> &[u32] {
        match pattern {
            F2QualificationPattern::A => &self.pattern_a,
            F2QualificationPattern::B => &self.pattern_b,
            F2QualificationPattern::HighFps => &self.high_fps,
            F2QualificationPattern::Texture => &self.texture,
            F2QualificationPattern::Transitions => &self.transitions,
            F2QualificationPattern::Memory => &self.memory,
            // ponytail: the candidate-only gates never feed the descent margin history.
            F2QualificationPattern::Endurance | F2QualificationPattern::TransitionShock => &[],
        }
    }

    fn push(&mut self, pattern: F2QualificationPattern, p5_mhz: u32) {
        match pattern {
            F2QualificationPattern::A => self.pattern_a.push(p5_mhz),
            F2QualificationPattern::B => self.pattern_b.push(p5_mhz),
            F2QualificationPattern::HighFps => self.high_fps.push(p5_mhz),
            F2QualificationPattern::Texture => self.texture.push(p5_mhz),
            F2QualificationPattern::Transitions => self.transitions.push(p5_mhz),
            F2QualificationPattern::Memory => self.memory.push(p5_mhz),
            // ponytail: the candidate-only gates never feed the descent margin history.
            F2QualificationPattern::Endurance | F2QualificationPattern::TransitionShock => {}
        }
    }
}

#[cfg(windows)]
fn median_u32(values: &[u32]) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some(((u64::from(sorted[middle - 1]) + u64::from(sorted[middle])) / 2) as u32)
    } else {
        Some(sorted[middle])
    }
}

#[cfg(windows)]
fn qualification_margin_p5(coverage: Option<&F2QualificationCoverage>) -> Option<u32> {
    let coverage = coverage?;
    if coverage.verdict != F2QualificationVerdict::Pass {
        return None;
    }
    let heavy_phase_p5: Vec<u32> = coverage
        .phase_metrics
        .iter()
        .filter(|metric| {
            metric.coverage_status == "pass"
                && matches!(
                    metric.phase_name.as_str(),
                    "heavy-spike" | "texture-rop" | "mixed-game" | "power-closing"
                )
        })
        .filter_map(|metric| metric.clock_p5)
        .collect();
    median_u32(&heavy_phase_p5)
}

#[cfg(windows)]
fn qualification_margin_is_clock_drop(
    current_p5_mhz: u32,
    stable_history: &[u32],
    target_mhz: u32,
) -> bool {
    let below_target =
        current_p5_mhz.saturating_add(F2_CLOCK_DROP_TOL_MHZ) < target_mhz;
    let below_relative_margin = stable_history.len() >= 2
        && median_u32(stable_history).is_some_and(|baseline| {
            current_p5_mhz.saturating_add(MARGIN_DROP_TOL_MHZ) < baseline
        });
    below_target || below_relative_margin
}

#[cfg(windows)]
fn qualification_attempt_dwell_ms(base_dwell_ms: u64, retry_count: usize) -> u64 {
    if retry_count == 0 {
        base_dwell_ms
    } else {
        base_dwell_ms.saturating_mul(3) / 2
    }
}

#[cfg(windows)]
fn qualification_should_retry_inconclusive(retry_count: usize) -> bool {
    retry_count < INCONCLUSIVE_RETRY_BUDGET
}

#[cfg(windows)]
fn apply_qualification_pattern_complete(
    inconclusive_count: usize,
    consecutive_clean_passes: usize,
) -> bool {
    inconclusive_count == 0 || consecutive_clean_passes >= 2
}

#[cfg(windows)]
fn annotate_qualification_report(
    report: &mut F2StepReport,
    strength: F2QualificationStrength,
    pattern: Option<F2QualificationPattern>,
    pass_index: u32,
    retry_count: u32,
) {
    if let Some(coverage) = report.qualification_coverage.as_mut() {
        coverage.strength = strength;
        coverage.pattern = pattern;
        coverage.pass_index = pass_index;
        coverage.retry_count = retry_count;
    }
}

/// Run the v8 qualification (`qualification_passes` independent reset/reapply patterns) on ONE
/// already-PowerRender-validated anchored candidate. Each pass uses the proven
/// arm→write→verify→dwell→reset motor and is persisted immediately as Qualification evidence. Pure
/// hardware sequencing — all policy (what to do with the result) stays in the caller.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn qualify_anchored_candidate(
    store: &SafeLoopStore,
    obs_store: &nidavellir_core::f2_observation::F2ObservationStore,
    qual_ctx: &mut crate::gpu_f2_sweep::ObsContext,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
    candidate: &AnchoredPositiveOffsetPlan,
    candidate_count: usize,
    unpruned_steps: usize,
    qualification_dwell_ms: u64,
    qualification_passes: usize,
    margin_history: &mut F2QualificationMarginHistory,
    render_goldens: Option<RenderGoldens>,
    stop: &std::sync::atomic::AtomicBool,
    logs: &mut Vec<String>,
    executed_steps: &mut usize,
    on_progress: &mut dyn FnMut(F2ClockDiscoveryProgress),
) -> F2QualificationOutcome {
    use std::sync::atomic::Ordering;

    use nidavellir_core::f2_observation::now_rfc3339;

    let Some(goldens) = render_goldens else {
        return F2QualificationOutcome::Aborted {
            stop_reason: "QualificationGoldenMissing".into(),
            retain_boot_flag: false,
        };
    };
    let patterns = qualification_gate_patterns(qualification_passes);
    for (pattern_index, pattern) in patterns.iter().copied().enumerate() {
        let pass_index = pattern_index + 1;
        let mut inconclusive_retries = 0usize;
        loop {
            if stop.load(Ordering::SeqCst) {
                return F2QualificationOutcome::Cancelled;
            }
            let attempt_dwell_ms =
                qualification_attempt_dwell_ms(qualification_dwell_ms, inconclusive_retries);
            let mut validation_ops = RealF2MultiOps {
                store,
                curve: sane.to_vec(),
                candidates: vec![candidate.clone()],
                limits: *limits,
                target_mhz,
                baseline_offset_mhz: 0,
                prev_offset_override_mhz: None,
                dwell_ms: attempt_dwell_ms,
                stress_purpose: F2StressPurpose::V8Qualification(pattern, goldens),
                cancel: Some(stop),
                cur: None,
            };
            if let Err(e) = validation_ops.select(0) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("QualificationPrecheckFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: None,
                line: format!(
                    "v8 {} {}/{}: {target_mhz} MHz @ {} mV ({} s)…",
                    qualification_pattern_label(pattern),
                    pass_index,
                    patterns.len(),
                    candidate.anchor.voltage_mv,
                    attempt_dwell_ms / 1000
                ),
            });
            let mut report = run_confirmed_f2_step(&mut validation_ops);
            annotate_qualification_report(
                &mut report,
                F2QualificationStrength::Fsgl4,
                Some(pattern),
                pass_index as u32,
                inconclusive_retries as u32,
            );
            if matches!(report.outcome, F2Outcome::Validated) {
                if let Some(current_p5) =
                    qualification_margin_p5(report.qualification_coverage.as_ref())
                {
                    if qualification_margin_is_clock_drop(
                        current_p5,
                        margin_history.values(pattern),
                        target_mhz,
                    ) {
                        report.outcome = F2Outcome::ClockDrop;
                        report.dwell = Some(F2DwellOutcome::ClockDrop);
                        report.validated = false;
                        logs.push(format!(
                            "{target_mhz} MHz @ {} mV v8 {}: colapso de margem p5={current_p5} MHz (baseline {:?} MHz, tolerância {} MHz)",
                            candidate.anchor.voltage_mv,
                            qualification_pattern_label(pattern),
                            median_u32(margin_history.values(pattern)),
                            MARGIN_DROP_TOL_MHZ
                        ));
                    } else {
                        margin_history.push(pattern, current_p5);
                    }
                }
            }
            logs.push(format!(
                "{target_mhz} MHz @ {} mV v8 {} {}/{}: {:?}",
                candidate.anchor.voltage_mv,
                qualification_pattern_label(pattern),
                pass_index,
                patterns.len(),
                report.outcome
            ));
            qual_ctx.timestamp = now_rfc3339();
            let observation = crate::gpu_f2_sweep::observation_from_anchored_step(
                qual_ctx,
                target_mhz,
                candidate,
                &report,
            );
            if let Err(e) = obs_store.append(&observation) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("ObservationPersistFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            *executed_steps = executed_steps.saturating_add(1);
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: Some(format!("{:?}", report.outcome)),
                line: format!(
                    "{target_mhz} MHz @ {} mV · v8 {} {}/{} → {:?} · aprendizado salvo",
                    candidate.anchor.voltage_mv,
                    qualification_pattern_label(pattern),
                    pass_index,
                    patterns.len(),
                    report.outcome
                ),
            });
            match &report.outcome {
                F2Outcome::Validated => break,
                F2Outcome::DeviceLost
                | F2Outcome::ResetFailed
                | F2Outcome::ArmFailed(_)
                | F2Outcome::ApplyFailed(_)
                | F2Outcome::VerifyFailed => {
                    return F2QualificationOutcome::Aborted {
                        stop_reason: format!("QualificationAborted: {:?}", report.outcome),
                        retain_boot_flag: f2_outcome_retains_boot_flag(&report.outcome),
                    };
                }
                F2Outcome::Inconclusive => {
                    if qualification_should_retry_inconclusive(inconclusive_retries) {
                        inconclusive_retries += 1;
                        logs.push(format!(
                            "{target_mhz} MHz @ {} mV v8 {} inconclusivo; retentativa {}/{} com dwell ampliado",
                            candidate.anchor.voltage_mv,
                            qualification_pattern_label(pattern),
                            inconclusive_retries,
                            INCONCLUSIVE_RETRY_BUDGET
                        ));
                        continue;
                    }
                    return F2QualificationOutcome::Inconclusive;
                }
                other => return F2QualificationOutcome::Rejected(format!("{other:?}")),
            }
        }
    }
    F2QualificationOutcome::Qualified
}

#[cfg(windows)]
fn qualification_pattern_label(pattern: F2QualificationPattern) -> &'static str {
    match pattern {
        F2QualificationPattern::A => "A",
        F2QualificationPattern::B => "B",
        F2QualificationPattern::HighFps => "High-FPS",
        F2QualificationPattern::Texture => "Texture",
        F2QualificationPattern::Transitions => "Transitions",
        F2QualificationPattern::Memory => "Memory",
        F2QualificationPattern::Endurance => "Endurance",
        F2QualificationPattern::TransitionShock => "TransitionShock",
    }
}

#[cfg(windows)]
fn qualification_gate_patterns(final_gate_passes: usize) -> Vec<F2QualificationPattern> {
    nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS
        .into_iter()
        .take(final_gate_passes)
        .collect()
}

#[cfg(windows)]
fn qualification_next_higher_candidate_index(rejected_index: usize) -> Option<usize> {
    rejected_index.checked_sub(1)
}

/// Run the optional final v8 boundary gate on ONE already-qualified candidate. This does not rediscover the
/// voltage ladder: a real failure rejects exactly this bin, and the caller moves one physical bin
/// higher before trying the v8 set again.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn gate_anchored_candidate_fsgl3(
    store: &SafeLoopStore,
    obs_store: &nidavellir_core::f2_observation::F2ObservationStore,
    qual_ctx: &mut crate::gpu_f2_sweep::ObsContext,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
    candidate: &AnchoredPositiveOffsetPlan,
    candidate_count: usize,
    unpruned_steps: usize,
    final_gate_dwell_ms: u64,
    final_gate_passes: usize,
    render_goldens: Option<RenderGoldens>,
    exact_apply: bool,
    stop: &std::sync::atomic::AtomicBool,
    logs: &mut Vec<String>,
    executed_steps: &mut usize,
    on_progress: &mut dyn FnMut(F2ClockDiscoveryProgress),
) -> F2QualificationOutcome {
    use std::sync::atomic::Ordering;

    use nidavellir_core::f2_observation::now_rfc3339;

    let Some(goldens) = render_goldens else {
        return F2QualificationOutcome::Aborted {
            stop_reason: "V8GoldenMissing".into(),
            retain_boot_flag: false,
        };
    };
    let patterns = qualification_gate_patterns(final_gate_passes);
    for (pattern_index, pattern) in patterns.iter().copied().enumerate() {
        let pass_index = (pattern_index + 1) as u32;
        let mut inconclusive_retries = 0usize;
        let mut clean_passes_after_inconclusive = 0usize;
        loop {
            if stop.load(Ordering::SeqCst) {
                return F2QualificationOutcome::Cancelled;
            }
            let mut validation_ops = RealF2MultiOps {
                store,
                curve: sane.to_vec(),
                candidates: vec![candidate.clone()],
                limits: *limits,
                target_mhz,
                baseline_offset_mhz: 0,
                prev_offset_override_mhz: None,
                dwell_ms: final_gate_dwell_ms,
                stress_purpose: if exact_apply {
                    F2StressPurpose::ApplyQualification(pattern, goldens)
                } else {
                    F2StressPurpose::V8Qualification(pattern, goldens)
                },
                cancel: Some(stop),
                cur: None,
            };
            if let Err(e) = validation_ops.select(0) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("V8PrecheckFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: None,
                line: format!(
                    "v8 {} {}/{}: {target_mhz} MHz @ {} mV ({} s)…",
                    qualification_pattern_label(pattern),
                    pass_index,
                    patterns.len(),
                    candidate.anchor.voltage_mv,
                    final_gate_dwell_ms / 1000
                ),
            });
            let mut report = run_confirmed_f2_step(&mut validation_ops);
            annotate_qualification_report(
                &mut report,
                F2QualificationStrength::Fsgl4,
                Some(pattern),
                pass_index,
                inconclusive_retries as u32,
            );
            logs.push(format!(
                "{target_mhz} MHz @ {} mV v8 {} {}/{}: {:?}",
                candidate.anchor.voltage_mv,
                qualification_pattern_label(pattern),
                pass_index,
                patterns.len(),
                report.outcome
            ));
            qual_ctx.timestamp = now_rfc3339();
            let observation = crate::gpu_f2_sweep::observation_from_anchored_step(
                qual_ctx,
                target_mhz,
                candidate,
                &report,
            );
            if let Err(e) = obs_store.append(&observation) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("ObservationPersistFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            *executed_steps = executed_steps.saturating_add(1);
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: Some(format!("{:?}", report.outcome)),
                line: format!(
                    "{target_mhz} MHz @ {} mV · v8 {} {}/{} → {:?} · aprendizado salvo",
                    candidate.anchor.voltage_mv,
                    qualification_pattern_label(pattern),
                    pass_index,
                    patterns.len(),
                    report.outcome
                ),
            });
            match &report.outcome {
                F2Outcome::Validated => {
                    if exact_apply && inconclusive_retries > 0 {
                        clean_passes_after_inconclusive += 1;
                        if !apply_qualification_pattern_complete(
                            inconclusive_retries,
                            clean_passes_after_inconclusive,
                        ) {
                            logs.push(format!(
                                "{target_mhz} MHz @ {} mV v8 {}: dívida inconclusiva preservada; exigindo mais um passe limpo consecutivo",
                                candidate.anchor.voltage_mv,
                                qualification_pattern_label(pattern)
                            ));
                            continue;
                        }
                    }
                    break;
                }
                F2Outcome::DeviceLost
                | F2Outcome::ResetFailed
                | F2Outcome::ArmFailed(_)
                | F2Outcome::ApplyFailed(_)
                | F2Outcome::VerifyFailed => {
                    return F2QualificationOutcome::Aborted {
                        stop_reason: format!("V8Aborted: {:?}", report.outcome),
                        retain_boot_flag: f2_outcome_retains_boot_flag(&report.outcome),
                    };
                }
                F2Outcome::Inconclusive => {
                    let may_retry = if exact_apply {
                        qualification_should_retry_inconclusive(inconclusive_retries)
                    } else {
                        inconclusive_retries == 0
                    };
                    if may_retry {
                        inconclusive_retries += 1;
                        clean_passes_after_inconclusive = 0;
                        logs.push(format!(
                            "{target_mhz} MHz @ {} mV v8 {} inconclusivo; {}",
                            candidate.anchor.voltage_mv,
                            qualification_pattern_label(pattern),
                            if exact_apply {
                                "dívida registrada, agora são exigidos dois passes limpos consecutivos"
                            } else {
                                "repetindo uma vez"
                            }
                        ));
                        continue;
                    }
                    return F2QualificationOutcome::Inconclusive;
                }
                other => return F2QualificationOutcome::Rejected(format!("{other:?}")),
            }
        }
    }
    // v14/v15 candidate-only STRESS gates at the EXACT Apply point, run ONLY at exact-Apply — the
    // frontier descent (exact_apply=false) never pays them. IN ORDER (fail cheap first):
    //   1. TransitionShock (~8 min): true-idle → heavy-slam cycles reproducing the game/benchmark
    //      LAUNCH transition (P-state exit + boost VF ramp + VRM load step) behind the observed
    //      in-game BusReset TDR cascade; the slam wall-time check fails Unstable at the pre-hang
    //      precursor, long before the ~2 s driver watchdog.
    //   2. Endurance (~20 min): one CONTINUOUS worst-realistic soak (sustained max-power +
    //      cap-slam + droop + mixed), no mid-soak reset, so thermal saturation truly accumulates.
    // Non-Validated ⇒ the exact point is rejected (fail closed). Each pass keeps the same
    // arm→apply→verify→dwell→reset motor + NVML clock ceiling + cooperative Stop.
    if exact_apply {
        for (pattern, dwell_ms, kind) in [
            (
                F2QualificationPattern::TransitionShock,
                F2_TRANSITION_SHOCK_DWELL_MS,
                "shock idle→slam",
            ),
            (
                F2QualificationPattern::Endurance,
                F2_ENDURANCE_QUALIFICATION_DWELL_MS,
                "soak contínuo",
            ),
        ] {
            let label = qualification_pattern_label(pattern);
            if stop.load(Ordering::SeqCst) {
                return F2QualificationOutcome::Cancelled;
            }
            let mut gate_ops = RealF2MultiOps {
                store,
                curve: sane.to_vec(),
                candidates: vec![candidate.clone()],
                limits: *limits,
                target_mhz,
                baseline_offset_mhz: 0,
                prev_offset_override_mhz: None,
                dwell_ms,
                stress_purpose: F2StressPurpose::ApplyQualification(pattern, goldens),
                cancel: Some(stop),
                cur: None,
            };
            if let Err(e) = gate_ops.select(0) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("{label}PrecheckFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: None,
                line: format!(
                    "v8 {label}: {target_mhz} MHz @ {} mV — {kind} ({} min)…",
                    candidate.anchor.voltage_mv,
                    dwell_ms / 60_000
                ),
            });
            let mut report = run_confirmed_f2_step(&mut gate_ops);
            annotate_qualification_report(
                &mut report,
                F2QualificationStrength::Fsgl4,
                Some(pattern),
                1,
                0,
            );
            logs.push(format!(
                "{target_mhz} MHz @ {} mV v8 {label} ({kind} {} min): {:?}",
                candidate.anchor.voltage_mv,
                dwell_ms / 60_000,
                report.outcome
            ));
            qual_ctx.timestamp = now_rfc3339();
            let observation = crate::gpu_f2_sweep::observation_from_anchored_step(
                qual_ctx,
                target_mhz,
                candidate,
                &report,
            );
            if let Err(e) = obs_store.append(&observation) {
                return F2QualificationOutcome::Aborted {
                    stop_reason: format!("ObservationPersistFailed: {e}"),
                    retain_boot_flag: false,
                };
            }
            *executed_steps = executed_steps.saturating_add(1);
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(candidate.anchor.voltage_mv),
                outcome: Some(format!("{:?}", report.outcome)),
                line: format!(
                    "{target_mhz} MHz @ {} mV · v8 {label} → {:?} · aprendizado salvo",
                    candidate.anchor.voltage_mv, report.outcome
                ),
            });
            if stop.load(Ordering::SeqCst) {
                return F2QualificationOutcome::Cancelled;
            }
            match &report.outcome {
                F2Outcome::Validated => {}
                F2Outcome::DeviceLost
                | F2Outcome::ResetFailed
                | F2Outcome::ArmFailed(_)
                | F2Outcome::ApplyFailed(_)
                | F2Outcome::VerifyFailed => {
                    return F2QualificationOutcome::Aborted {
                        stop_reason: format!("{label}Aborted: {:?}", report.outcome),
                        retain_boot_flag: f2_outcome_retains_boot_flag(&report.outcome),
                    };
                }
                F2Outcome::Inconclusive => return F2QualificationOutcome::Inconclusive,
                other => {
                    return F2QualificationOutcome::Rejected(format!(
                        "{label} {other:?}"
                    ))
                }
            }
        }
    }
    F2QualificationOutcome::Qualified
}

/// Qualify the exact `(target_mhz, apply_mv)` pair selected after the +Apply margin. Unlike the
/// frontier gate, this proof is stored as `ApplyQualification` evidence and an inconclusive attempt
/// creates debt: that pattern then needs two consecutive clean passes before it can qualify.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_confirmed_f2_apply_qualification(
    store: &SafeLoopStore,
    obs_store: &nidavellir_core::f2_observation::F2ObservationStore,
    run_id: &str,
    gpu_key: &str,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
    apply_mv: u32,
    reference_offset_mhz: i32,
    qualification_dwell_ms: u64,
    render_goldens: Option<RenderGoldens>,
    stop: &std::sync::atomic::AtomicBool,
    on_progress: &mut dyn FnMut(F2ClockDiscoveryProgress),
) -> F2ApplyQualificationSummary {
    use std::sync::atomic::Ordering;

    use nidavellir_core::f2_observation::{
        now_rfc3339, F2EvidenceKind, F2ObsMode, F2_QUALIFICATION_CONTRACT_VERSION,
    };

    let mut logs = Vec::new();
    if stop.load(Ordering::SeqCst) {
        return F2ApplyQualificationSummary {
            qualified: false,
            executed_steps: 0,
            aborted: false,
            cancelled: true,
            retain_boot_flag: false,
            stop_reason: "Cancelled".into(),
            logs,
        };
    }
    let candidate = match plan_f2_power_calibration_candidate(
        sane,
        target_mhz,
        apply_mv,
        reference_offset_mhz,
        limits,
    ) {
        Ok(candidate) => candidate,
        Err(e) => {
            return F2ApplyQualificationSummary {
                qualified: false,
                executed_steps: 0,
                aborted: true,
                cancelled: false,
                retain_boot_flag: false,
                stop_reason: format!("ApplyQualificationPlanFailed: {e}"),
                logs,
            }
        }
    };
    if let Some(reason) = confirmed_f2_refusal(
        &store.load_record(),
        store.is_boot_flag_armed(),
        Some(1),
        Some(&candidate.anchor),
        limits,
        target_mhz,
    ) {
        return F2ApplyQualificationSummary {
            qualified: false,
            executed_steps: 0,
            aborted: true,
            cancelled: false,
            retain_boot_flag: false,
            stop_reason: format!("ApplyQualificationSafetyGateRefused: {reason}"),
            logs,
        };
    }

    let mut ctx = crate::gpu_f2_sweep::ObsContext {
        run_id: run_id.to_string(),
        timestamp: now_rfc3339(),
        gpu_key: Some(gpu_key.to_string()),
        evidence_kind: F2EvidenceKind::ApplyQualification,
        discovery_contract_version: None,
        qualification_contract_version: Some(F2_QUALIFICATION_CONTRACT_VERSION),
        qualification_coverage: None,
        mode: F2ObsMode::ApplyQualification,
        requested_start_mv: Some(apply_mv),
        positive_offset_cap_mhz: limits.abs_max_offset_mhz,
    };
    let mut executed_steps = 0usize;
    let outcome = gate_anchored_candidate_fsgl3(
        store,
        obs_store,
        &mut ctx,
        sane,
        limits,
        target_mhz,
        &candidate,
        3,
        3,
        qualification_dwell_ms,
        // The exact-Apply gate must run the COMPLETE required pattern set: the p95/p99 publish
        // gates demand every pattern, so a shorter list (this was a hardcoded 3) discards a fully
        // passed soak as "no measurable sustained p95".
        nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS.len(),
        render_goldens,
        true,
        stop,
        &mut logs,
        &mut executed_steps,
        on_progress,
    );
    let (qualified, aborted, cancelled, retain_boot_flag, stop_reason) = match outcome {
        F2QualificationOutcome::Qualified => (
            true,
            false,
            false,
            false,
            "ExactApplyQualified".to_string(),
        ),
        F2QualificationOutcome::Rejected(reason) => (
            false,
            false,
            false,
            false,
            format!("ExactApplyRejected: {reason}"),
        ),
        F2QualificationOutcome::Inconclusive => (
            false,
            false,
            false,
            false,
            "ExactApplyInconclusive".to_string(),
        ),
        F2QualificationOutcome::Cancelled => {
            (false, false, true, false, "Cancelled".to_string())
        }
        F2QualificationOutcome::Aborted {
            stop_reason,
            retain_boot_flag,
        } => (false, true, false, retain_boot_flag, stop_reason),
    };
    F2ApplyQualificationSummary {
        qualified,
        executed_steps,
        aborted,
        cancelled,
        retain_boot_flag,
        stop_reason,
        logs,
    }
}

/// Measure a missing exact Apply bin with the same supervised PowerRender motor and discovery-v4
/// p99 consistency contract used by frontier descent. The qualified frontier is not changed and
/// this function contributes only card/profile power calibration; the separate post-synthesis
/// gate proves exact-Apply stability. Every raw attempt is persisted; no consensus remains neutral
/// and ineligible.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_confirmed_f2_power_calibration(
    store: &SafeLoopStore,
    obs_store: &nidavellir_core::f2_observation::F2ObservationStore,
    run_id: &str,
    gpu_key: &str,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
    apply_mv: u32,
    reference_offset_mhz: i32,
    power_limit_w: Option<f32>,
    discovery_dwell_ms: u64,
    stop: &std::sync::atomic::AtomicBool,
    on_progress: &mut dyn FnMut(F2ClockDiscoveryProgress),
) -> F2PowerCalibrationSummary {
    use std::sync::atomic::Ordering;

    use nidavellir_core::f2_observation::{
        is_current_discovery_evidence, now_rfc3339, F2ObsMode, F2ObsOutcome,
    };

    let mut logs = Vec::new();
    if stop.load(Ordering::SeqCst) {
        return F2PowerCalibrationSummary {
            confirmed: false,
            executed_steps: 0,
            aborted: false,
            retain_boot_flag: false,
            stop_reason: "Cancelled".into(),
            logs,
        };
    }
    let candidate = match plan_f2_power_calibration_candidate(
        sane,
        target_mhz,
        apply_mv,
        reference_offset_mhz,
        limits,
    ) {
        Ok(candidate) => candidate,
        Err(e) => {
            return F2PowerCalibrationSummary {
                confirmed: false,
                executed_steps: 0,
                aborted: true,
                retain_boot_flag: false,
                stop_reason: format!("CalibrationPlanFailed: {e}"),
                logs,
            }
        }
    };
    if let Some(reason) = confirmed_f2_refusal(
        &store.load_record(),
        store.is_boot_flag_armed(),
        Some(1),
        Some(&candidate.anchor),
        limits,
        target_mhz,
    ) {
        return F2PowerCalibrationSummary {
            confirmed: false,
            executed_steps: 0,
            aborted: true,
            retain_boot_flag: false,
            stop_reason: format!("CalibrationSafetyGateRefused: {reason}"),
            logs,
        };
    }

    let previous_confirmed_power = obs_store
        .query_by_target_for_gpu(target_mhz, gpu_key)
        .into_iter()
        .filter(|observation| {
            is_current_discovery_evidence(observation)
                && observation.reset_to_stock_ok
                && observation.boot_flag_cleared
                && !observation.thermal_throttled
                && matches!(
                    observation.outcome,
                    F2ObsOutcome::Validated | F2ObsOutcome::PowerBoundClockDrop
                )
        })
        .filter_map(|observation| {
            Some((
                observation.anchor_mv.abs_diff(apply_mv),
                observation.power_p99_w?,
                observation.sustained_clock_mhz?,
            ))
        })
        .min_by_key(|(distance, _, _)| *distance)
        .map(|(_, power_p99, p5)| (power_p99, p5));

    let mut ops = RealF2Ops {
        store,
        curve: sane.to_vec(),
        candidate: candidate.anchor,
        anchored: Some(candidate.clone()),
        mode: UndervoltMode::Anchored,
        limits: *limits,
        target_mhz,
        prev_offset_mhz: reference_offset_mhz,
        dwell_ms: discovery_dwell_ms,
        stress_purpose: F2StressPurpose::PowerDiscovery,
        cancel: Some(stop),
    };
    on_progress(F2ClockDiscoveryProgress {
        target_mhz,
        planned_steps: POWER_P99_MAX_ATTEMPTS,
        unpruned_steps: POWER_P99_MAX_ATTEMPTS,
        anchor_mv: Some(apply_mv),
        outcome: None,
        line: format!(
            "Calibração p99: {target_mhz} MHz @ {apply_mv} mV — PowerRender no bin exato de Apply…"
        ),
    });
    let initial_report = run_confirmed_f2_step(&mut ops);
    let rechecked =
        f2_power_p99_requires_recheck(previous_confirmed_power, &initial_report);
    let mut attempts = vec![initial_report];
    if rechecked {
        logs.push(format!(
            "{target_mhz} MHz @ {apply_mv} mV calibration: salto p99 anômalo; repetindo o mesmo bin"
        ));
        while attempts.len() < POWER_P99_MAX_ATTEMPTS
            && !stop.load(Ordering::SeqCst)
        {
            let attempt_number = attempts.len() + 1;
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: POWER_P99_MAX_ATTEMPTS,
                unpruned_steps: POWER_P99_MAX_ATTEMPTS,
                anchor_mv: Some(apply_mv),
                outcome: None,
                line: format!(
                    "Calibração p99 {attempt_number}/{POWER_P99_MAX_ATTEMPTS}: {target_mhz} MHz @ {apply_mv} mV — repetindo PowerRender…"
                ),
            });
            let repeated = run_confirmed_f2_step(&mut ops);
            let terminal_failure = matches!(
                repeated.outcome,
                F2Outcome::DeviceLost
                    | F2Outcome::ResetFailed
                    | F2Outcome::ArmFailed(_)
                    | F2Outcome::ApplyFailed(_)
                    | F2Outcome::VerifyFailed
                    | F2Outcome::SilentError
                    | F2Outcome::Unstable
            );
            attempts.push(repeated);
            if terminal_failure
                || f2_power_attempts_have_consistent_pair(&attempts)
            {
                break;
            }
        }
    }

    let conservative_p99 = f2_confirm_power_attempts(&mut attempts, rechecked);
    let mut aggregate = f2_aggregate_power_attempts(&attempts, conservative_p99);
    let near_cap = f2_near_power_limit(
        aggregate.power_p99_w,
        power_limit_w,
        aggregate.power_capped_frac,
    );
    aggregate.outcome = f2_power_bound_clock_drop(&aggregate.outcome, near_cap);
    for attempt in &mut attempts {
        if attempt.power_p99_confirmed {
            attempt.outcome = f2_power_bound_clock_drop(&attempt.outcome, near_cap);
        }
    }

    let mut ctx = crate::gpu_f2_sweep::ObsContext {
        run_id: run_id.to_string(),
        timestamp: now_rfc3339(),
        gpu_key: Some(gpu_key.to_string()),
        evidence_kind: nidavellir_core::f2_observation::F2EvidenceKind::Discovery,
        discovery_contract_version: Some(
            nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
        ),
        qualification_contract_version: None,
        qualification_coverage: None,
        mode: F2ObsMode::LadderSweep,
        requested_start_mv: Some(apply_mv),
        positive_offset_cap_mhz: limits.abs_max_offset_mhz,
    };
    let mut executed_steps = 0usize;
    for (attempt_index, attempt) in attempts.iter().enumerate() {
        ctx.timestamp = now_rfc3339();
        let observation = crate::gpu_f2_sweep::observation_from_anchored_step(
            &ctx,
            target_mhz,
            &candidate,
            attempt,
        );
        if let Err(e) = obs_store.append(&observation) {
            return F2PowerCalibrationSummary {
                confirmed: false,
                executed_steps,
                aborted: true,
                retain_boot_flag: f2_outcome_retains_boot_flag(&aggregate.outcome),
                stop_reason: format!("CalibrationObservationPersistFailed: {e}"),
                logs,
            };
        }
        executed_steps = executed_steps.saturating_add(1);
        logs.push(format!(
            "{target_mhz} MHz @ {apply_mv} mV calibration attempt {}/{}: {:?}, p5={:?} MHz, power_p99={:?} W, confirmed={}",
            attempt_index + 1,
            attempts.len(),
            attempt.outcome,
            attempt.p5_clock_mhz,
            attempt.power_p99_w,
            attempt.power_p99_confirmed
        ));
        on_progress(F2ClockDiscoveryProgress {
            target_mhz,
            planned_steps: POWER_P99_MAX_ATTEMPTS,
            unpruned_steps: POWER_P99_MAX_ATTEMPTS,
            anchor_mv: Some(apply_mv),
            outcome: Some(format!("{:?}", attempt.outcome)),
            line: format!(
                "{target_mhz} MHz @ {apply_mv} mV · calibração p99 {}/{} → {:?} · p99 {:.0} W · {}",
                attempt_index + 1,
                attempts.len(),
                attempt.outcome,
                attempt.power_p99_w.unwrap_or(0.0),
                if attempt.power_p99_confirmed {
                    "medição confirmada"
                } else {
                    "medição inconclusiva"
                }
            ),
        });
    }

    let confirmed = conservative_p99.is_some()
        && matches!(
            aggregate.outcome,
            F2Outcome::Validated | F2Outcome::PowerBoundClockDrop
        );
    let aborted = matches!(
        aggregate.outcome,
        F2Outcome::DeviceLost
            | F2Outcome::ResetFailed
            | F2Outcome::ArmFailed(_)
            | F2Outcome::ApplyFailed(_)
            | F2Outcome::VerifyFailed
    );
    let retain_boot_flag = f2_outcome_retains_boot_flag(&aggregate.outcome);
    let stop_reason = if confirmed {
        format!(
            "Confirmed p99 {:.3} W",
            conservative_p99.unwrap_or_default()
        )
    } else if stop.load(Ordering::SeqCst) {
        "Cancelled".into()
    } else {
        format!("{:?}", aggregate.outcome)
    };
    F2PowerCalibrationSummary {
        confirmed,
        executed_steps,
        aborted,
        retain_boot_flag,
        stop_reason,
        logs,
    }
}

/// Run the live F2 discovery for one real target clock. Unlike the legacy CLI motor, this has no
/// arbitrary candidate cap: confirmed power-bound spans may skip bounded physical bins, while the
/// first approved off-cap point and every recovery bracket return to exact local discovery. A
/// pre-sustain clock drop continues only while p99 remains at 99–100% of the numeric power limit.
/// Every executed candidate still runs arm→write→verify→dwell→reset and is persisted immediately;
/// any untrustworthy recovery state aborts the whole forge.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_confirmed_f2_clock_discovery(
    store: &SafeLoopStore,
    obs_store: &nidavellir_core::f2_observation::F2ObservationStore,
    run_id: &str,
    gpu_key: &str,
    sane: &[(usize, u32, u32)],
    limits: &PositiveOffsetLimits,
    target_mhz: u32,
    start_mv: Option<u32>,
    power_limit_w: Option<f32>,
    discovery_dwell_ms: u64,
    qualification_dwell_ms: u64,
    qualification_passes: usize,
    final_gate_dwell_ms: u64,
    final_gate_passes: usize,
    render_goldens: Option<RenderGoldens>,
    stop: &std::sync::atomic::AtomicBool,
    on_progress: &mut dyn FnMut(F2ClockDiscoveryProgress),
) -> F2ClockDiscoverySummary {
    use std::sync::atomic::Ordering;

    use nidavellir_core::f2_observation::{
        first_bad_for_target, is_current_discovery_evidence, is_current_qualification_pass,
        last_discovery_good_for_target, now_rfc3339, F2ObsMode,
    };

    let mut logs = Vec::new();
    let unpruned_descent =
        plan_anchored_undervolt_descent(sane, target_mhz, None, limits, usize::MAX);
    let unpruned_steps = unpruned_descent.candidates.len();
    let mut descent =
        plan_anchored_undervolt_descent(sane, target_mhz, start_mv, limits, usize::MAX);
    let prior = obs_store.query_by_target_for_gpu(target_mhz, gpu_key);
    // A driver/firmware update can change the static VF table without changing the GPU UUID. Resume
    // only from observations whose exact anchor/base/offset still exists in the current plan.
    let compatible_prior: Vec<_> = prior
        .into_iter()
        .filter(|observation| {
            f2_observation_matches_current_candidate(
                &unpruned_descent.candidates,
                observation.anchor_mv,
                observation.base_mhz,
                observation.offset_mhz,
            )
        })
        .collect();
    let prior_good_observation =
        last_discovery_good_for_target(&compatible_prior, target_mhz);
    let prior_good_mv = prior_good_observation.map(|o| o.anchor_mv);
    let prior_reference_offset_mhz = if start_mv.is_some() {
        prior_good_observation.map(|o| o.offset_mhz).unwrap_or(0)
    } else {
        0
    };
    if start_mv.is_some() && descent.candidates.is_empty() && prior_good_observation.is_some() {
        // A same-target v4 boundary may be far enough from stock that replanning directly at the
        // predicted start hits the +15 MHz progression guard. Reuse only the already-compatible
        // historical offset as the writer's cross-run baseline, while still executing fresh
        // PowerRender + current v8 evidence at every selected bin.
        descent = unpruned_descent.clone();
        descent.start_mv = start_mv;
        if let Some(start_mv) = start_mv {
            descent
                .candidates
                .retain(|candidate| candidate.anchor.voltage_mv <= start_mv);
        }
        logs.push(format!(
            "{target_mhz} MHz: previsão v4 usa offset histórico compatível +{prior_reference_offset_mhz} MHz apenas como limite de progressão; estabilidade será medida novamente"
        ));
    }
    let prior_bad_mv = first_bad_for_target(&compatible_prior, target_mhz).map(|o| o.anchor_mv);
    let prior_power_bound_mv = compatible_prior
        .iter()
        .filter(|o| {
            is_current_discovery_evidence(o)
                && matches!(
                    o.outcome,
                    nidavellir_core::f2_observation::F2ObsOutcome::PowerBoundClockDrop
                )
        })
        .map(|o| o.anchor_mv)
        .min();
    // Standard/Long must not qualify a boundary that exists only because a previous run accepted it.
    // Prior positives may guide Fast/resume, but qualification modes must rediscover the boundary with
    // the current PowerRender discovery contract before the FailureSeekingGameLoop is allowed to run.
    let refresh_discovery_for_qualification = qualification_passes > 0 || final_gate_passes > 0;
    let resume_good_mv = if refresh_discovery_for_qualification {
        None
    } else {
        prior_good_mv
    };
    let resume_power_bound_mv = if refresh_discovery_for_qualification {
        None
    } else {
        prior_power_bound_mv
    };
    if refresh_discovery_for_qualification && prior_good_mv.is_some() {
        logs.push(format!(
            "{target_mhz} MHz: Standard/Long exige redescoberta fresca; ponto antigo {:?} mV não será qualificado diretamente",
            prior_good_mv
        ));
    }
    // Resume below the deepest reset-clean point already observed on this exact GPU. A known
    // good+bad bracket is already complete; Long may still independently revalidate its best point.
    let resume_below_mv = resume_f2_candidates(
        &mut descent.candidates,
        resume_good_mv,
        prior_bad_mv,
        resume_power_bound_mv,
    );
    if prior_bad_mv.is_some() {
        logs.push(format!(
            "{target_mhz} MHz: retomando fronteira já delimitada em {:?}/{:?} mV",
            resume_good_mv, prior_bad_mv
        ));
    } else if let Some(resume_below_mv) = resume_below_mv {
        logs.push(format!(
            "{target_mhz} MHz: retomando abaixo de {resume_below_mv} mV; pontos mais altos já confirmados"
        ));
    }
    let candidate_count = descent.candidates.len();
    on_progress(F2ClockDiscoveryProgress {
        target_mhz,
        planned_steps: candidate_count,
        unpruned_steps,
        anchor_mv: None,
        outcome: None,
        line: match start_mv {
            Some(mv) => format!(
                "{target_mhz} MHz: {candidate_count} dwell(s) planejados; início conservador em {mv} mV ({unpruned_steps} sem reaproveitamento)."
            ),
            None => format!(
                "{target_mhz} MHz: {candidate_count} dwell(s) planejados ({unpruned_steps} sem reaproveitamento)."
            ),
        },
    });

    let rec = store.load_record();
    let armed = store.is_boot_flag_armed();
    let hardware_work_count = candidate_count;
    if hardware_work_count > 0
        && confirmed_f2_multi_refusal(
            &rec,
            armed,
            Some(hardware_work_count),
            hardware_work_count,
            hardware_work_count,
        )
        .is_some()
    {
        return F2ClockDiscoverySummary {
            sustainable: false,
            last_good_mv: None,
            first_bad_mv: None,
            next_clock_start_mv: None,
            conservative_start_mv: None,
            warm_start_rejected: false,
            executed_steps: 0,
            completed: false,
            aborted: true,
            retain_boot_flag: false,
            stop_reason: "SafetyGateRefused".into(),
            logs,
        };
    }
    let mut ops = RealF2MultiOps {
        store,
        curve: sane.to_vec(),
        candidates: descent.candidates.clone(),
        limits: *limits,
        target_mhz,
        baseline_offset_mhz: 0,
        prev_offset_override_mhz: None,
        dwell_ms: discovery_dwell_ms,
        stress_purpose: F2StressPurpose::PowerDiscovery,
        cancel: Some(stop),
        cur: None,
    };
    let mut ctx = crate::gpu_f2_sweep::ObsContext {
        run_id: run_id.to_string(),
        timestamp: now_rfc3339(),
        gpu_key: Some(gpu_key.to_string()),
        evidence_kind: nidavellir_core::f2_observation::F2EvidenceKind::Discovery,
        discovery_contract_version: Some(
            nidavellir_core::f2_observation::F2_DISCOVERY_CONTRACT_VERSION,
        ),
        qualification_contract_version: None,
        qualification_coverage: None,
        mode: F2ObsMode::LadderSweep,
        requested_start_mv: start_mv,
        positive_offset_cap_mhz: limits.abs_max_offset_mhz,
    };

    let mut had_sustainable = resume_good_mv.is_some();
    let mut completed = !refresh_discovery_for_qualification
        && (prior_bad_mv.is_some()
            || (candidate_count == 0
                && (resume_good_mv.is_some()
                    || prior_bad_mv.is_some()
                    || resume_power_bound_mv.is_some())));
    let mut aborted = false;
    let mut retain_boot_flag = false;
    let mut executed_steps = 0usize;
    let mut previous_confirmed_power: Option<(f32, u32)> = None;
    let mut stop_reason = if prior_bad_mv.is_some() {
        "KnownBoundaryResumed".to_string()
    } else {
        "PhysicalFloorReached".to_string()
    };

    if candidate_count == 0
        && resume_good_mv.is_none()
        && prior_bad_mv.is_none()
        && resume_power_bound_mv.is_none()
    {
        if start_mv.is_some() {
            completed = true;
            stop_reason = "WarmStartNoPhysicalCandidates".into();
        } else {
            aborted = true;
            stop_reason = "NoPhysicalCandidates".into();
        }
    }
    if refresh_discovery_for_qualification && candidate_count == 0 && prior_good_mv.is_some() {
        completed = false;
        stop_reason = "FreshDiscoveryNoPhysicalCandidates".into();
        logs.push(format!(
            "{target_mhz} MHz: qualification recusou reaproveitar ponto antigo, mas não há bin físico disponível para redescoberta fresca"
        ));
    }

    // Qualification evidence context, separate from the Discovery `ctx` used by PowerRender.
    // The v8 three-pattern set is the default per-candidate qualifier during descent. The optional final gate is kept
    // dormant for now and shares the same boundary shape.
    let mut qual_ctx = ctx.clone();
    qual_ctx.evidence_kind = nidavellir_core::f2_observation::F2EvidenceKind::Qualification;
    qual_ctx.discovery_contract_version = None;
    qual_ctx.qualification_contract_version =
        Some(nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION);
    qual_ctx.qualification_coverage = None;
    let mut last_qualified_index: Option<usize> = None;
    let mut qualification_margin_history = F2QualificationMarginHistory::default();
    let mut current_index = 0usize;
    let mut reference_offset_mhz = prior_reference_offset_mhz;
    let mut jumped_from_index: Option<usize> = None;
    let mut recovery_bracket: Option<(usize, usize)> = None;
    let mut known_failed_index: Option<usize> = None;
    let mut local_sequential = false;

    while current_index < candidate_count {
        let i = current_index;
        if known_failed_index == Some(i) {
            completed = true;
            stop_reason = "AdaptiveKnownFailureBoundary".into();
            logs.push(format!(
                "{target_mhz} MHz @ {} mV: fronteira já medida como falha durante recuperação; sem novo dwell",
                descent.candidates[i].anchor.voltage_mv
            ));
            break;
        }
        if stop.load(Ordering::SeqCst) {
            stop_reason = "Cancelled".into();
            break;
        }
        ops.prev_offset_override_mhz = Some(reference_offset_mhz);
        // A blacklisted NEXT candidate during the frontier DESCENT is BOUNDARY knowledge ("a prior
        // run's crash/TDR proved this (clock, vf_bin) unsafe — don't undervolt this low here"), NOT a
        // live safety emergency. Treat it like reaching the physical floor: stop THIS clock at the last
        // validated bin and let the frontier continue, rather than aborting the whole forge and
        // publishing nothing. The Safe Loop blacklist is DURABLE, so this is exactly how a run resumed
        // after a TDR re-enters — the accumulated blacklist must CAP the descent, never kill it. The
        // genuine live safety refusals (Safe Mode active / boot flag already armed) still hard-abort via
        // `select` below; those are current-state emergencies, not boundary knowledge.
        if candidate_blacklisted(
            &ops.store.load_record(),
            ops.target_mhz,
            &descent.candidates[i].anchor,
        ) {
            completed = true;
            stop_reason = "BlacklistedBoundary".into();
            logs.push(format!(
                "{target_mhz} MHz @ {} mV: próximo candidato na blacklist do Safe Loop (falha/TDR de execução anterior) — fronteira de segurança; parando acima sem novo dwell.",
                descent.candidates[i].anchor.voltage_mv
            ));
            break;
        }
        if let Err(e) = ops.select(i) {
            aborted = true;
            stop_reason = format!("SafetyPrecheckFailed: {e}");
            break;
        }
        let anchor_mv = descent.candidates[i].anchor.voltage_mv;
        on_progress(F2ClockDiscoveryProgress {
            target_mhz,
            planned_steps: candidate_count,
            unpruned_steps,
            anchor_mv: Some(anchor_mv),
            outcome: None,
            line: format!("Testing {target_mhz} MHz @ {anchor_mv} mV — dwell em andamento…"),
        });
        let initial_report = run_confirmed_f2_step(&mut ops);
        let rechecked =
            f2_power_p99_requires_recheck(previous_confirmed_power, &initial_report);
        let mut attempts = vec![initial_report];
        if rechecked {
            logs.push(format!(
                "{target_mhz} MHz @ {anchor_mv} mV: salto p99 anômalo detectado; repetindo o mesmo bin até {} tentativas reset-clean",
                POWER_P99_MAX_ATTEMPTS
            ));
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(anchor_mv),
                outcome: None,
                line: format!(
                    "{target_mhz} MHz @ {anchor_mv} mV → p99 anômalo; repetindo exatamente o mesmo bin…"
                ),
            });
            while attempts.len() < POWER_P99_MAX_ATTEMPTS {
                let attempt_number = attempts.len() + 1;
                on_progress(F2ClockDiscoveryProgress {
                    target_mhz,
                    planned_steps: candidate_count,
                    unpruned_steps,
                    anchor_mv: Some(anchor_mv),
                    outcome: None,
                    line: format!(
                        "Reteste p99 {attempt_number}/{POWER_P99_MAX_ATTEMPTS}: {target_mhz} MHz @ {anchor_mv} mV — dwell em andamento…"
                    ),
                });
                let repeated = run_confirmed_f2_step(&mut ops);
                let terminal_safety_failure = matches!(
                    repeated.outcome,
                    F2Outcome::DeviceLost
                        | F2Outcome::ResetFailed
                        | F2Outcome::ArmFailed(_)
                        | F2Outcome::ApplyFailed(_)
                        | F2Outcome::VerifyFailed
                );
                attempts.push(repeated);
                if terminal_safety_failure
                    || f2_power_attempts_have_consistent_pair(&attempts)
                {
                    break;
                }
            }
        }
        let conservative_p99 = f2_confirm_power_attempts(&mut attempts, rechecked);
        let mut report = f2_aggregate_power_attempts(&attempts, conservative_p99);
        let near_cap =
            f2_near_power_limit(report.power_p99_w, power_limit_w, report.power_capped_frac);
        report.outcome = f2_power_bound_clock_drop(&report.outcome, near_cap);
        for attempt in &mut attempts {
            if attempt.power_p99_confirmed {
                attempt.outcome = f2_power_bound_clock_drop(&attempt.outcome, near_cap);
            }
        }

        if let (Some(power_p99), Some(p5)) = (conservative_p99, report.p5_clock_mhz) {
            previous_confirmed_power = Some((power_p99, p5));
        }
        logs.push(format!(
            "{target_mhz} MHz @ {anchor_mv} mV: {:?}, attempts={}, p5={:?} MHz, power_avg={:?} W, power_p99_conservative={:?} W, cap_near={near_cap}",
            report.outcome,
            attempts.len(),
            report.p5_clock_mhz,
            report.power_w,
            conservative_p99
        ));
        for (attempt_index, attempt) in attempts.iter().enumerate() {
            logs.push(format!(
                "{target_mhz} MHz @ {anchor_mv} mV attempt {}/{}: {:?}, p5={:?} MHz, power_avg={:?} W, power_p99={:?} W, confirmed={}",
                attempt_index + 1,
                attempts.len(),
                attempt.outcome,
                attempt.p5_clock_mhz,
                attempt.power_w,
                attempt.power_p99_w,
                attempt.power_p99_confirmed
            ));
            ctx.timestamp = now_rfc3339();
            let observation = crate::gpu_f2_sweep::observation_from_anchored_step(
                &ctx,
                target_mhz,
                &descent.candidates[i],
                attempt,
            );
            if let Err(e) = obs_store.append(&observation) {
                aborted = true;
                stop_reason = format!("ObservationPersistFailed: {e}");
                break;
            }
            executed_steps = executed_steps.saturating_add(1);
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(anchor_mv),
                outcome: Some(format!("{:?}", attempt.outcome)),
                line: format!(
                    "{target_mhz} MHz @ {anchor_mv} mV · p99 {}/{} → {:?} · p5 {} MHz · p99 {:.0} W · {}",
                    attempt_index + 1,
                    attempts.len(),
                    attempt.outcome,
                    attempt.p5_clock_mhz.unwrap_or(0),
                    attempt.power_p99_w.unwrap_or(0.0),
                    if attempt.power_p99_confirmed {
                        "medição confirmada"
                    } else {
                        "medição inconclusiva"
                    }
                ),
            });
        }
        if aborted {
            break;
        }
        if rechecked {
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(anchor_mv),
                outcome: Some(format!("{:?}", report.outcome)),
                line: match conservative_p99 {
                    Some(power) => format!(
                        "{target_mhz} MHz @ {anchor_mv} mV → consenso p99 confirmado; valor conservador {power:.0} W"
                    ),
                    None => format!(
                        "{target_mhz} MHz @ {anchor_mv} mV → p99 inconclusivo após {} tentativa(s); bin inelegível",
                        attempts.len()
                    ),
                },
            });
        }

        let decision = f2_discovery_decision(&report.outcome, had_sustainable, near_cap);
        if matches!(decision, F2DiscoveryDecision::MarkSustainableAndContinue) {
            had_sustainable = true;
        }
        if report.reset_ok == Some(true)
            && report.boot_flag_cleared
            && matches!(
                report.outcome,
                F2Outcome::Validated | F2Outcome::PowerBoundClockDrop
            )
        {
            reference_offset_mhz = descent.candidates[i].anchor.offset_mhz;
        }
        let arrival_jump_origin = jumped_from_index;
        if f2_reset_clean_discovery_failure(decision, &report) {
            let shallower_safe_index = recovery_bracket
                .map(|(safe, _)| safe)
                .or(jumped_from_index);
            if let Some(safe_index) = shallower_safe_index.filter(|safe| i > safe + 1) {
                recovery_bracket = Some((safe_index, i));
                jumped_from_index = None;
                if let Some(midpoint) = f2_recovery_midpoint(safe_index, i) {
                    logs.push(format!(
                        "{target_mhz} MHz: falha reset-clean após salto {}→{}; recuperando para cima no bin intermediário {} mV",
                        descent.candidates[safe_index].anchor.voltage_mv,
                        anchor_mv,
                        descent.candidates[midpoint].anchor.voltage_mv
                    ));
                    on_progress(F2ClockDiscoveryProgress {
                        target_mhz,
                        planned_steps: candidate_count,
                        unpruned_steps,
                        anchor_mv: Some(descent.candidates[midpoint].anchor.voltage_mv),
                        outcome: None,
                        line: format!(
                            "{target_mhz} MHz: recuperação ascendente segura → {} mV (bisseção do intervalo conhecido)",
                            descent.candidates[midpoint].anchor.voltage_mv
                        ),
                    });
                    current_index = midpoint;
                    continue;
                }
            }
        } else {
            jumped_from_index = None;
        }
        match decision {
            F2DiscoveryDecision::ContinueVoltage
            | F2DiscoveryDecision::MarkSustainableAndContinue => {
                // Only an actual sustained (Validated) dwell is a qualification candidate; a pre-sustain
                // power-bound clock drop just keeps the descent going lower.
                if f2_should_qualify_discovery_candidate(
                    &report.outcome,
                    near_cap,
                    qualification_passes,
                ) {
                    // Standard/Long: qualify THIS bin with all v8 patterns before going any
                    // deeper. If it passes we descend one real bin lower (next iteration's PowerRender
                    // measures its power and gates the next qualification); if it fails we stop here with
                    // the last qualified bin as the boundary — the heavy qualifier never runs more than
                    // one bin below a proven point, so an over-aggressive bin can no longer TDR here.
                    let qualification_outcome = qualify_anchored_candidate(
                        store,
                        obs_store,
                        &mut qual_ctx,
                        sane,
                        limits,
                        target_mhz,
                        &descent.candidates[i],
                        candidate_count,
                        unpruned_steps,
                        qualification_dwell_ms,
                        qualification_passes,
                        &mut qualification_margin_history,
                        render_goldens,
                        stop,
                        &mut logs,
                        &mut executed_steps,
                        on_progress,
                    );
                    let qualification_completed = qualification_outcome.completes_clock();
                    match qualification_outcome {
                        F2QualificationOutcome::Qualified => {
                            last_qualified_index = Some(i);
                            local_sequential = true;
                            if let Some((_, failed_index)) = recovery_bracket.take() {
                                known_failed_index = Some(failed_index);
                                logs.push(format!(
                                    "{target_mhz} MHz @ {anchor_mv} mV: recuperação encontrou ponto qualificado; fronteira volta a descer bin a bin"
                                ));
                            }
                            completed = qualification_completed;
                            stop_reason = "Qualified".into();
                            // Fall through: descend one real bin lower on the next iteration.
                        }
                        F2QualificationOutcome::Rejected(reason) => {
                            let shallower_safe_index = recovery_bracket
                                .map(|(safe, _)| safe)
                                .or(arrival_jump_origin);
                            if let Some(safe_index) =
                                shallower_safe_index.filter(|safe| i > safe + 1)
                            {
                                recovery_bracket = Some((safe_index, i));
                                jumped_from_index = None;
                                reference_offset_mhz =
                                    descent.candidates[safe_index].anchor.offset_mhz;
                                if let Some(midpoint) = f2_recovery_midpoint(safe_index, i) {
                                    logs.push(format!(
                                        "{target_mhz} MHz @ {anchor_mv} mV: v8 rejeitou após salto ({reason}); recuperando para cima em {} mV",
                                        descent.candidates[midpoint].anchor.voltage_mv
                                    ));
                                    current_index = midpoint;
                                    continue;
                                }
                            }
                            // A reset-clean v8 rejection completes only this clock. Even when no
                            // shallower bin qualified, the outer ladder must continue to a lower
                            // target and discover the real Cmax instead of aborting the whole Forge.
                            completed = qualification_completed;
                            stop_reason = if last_qualified_index.is_some() {
                                format!(
                                    "QualifiedBoundary: {anchor_mv} mV rejeitado abaixo do último qualificado ({reason})"
                                )
                            } else {
                                format!("QualificationRejected: {reason}")
                            };
                            break;
                        }
                        F2QualificationOutcome::Inconclusive => {
                            let shallower_safe_index = recovery_bracket
                                .map(|(safe, _)| safe)
                                .or(arrival_jump_origin);
                            if let Some(safe_index) =
                                shallower_safe_index.filter(|safe| i > safe + 1)
                            {
                                recovery_bracket = Some((safe_index, i));
                                jumped_from_index = None;
                                reference_offset_mhz =
                                    descent.candidates[safe_index].anchor.offset_mhz;
                                if let Some(midpoint) = f2_recovery_midpoint(safe_index, i) {
                                    logs.push(format!(
                                        "{target_mhz} MHz @ {anchor_mv} mV: v8 inconclusivo após salto; recuperando para cima em {} mV",
                                        descent.candidates[midpoint].anchor.voltage_mv
                                    ));
                                    current_index = midpoint;
                                    continue;
                                }
                            }
                            // Coverage ambiguity is local to this clock. Preserve any shallower
                            // qualified boundary, skip the remainder of this clock and let the
                            // outer multi-clock Forge continue.
                            completed = qualification_completed;
                            stop_reason = if last_qualified_index.is_some() {
                                "QualifiedBoundaryInconclusiveDeeper".into()
                            } else {
                                "QualificationInconclusiveSkippedClock".into()
                            };
                            break;
                        }
                        F2QualificationOutcome::Cancelled => {
                            completed = false;
                            stop_reason = "CancelledDuringQualification".into();
                            break;
                        }
                        F2QualificationOutcome::Aborted {
                            stop_reason: reason,
                            retain_boot_flag: retain,
                        } => {
                            aborted = true;
                            retain_boot_flag |= retain;
                            stop_reason = reason;
                            break;
                        }
                    }
                }
                if matches!(report.outcome, F2Outcome::Validated)
                    && near_cap
                    && qualification_passes > 0
                {
                    logs.push(format!(
                        "{target_mhz} MHz @ {anchor_mv} mV: Validated ainda no cap (p99 {:?} W); v8 adiado e descida continua",
                        report.power_p99_w
                    ));
                }
                if matches!(report.outcome, F2Outcome::Validated)
                    && !near_cap
                    && qualification_passes == 0
                {
                    local_sequential = true;
                    if let Some((_, failed_index)) = recovery_bracket.take() {
                        known_failed_index = Some(failed_index);
                    }
                }
                if let Some((_, failed_index)) = recovery_bracket {
                    if !local_sequential {
                        recovery_bracket = Some((i, failed_index));
                        if let Some(midpoint) = f2_recovery_midpoint(i, failed_index) {
                            logs.push(format!(
                                "{target_mhz} MHz: recuperação estreitou o intervalo seguro/falha para {}–{} mV; próximo teste {} mV",
                                anchor_mv,
                                descent.candidates[failed_index].anchor.voltage_mv,
                                descent.candidates[midpoint].anchor.voltage_mv
                            ));
                            current_index = midpoint;
                            continue;
                        }
                        completed = true;
                        stop_reason = "AdaptiveRecoveryNoSustainablePoint".into();
                        logs.push(format!(
                            "{target_mhz} MHz: recuperação terminou entre bin power-bound {anchor_mv} mV e falha adjacente {} mV",
                            descent.candidates[failed_index].anchor.voltage_mv
                        ));
                        break;
                    }
                }
                // Fast (qualification_passes == 0) or a pre-sustain power-bound drop: keep descending.
                // Fast leaves the deepest PowerRender-good point provisional; discovery observations
                // carry it for synthesis.
            }
            F2DiscoveryDecision::NextClockUnsustainable => {
                completed = true;
                stop_reason = "OffCapClockDropBeforeSustain".into();
                break;
            }
            F2DiscoveryDecision::BoundaryFound => {
                completed = true;
                stop_reason = format!("{:?}", report.outcome);
                break;
            }
            F2DiscoveryDecision::NextClockAfterFailure => {
                completed = true;
                stop_reason = format!("{:?}BeforeSustain", report.outcome);
                break;
            }
            F2DiscoveryDecision::AbortForge => {
                aborted = true;
                retain_boot_flag = f2_outcome_retains_boot_flag(&report.outcome);
                stop_reason = format!("{:?}", report.outcome);
                break;
            }
        }
        let next_index = if local_sequential || !near_cap {
            i.saturating_add(1)
        } else {
            f2_adaptive_power_bound_next_index(
                &descent.candidates,
                i,
                target_mhz,
                report.p5_clock_mhz,
                reference_offset_mhz,
                limits.step_max_offset_mhz,
            )
        };
        if next_index > i + 1 {
            jumped_from_index = Some(i);
            logs.push(format!(
                "{target_mhz} MHz @ {anchor_mv} mV: região power-bound confirmada (p5 {:?} MHz, p99 {:?} W); salto adaptativo de {} bins para {} mV",
                report.p5_clock_mhz,
                report.power_p99_w,
                next_index - i,
                descent.candidates[next_index].anchor.voltage_mv
            ));
            on_progress(F2ClockDiscoveryProgress {
                target_mhz,
                planned_steps: candidate_count,
                unpruned_steps,
                anchor_mv: Some(descent.candidates[next_index].anchor.voltage_mv),
                outcome: None,
                line: format!(
                    "{target_mhz} MHz: power-bound sustentado; pulando {} bins com limites de 25 mV/+{} MHz → {} mV",
                    next_index - i,
                    limits.step_max_offset_mhz,
                    descent.candidates[next_index].anchor.voltage_mv
                ),
            });
        } else {
            jumped_from_index = None;
        }
        current_index = next_index;
        if current_index == candidate_count {
            completed = true;
        }
    }

    if final_gate_passes > 0
        && !aborted
        && !stop.load(Ordering::SeqCst)
        && last_qualified_index.is_some()
    {
        let mut gate_index = last_qualified_index;
        completed = false;
        while let Some(idx) = gate_index {
            let candidate = &descent.candidates[idx];
            match gate_anchored_candidate_fsgl3(
                store,
                obs_store,
                &mut qual_ctx,
                sane,
                limits,
                target_mhz,
                candidate,
                candidate_count,
                unpruned_steps,
                final_gate_dwell_ms,
                final_gate_passes,
                render_goldens,
                false,
                stop,
                &mut logs,
                &mut executed_steps,
                on_progress,
            ) {
                F2QualificationOutcome::Qualified => {
                    completed = true;
                    stop_reason = "BoundaryAccepted".into();
                    logs.push(format!(
                        "{target_mhz} MHz @ {} mV BoundaryAccepted",
                        candidate.anchor.voltage_mv
                    ));
                    break;
                }
                F2QualificationOutcome::Rejected(reason) => {
                    logs.push(format!(
                        "{target_mhz} MHz @ {} mV boundary rejected by v8 ({reason})",
                        candidate.anchor.voltage_mv
                    ));
                    gate_index = qualification_next_higher_candidate_index(idx);
                    if gate_index.is_none() {
                        completed = false;
                        stop_reason = format!("V8RejectedNoHigherBin: {reason}");
                    }
                }
                F2QualificationOutcome::Inconclusive => {
                    completed = false;
                    stop_reason = "V8Inconclusive".into();
                    break;
                }
                F2QualificationOutcome::Cancelled => {
                    completed = false;
                    stop_reason = "CancelledDuringV8".into();
                    break;
                }
                F2QualificationOutcome::Aborted {
                    stop_reason: reason,
                    retain_boot_flag: retain,
                } => {
                    aborted = true;
                    retain_boot_flag |= retain;
                    stop_reason = reason;
                    break;
                }
            }
        }
    } else if final_gate_passes == 0 && !aborted && completed {
        if let Some(idx) = last_qualified_index {
            let candidate = &descent.candidates[idx];
            logs.push(format!(
                "{target_mhz} MHz @ {} mV BoundaryAccepted (v8 default)",
                candidate.anchor.voltage_mv
            ));
        }
    }

    let scoped: Vec<_> = obs_store
        .query_by_target_for_gpu(target_mhz, gpu_key)
        .into_iter()
        .filter(|observation| {
            f2_observation_matches_current_candidate(
                &unpruned_descent.candidates,
                observation.anchor_mv,
                observation.base_mhz,
                observation.offset_mhz,
            )
        })
        .collect();
    let last_good_mv =
        last_discovery_good_for_target(&scoped, target_mhz).map(|o| o.anchor_mv);
    let current_run_last_good_mv = last_discovery_good_for_target(
        &scoped
            .iter()
            .filter(|observation| observation.run_id == run_id)
            .cloned()
            .collect::<Vec<_>>(),
        target_mhz,
    )
    .map(|o| o.anchor_mv);
    let first_bad_mv = first_bad_for_target(&scoped, target_mhz).map(|o| o.anchor_mv);
    let power_bound_clock_drops: Vec<u32> = scoped
        .iter()
        .filter(|o| {
            is_current_discovery_evidence(o)
                && matches!(
                    o.outcome,
                    nidavellir_core::f2_observation::F2ObsOutcome::PowerBoundClockDrop
                )
        })
        .map(|o| o.anchor_mv)
        .collect();
    let validated_voltages: Vec<u32> = scoped
        .iter()
        .filter(|o| {
            is_current_discovery_evidence(o)
                && matches!(
                    o.outcome,
                    nidavellir_core::f2_observation::F2ObsOutcome::Validated
                )
        })
        .map(|o| o.anchor_mv)
        .collect();
    let conservative_start_mv =
        f2_conservative_next_clock_start(&power_bound_clock_drops, &validated_voltages);
    let planned_voltages: Vec<u32> = unpruned_descent
        .candidates
        .iter()
        .map(|candidate| candidate.anchor.voltage_mv)
        .collect();
    let next_clock_start_mv =
        f2_optimized_next_clock_start(&planned_voltages, last_good_mv, conservative_start_mv);
    // A bin counts as a warm-start success (and makes the clock "sustainable") only when the
    // patterns the DESCENT actually runs all passed there. The descent runs the first
    // `qualification_passes` of REQUIRED_QUALIFICATION_PATTERNS — the single binding detector under
    // v13 (Texture); the FULL required set runs only at exact-Apply, NEVER here. Using the full
    // REQUIRED length here made every single-detector descent read as non-qualified → `sustainable`
    // false on every clock → `cmax` never set → the 90% frontier floor never fired and the descent
    // ran away through all physical bins (the ~5 h runaway). It still guards the original case (a
    // bin where an earlier descent pattern passed but a later one failed does not count).
    let has_current_full_qualification = {
        use nidavellir_core::f2_observation::REQUIRED_QUALIFICATION_PATTERNS;
        let descent_passes = qualification_passes.min(REQUIRED_QUALIFICATION_PATTERNS.len());
        let mut by_anchor = std::collections::BTreeMap::<
            u32,
            [bool; REQUIRED_QUALIFICATION_PATTERNS.len()],
        >::new();
        for observation in scoped
            .iter()
            .filter(|o| o.run_id == run_id && is_current_qualification_pass(o))
        {
            let Some(index) = observation
                .qualification_coverage
                .as_ref()
                .and_then(|coverage| coverage.pattern)
                .and_then(|pattern| {
                    REQUIRED_QUALIFICATION_PATTERNS.iter().position(|p| *p == pattern)
                })
            else {
                continue;
            };
            by_anchor.entry(observation.anchor_mv).or_default()[index] = true;
        }
        descent_passes > 0
            && by_anchor
                .values()
                .any(|seen| seen.iter().take(descent_passes).all(|present| *present))
    };
    let warm_start_rejected = start_mv.is_some()
        && (refresh_discovery_for_qualification || prior_good_mv.is_none())
        && (if qualification_passes > 0 {
            !has_current_full_qualification
        } else {
            current_run_last_good_mv.is_none()
        })
        && (executed_steps > 0 || stop_reason == "WarmStartNoPhysicalCandidates")
        && !aborted;
    F2ClockDiscoverySummary {
        sustainable: last_good_mv.is_some()
            && (qualification_passes == 0 || has_current_full_qualification),
        last_good_mv,
        first_bad_mv,
        next_clock_start_mv,
        conservative_start_mv,
        warm_start_rejected,
        executed_steps,
        completed,
        aborted,
        retain_boot_flag,
        stop_reason,
        logs,
    }
}

/// MANUAL-PRIOR anchored probe (`--manual-prior`, explicit development / known-GPU shortcut). DRY-RUN
/// by default: requires an explicit `--start-mv`, plans ONE anchored point at that operator-provided
/// voltage using a SEPARATE larger bounded offset cap ([`F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ`]),
/// runs the Safe Loop preflight read-only, prints the plan + self-check, and reports required offset vs
/// cap. WITH `--confirm` it runs the fail-closed manual-prior preflight ([`confirmed_manual_prior_refusal`],
/// single-step only) then ONE supervised anchored step over the SAME validated motor
/// ([`run_confirmed_f2_step`] / [`RealF2Ops`]) with the manual-prior limits. NEVER the default; the
/// default/autonomous discovery caps are untouched; never persists/applies/promotes a profile.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_manual_prior_undervolt_probe(
    store: &SafeLoopStore,
    confirm: bool,
    args: &UndervoltArgs,
    sane: &[(usize, u32, u32)],
    floor_mv: u32,
    boost_top: u32,
    focus_target: u32,
    record: &SafeLoopRecord,
    boot_flag_armed: bool,
) {
    // Manual-prior REQUIRES an explicit --start-mv — it never guesses a voltage for an unknown GPU.
    let Some(start_mv) = args.start_mv else {
        println!(
            "undervolt-probe: --manual-prior REQUIRES an explicit --start-mv (e.g. --start-mv 875). \
             No hardware touched."
        );
        warn!("undervolt-probe: --manual-prior without --start-mv — refused, no hardware touched");
        return;
    };
    // SEPARATE manual-prior offset envelope (larger bounded cap); floor / ceiling / sanity / real-bin
    // checks stay EXACTLY as default discovery. The default/autonomous path NEVER sees this cap.
    let manual_limits =
        PositiveOffsetLimits::manual_prior(floor_mv, boost_top, F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ,
    );
    let plan = plan_manual_prior_undervolt(sane, focus_target, start_mv, &manual_limits);

    // Read-only Safe Loop preflight over the planned bins (anchor + caps + elastic), if a plan exists.
    let points: Vec<TuningPoint> = plan.probe.plan.as_ref().map_or_else(Vec::new, |p| {
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

    for line in manual_prior_plan_lines(&plan, &manual_limits, &preflight) {
        println!("{line}");
    }

    // Plan/verifier self-consistency (only when within bounds): the planned curve must verify as
    // AnchoredRaiseVerified using the SAME verifier the confirmed path uses — no hardware.
    if let Some(p) = &plan.probe.plan {
        let observed: Vec<(usize, Option<i32>)> =
            p.entries.iter().map(|e| (e.index, Some(e.offset_mhz))).collect();
        let v = crate::gpu_verify::verify_anchored_positive_offset(p, &observed, F2_VERIFY_TOL_MHZ);
        println!("plan self-check    : anchored plan verifies as {v:?} (tol {F2_VERIFY_TOL_MHZ} MHz)");
    }

    if confirm {
        // Confirmed MANUAL-PRIOR single step. The candidate is the anchor bin; the gate requires an
        // explicit --start-mv + --steps 1, then re-runs the full fail-closed preflight (with the
        // manual-prior limits) before touching anything.
        let candidate = plan.probe.plan.as_ref().map(|p| p.anchor);
        match confirmed_manual_prior_refusal(
            record,
            boot_flag_armed,
            args.start_mv,
            args.steps,
            candidate.as_ref(),
            &manual_limits,
            focus_target,
        ) {
            Some(reason) => {
                println!(
                    "undervolt-probe: --confirm REFUSED — {reason}. No Safe Loop arm, no apply, no \
                     dwell, no VF write performed."
                );
                warn!("undervolt-probe: --confirm refused (manual-prior): {reason} — no hardware touched");
            }
            None => {
                let p = plan.probe.plan.expect("refusal None guarantees a plan");
                let anchor = p.anchor;
                warn!(
                    "undervolt-probe: --confirm — executing ONE supervised MANUAL-PRIOR ANCHORED F2 step \
                     ({} MHz @ {} mV, +{} MHz anchor, {} plateau cap(s)) — can TDR/reboot.",
                    focus_target, anchor.voltage_mv, anchor.offset_mhz, p.capped_above_bins
                );
                let mut ops = RealF2Ops {
                    store,
                    curve: sane.to_vec(),
                    candidate: anchor,
                    anchored: Some(p.clone()),
                    mode: UndervoltMode::Anchored,
                    limits: manual_limits,
                    target_mhz: focus_target,
                    prev_offset_mhz: 0, // manual-prior is single-step: per-step measured from stock (unchanged)
                    dwell_ms: F2_STANDARD_DWELL_MS,
                    stress_purpose: F2StressPurpose::PowerDiscovery,
                    cancel: None,
                };
                let report = run_confirmed_f2_step(&mut ops);
                for line in confirmed_report_lines(focus_target, &anchor, &manual_limits, &report) {
                    println!("{line}");
                }
                info!(
                    "undervolt-probe: confirmed MANUAL-PRIOR ANCHORED F2 single step outcome={:?}",
                    report.outcome
                );
            }
        }
        return;
    }
    println!(
        "(dry-run — pass `--target-mhz {} --start-mv {} --steps 1 --manual-prior --confirm` for ONE \
         supervised manual-prior step; nothing was written)",
        focus_target, start_mv
    );
    info!(
        "undervolt-probe: DRY-RUN (manual-prior) — target={} MHz start_mv={} selected_mv={:?} \
         required_offset={:?} manual_cap=+{} within_bounds={} preflight_safe={} — no Safe Loop arm, \
         no apply, no dwell, no VF write.",
        focus_target, start_mv, plan.selected_mv, plan.required_offset_mhz,
        F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ, plan.within_bounds, preflight.safe
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
        vec![(0, 850, 1700), (1, 900, 1725), (2, 950, 1740), (3, 1000, 1748), (4, 1062, 1755),
        ]
    }

    fn pt(freq: u32, mv: u32) -> TuningPoint {
        TuningPoint::from_axes([("gpu_freq_mhz", freq as i64), ("gpu_vf_bin_mv", mv as i64)])
    }

    // ── chained same-target descent (observation-aware baseline + within-run advancement) ────────
    #[test]
    fn chained_prev_offset_chains_off_prior_candidate_else_baseline() {
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        let d = plan_anchored_undervolt_descent(&t_base(), 1755, None, &limits, usize::MAX);
        // Descent candidates for 1755 MHz on t_base: 1000 mV (+7), 950 mV (+15), 900 mV (+30).
        let offs: Vec<i32> = d.candidates.iter().map(|c| c.anchor.offset_mhz).collect();
        assert_eq!(offs, vec![7, 15, 30]);
        // Candidate 0 uses the cross-run observation baseline (0 when none); deeper candidates chain off
        // the prior candidate, which the motor only reaches AFTER it validated.
        assert_eq!(chained_prev_offset(&d.candidates, 0, 0), 0);
        assert_eq!(chained_prev_offset(&d.candidates, 0, 7), 7);
        assert_eq!(chained_prev_offset(&d.candidates, 1, 0), 7);
        assert_eq!(chained_prev_offset(&d.candidates, 2, 0), 15);
        // Out-of-range index falls back to the baseline (defensive — never panics).
        assert_eq!(chained_prev_offset(&d.candidates, 9, 3), 3);
    }

    #[test]
    fn chained_descent_admits_plus30_after_validated_plus15() {
        use nidavellir_gpu_nvapi::plan_bounded_anchored_positive_offset;
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        // The 900 mV bin (index 1) needs +30 to hold 1755. A single +30 step from STOCK (+0) is rejected
        // by the per-step +15 cap — the exact PASS-PARTIAL failure on real hardware...
        let from_stock = plan_bounded_anchored_positive_offset(&t_base(), 1, 1755, 0, &limits);
        assert!(from_stock.is_err());
        assert!(from_stock.unwrap_err().contains("per-step"));
        // ...but allowed once the prior candidate validated at +15: the chained per-step delta is +15.
        assert!(plan_bounded_anchored_positive_offset(&t_base(), 1, 1755, 15, &limits).is_ok());
    }

    #[test]
    fn chained_descent_still_enforces_absolute_cap_with_baseline() {
        use nidavellir_gpu_nvapi::plan_bounded_anchored_positive_offset;
        let limits = PositiveOffsetLimits::conservative(850, 1755);
        // The 850 mV bin (index 0) needs +55. Even with a baseline that satisfies the per-step delta, the
        // ABSOLUTE +30 cap still rejects it (fail closed) — chaining never widens the absolute bound.
        let err = plan_bounded_anchored_positive_offset(&t_base(), 0, 1755, 45, &limits).unwrap_err();
        assert!(err.contains("absolute cap"));
    }

    // ── TARGET-SWEEP learned offset horizon (official --auto-sweep descent envelope) ─────────────
    // A clean +15-per-bin ladder so the descent crosses the +30 default absolute cap: the horizon keeps
    // descending (through validated chained increments) exactly where the conservative envelope stops.
    fn sweep_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1740), (1, 900, 1755), (2, 950, 1770), (3, 1000, 1785), (4, 1062, 1810),
        ]
    }

    #[test]
    fn power_calibration_plans_exact_apply_bin_from_qualified_boundary_offset() {
        let limits = PositiveOffsetLimits::target_sweep_learning_horizon(850, 1800);
        let candidate =
            plan_f2_power_calibration_candidate(&sweep_base(), 1800, 1000, 45, &limits).unwrap();
        assert_eq!(candidate.anchor.voltage_mv, 1000);
        assert_eq!(candidate.anchor.offset_mhz, 15);
        assert_eq!(candidate.anchor.prev_offset_mhz, 45);
        assert!(plan_f2_power_calibration_candidate(
            &sweep_base(),
            1800,
            987,
            45,
            &limits
        )
        .is_err());
    }

    #[test]
    fn target_sweep_horizon_descent_continues_past_plus30_where_conservative_stops() {
        // Conservative envelope: the descent reaches +15, +30 and then STOPS — the 900 mV bin needs +45,
        // which the +30 ABSOLUTE cap rejects (even though the per-step delta from +30 is a valid +15).
        let cons = PositiveOffsetLimits::conservative(850, 1800);
        let dc =
            plan_anchored_undervolt_descent(&sweep_base(), 1800, None, &cons, F2_SWEEP_DRYRUN_BUDGET,
        );
        let cons_offs: Vec<i32> = dc.candidates.iter().map(|c| c.anchor.offset_mhz).collect();
        assert_eq!(cons_offs, vec![15, 30]);
        assert!(dc.stop_reason.unwrap().contains("absolute cap"));

        // Target-sweep learning horizon: SAME per-step +15 chaining, but the larger absolute cap lets the
        // descent keep planning deeper bins (+45, +60) — each reached only via a valid +15 chained step.
        let hz = PositiveOffsetLimits::target_sweep_learning_horizon(850, 1800);
        let dh =
            plan_anchored_undervolt_descent(&sweep_base(), 1800, None, &hz, F2_SWEEP_DRYRUN_BUDGET);
        let hz_offs: Vec<i32> = dh.candidates.iter().map(|c| c.anchor.offset_mhz).collect();
        assert_eq!(hz_offs, vec![15, 30, 45, 60]);
        // No-last-good start is still conservative: candidate 0 is a +15 step from stock (+0).
        assert_eq!(dh.candidates[0].anchor.step_delta_mhz, 15);
        // Every chained step stays within the conserved per-step cap — the horizon never relaxes it.
        assert!(dh.candidates.iter().all(|c| c.anchor.step_delta_mhz <= hz.step_max_offset_mhz));
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
    fn parse_undervolt_args_reads_validation_passes_and_defaults_to_one() {
        // Default (flag absent) = 1 (today's behavior — one validation per point).
        assert_eq!(parse_undervolt_args(&os(&["undervolt-probe"])).unwrap().validation_passes, 1);
        // Explicit value parses.
        assert_eq!(
            parse_undervolt_args(&os(&["undervolt-probe", "--validation-passes", "5"]))
                .unwrap()
                .validation_passes,
            5
        );
        // Non-numeric / missing values fail closed (like the other numeric flags).
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--validation-passes", "x"])).is_err());
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--validation-passes"])).is_err());
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

    #[test]
    fn discovery_keeps_high_clock_while_clock_drop_is_power_bound() {
        assert!(f2_near_power_limit(Some(199.0), Some(200.0), Some(0.0)));
        assert_eq!(
            f2_power_bound_clock_drop(&F2Outcome::ClockDrop, true),
            F2Outcome::PowerBoundClockDrop
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::ClockDrop, false, true),
            F2DiscoveryDecision::ContinueVoltage
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::Validated, false, false),
            F2DiscoveryDecision::MarkSustainableAndContinue
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::ClockDrop, true, true),
            F2DiscoveryDecision::ContinueVoltage
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::PowerBoundClockDrop, true, true),
            F2DiscoveryDecision::ContinueVoltage
        );
    }

    #[test]
    fn power_bound_classification_uses_p99_not_mean() {
        let power_avg_w = 174.0;
        let power_p99_w = 200.0;
        assert!(power_avg_w / 200.0 < 0.99);
        assert!(f2_near_power_limit(
            Some(power_p99_w),
            Some(200.0),
            Some(0.0)
        ));

        let misleading_avg_w = 200.0;
        let off_cap_p99_w = 180.0;
        assert!(misleading_avg_w / 200.0 >= 0.99);
        assert!(!f2_near_power_limit(
            Some(off_cap_p99_w),
            Some(200.0),
            Some(1.0)
        ));
    }

    fn power_report(power_p99_w: f32, p5_clock_mhz: u32) -> F2StepReport {
        F2StepReport {
            outcome: F2Outcome::Validated,
            armed: true,
            applied: true,
            verify: Some(PositiveOffsetVerification::RaiseVerified),
            dwell: Some(F2DwellOutcome::Stable),
            avg_clock_mhz: Some(p5_clock_mhz),
            p5_clock_mhz: Some(p5_clock_mhz),
            p95_clock_mhz: Some(p5_clock_mhz),
            power_w: Some(power_p99_w.round() as u32),
            max_power_w: Some(power_p99_w.ceil() as u32),
            power_p99_w: Some(power_p99_w),
            power_p99_confirmed: false,
            power_p99_attempts: 0,
            power_capped_frac: Some(0.0),
            max_temp_c: Some(70.0),
            thermal_throttled: false,
            measured_voltage_min_mv: Some(886),
            measured_voltage_avg_mv: Some(887),
            measured_voltage_max_mv: Some(888),
            measured_voltage_sample_count: 20,
            render_frames: Some(600),
            render_fps: Some(60.0),
            dwell_duration_ms: Some(10_000),
            sample_count: Some(100),
            qualification_coverage: None,
            reset_ok: Some(true),
            boot_flag_cleared: true,
            blacklisted: false,
            validated: true,
        }
    }

    #[test]
    fn anomalous_adjacent_p99_step_requires_same_bin_recheck() {
        let report = power_report(160.0, 1890);
        assert!(f2_power_p99_requires_recheck(Some((183.0, 1890)), &report));
        assert!(!f2_power_p99_requires_recheck(
            Some((168.0, 1890)),
            &report
        ));
        assert!(!f2_power_p99_requires_recheck(Some((183.0, 1920)), &report));
    }

    #[test]
    fn repeated_p99_uses_consensus_and_conservative_measured_max() {
        let mut reports = vec![
            power_report(160.0, 1890),
            power_report(180.0, 1890),
            power_report(181.0, 1890),
        ];
        assert_eq!(f2_confirm_power_attempts(&mut reports, true), Some(181.0));
        assert!(reports.iter().all(|report| report.power_p99_confirmed));
        assert!(reports
            .iter()
            .all(|report| report.power_p99_attempts == 3));
    }

    #[test]
    fn repeated_p99_without_consistent_pair_is_inconclusive() {
        let mut reports = vec![
            power_report(160.0, 1890),
            power_report(180.0, 1890),
            power_report(200.0, 1890),
        ];
        assert_eq!(f2_confirm_power_attempts(&mut reports, true), None);
        assert!(reports
            .iter()
            .all(|report| report.outcome == F2Outcome::Inconclusive));
        assert!(reports.iter().all(|report| !report.power_p99_confirmed));
    }

    #[test]
    fn repeated_p99_never_confirms_positive_evidence_across_hard_failure() {
        let mut failed = power_report(181.0, 1890);
        failed.outcome = F2Outcome::SilentError;
        failed.dwell = Some(F2DwellOutcome::SilentError);
        let mut reports = vec![
            power_report(180.0, 1890),
            power_report(181.0, 1890),
            failed,
        ];
        assert_eq!(f2_confirm_power_attempts(&mut reports, true), None);
        assert!(reports.iter().all(|report| !report.power_p99_confirmed));
        assert_eq!(
            f2_aggregate_power_attempts(&reports, None).outcome,
            F2Outcome::SilentError
        );
    }

    #[test]
    fn validated_at_cap_continues_discovery_before_fsgl3() {
        assert!(!f2_should_qualify_discovery_candidate(
            &F2Outcome::Validated,
            true,
            2
        ));
        assert!(f2_should_qualify_discovery_candidate(
            &F2Outcome::Validated,
            false,
            2
        ));
    }

    #[test]
    fn next_clock_keeps_conservative_fallback() {
        assert_eq!(
            f2_conservative_next_clock_start(&[1112, 975, 950], &[943, 937, 931]),
            Some(950)
        );
        assert_eq!(
            f2_conservative_next_clock_start(&[], &[975, 968, 962]),
            Some(975)
        );
        assert_eq!(f2_conservative_next_clock_start(&[], &[]), None);
    }

    #[test]
    fn next_clock_starts_one_physical_bin_above_previous_minimum() {
        let bins = [950, 943, 937, 931];
        assert_eq!(
            f2_optimized_next_clock_start(&bins, Some(937), Some(950)),
            Some(943)
        );
        assert_eq!(
            f2_optimized_next_clock_start(&bins, Some(950), Some(975)),
            Some(950)
        );
        assert_eq!(
            f2_optimized_next_clock_start(&bins, None, Some(975)),
            Some(975)
        );
    }

    #[test]
    fn discovery_abandons_unsustained_clock_once_off_cap() {
        assert!(!f2_near_power_limit(Some(185.0), Some(200.0), Some(1.0)));
        assert_eq!(
            f2_power_bound_clock_drop(&F2Outcome::ClockDrop, false),
            F2Outcome::ClockDrop
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::ClockDrop, false, false),
            F2DiscoveryDecision::NextClockUnsustainable
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::SilentError, false, false),
            F2DiscoveryDecision::NextClockAfterFailure
        );
    }

    #[test]
    fn fsgl3_qualification_does_not_treat_light_phase_p5_as_clock_drop() {
        let stable_low_p5 = crate::gpu_power_sweep::SingleDwell {
            cancelled: false,
            crashed: false,
            silent_error: false,
            stable: true,
            avg_clock_mhz: 1500,
            p5_clock_mhz: 1200,
            p95_clock_mhz: 1800,
            power_w: 120.0,
            max_power_w: 130.0,
            power_p99_w: Some(128.0),
            power_capped_frac: 0.0,
            max_temp_c: Some(68.0),
            thermal_throttled: false,
            volt_min_mv: Some(949),
            volt_avg_mv: Some(950),
            volt_max_mv: Some(951),
            volt_sample_count: 20,
            render_frames: Some(3_600),
            render_fps: Some(60.0),
            duration_ms: 60_000,
            sample_count: 1_000,
            qualification_coverage: None,
            prehang_stall_detected: false,
        };
        assert_eq!(
            classify_f2_stress_dwell(
                &stable_low_p5,
                1800,
                F2StressPurpose::PowerDiscovery
            ),
            F2DwellOutcome::ClockDrop
        );
        assert_eq!(
            classify_f2_stress_dwell(
                &stable_low_p5,
                1800,
                F2StressPurpose::V8Qualification(
                    F2QualificationPattern::A,
                    RenderGoldens {
                        power: 1,
                        boost: 2,
                        texrop: 3,
                        cadence: 4,
                        geometry: 5,
                        stream: 6,
                        stream_frame_reference_ms: 20,
                    },
                )
            ),
            F2DwellOutcome::Stable
        );

        let mut thermal = stable_low_p5;
        thermal.p5_clock_mhz = 1800;
        thermal.thermal_throttled = true;
        assert_eq!(
            classify_f2_stress_dwell(&thermal, 1800, F2StressPurpose::PowerDiscovery),
            F2DwellOutcome::Inconclusive
        );
        thermal.cancelled = true;
        thermal.stable = true;
        assert_eq!(
            classify_f2_stress_dwell(&thermal, 1800, F2StressPurpose::PowerDiscovery),
            F2DwellOutcome::Inconclusive,
            "operator cancellation must never become unstable or validated evidence"
        );
        thermal.thermal_throttled = false;
        thermal.power_p99_w = None;
        assert_eq!(
            classify_f2_stress_dwell(&thermal, 1800, F2StressPurpose::PowerDiscovery),
            F2DwellOutcome::Inconclusive
        );
    }

    #[test]
    fn apply_qualification_thermal_slowdown_that_held_clock_is_not_inconclusive() {
        // A thermal-slowdown flag during exact-Apply qualification is only disqualifying when the
        // slowdown backed the card OFF the qualified point. If the card HELD >= target the hard VF
        // point was exercised, so the dwell must validate — otherwise a card that momentarily hits a
        // memory-junction hotspot at a cool core temp can never certify an Apply point it is in fact
        // stable at (the exact failure that left a whole run with zero applicable profiles).
        let base = crate::gpu_power_sweep::SingleDwell {
            cancelled: false,
            crashed: false,
            silent_error: false,
            stable: true,
            // v13: dwells run under the absolute clock ceiling, so p95 never exceeds target.
            avg_clock_mhz: 1935,
            p5_clock_mhz: 1935,
            p95_clock_mhz: 1935,
            power_w: 199.0,
            max_power_w: 200.0,
            power_p99_w: Some(199.7),
            power_capped_frac: 1.0,
            max_temp_c: Some(69.0),
            thermal_throttled: true,
            volt_min_mv: Some(955),
            volt_avg_mv: Some(956),
            volt_max_mv: Some(957),
            volt_sample_count: 300,
            render_frames: Some(18_000),
            render_fps: Some(60.0),
            duration_ms: 300_000,
            sample_count: 5_000,
            qualification_coverage: None,
            prehang_stall_detected: false,
        };
        // Held clock (p5 == target) despite the throttle flag → hard point exercised → validate.
        assert_eq!(
            classify_f2_stress_dwell(
                &base,
                1935,
                F2StressPurpose::ApplyQualification(
                    F2QualificationPattern::A,
                    RenderGoldens {
                        power: 1,
                        boost: 2,
                        texrop: 3,
                        cadence: 4,
                        geometry: 5,
                        stream: 6,
                        stream_frame_reference_ms: 20,
                    },
                ),
            ),
            F2DwellOutcome::Stable
        );
        // Power discovery is unchanged: thermal slowdown corrupts the V↔W map even at held clock.
        assert_eq!(
            classify_f2_stress_dwell(&base, 1935, F2StressPurpose::PowerDiscovery),
            F2DwellOutcome::Inconclusive
        );
        // A thermal slowdown that actually sagged the sustained clock below tolerance stays
        // inconclusive for Apply qualification — the card was backed off the qualified point.
        let mut dropped = base;
        dropped.p5_clock_mhz = 1935 - F2_CLOCK_DROP_TOL_MHZ - 1;
        assert_eq!(
            classify_f2_stress_dwell(
                &dropped,
                1935,
                F2StressPurpose::ApplyQualification(
                    F2QualificationPattern::A,
                    RenderGoldens {
                        power: 1,
                        boost: 2,
                        texrop: 3,
                        cadence: 4,
                        geometry: 5,
                        stream: 6,
                        stream_frame_reference_ms: 20,
                    },
                ),
            ),
            F2DwellOutcome::Inconclusive
        );
        // v15 regression: TransitionShock dwells are ~60% TRUE idle by design, so p5 is an idle
        // clock and the p5-sag thermal rule must NOT apply — a routine throttle flag would
        // otherwise misclassify every shock dwell as Inconclusive and refuse the candidate at the
        // END of a full run. The shock carries its own detectors (slam-stall → Unstable, golden →
        // SilentError); a clean shock dwell with a throttle flag and idle-low p5 must validate.
        assert_eq!(
            classify_f2_stress_dwell(
                &dropped,
                1935,
                F2StressPurpose::ApplyQualification(
                    F2QualificationPattern::TransitionShock,
                    RenderGoldens {
                        power: 1,
                        boost: 2,
                        texrop: 3,
                        cadence: 4,
                        geometry: 5,
                        stream: 6,
                        stream_frame_reference_ms: 20,
                    },
                ),
            ),
            F2DwellOutcome::Stable
        );
    }

    fn qualification_coverage_with_phase_p5(
        pattern: F2QualificationPattern,
        phase_values: &[(&str, u32)],
    ) -> F2QualificationCoverage {
        F2QualificationCoverage {
            strength: F2QualificationStrength::Fsgl3,
            pattern: Some(pattern),
            pass_index: 1,
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
            phase_metrics: phase_values
                .iter()
                .map(|(phase_name, p5)| {
                    nidavellir_core::f2_observation::F2QualificationPhaseMetric {
                        phase_name: (*phase_name).to_string(),
                        phase_pattern: match pattern {
                            F2QualificationPattern::A => "fsgl3-a",
                            F2QualificationPattern::B => "fsgl3-b",
                            F2QualificationPattern::HighFps => "v8-high-fps",
                            F2QualificationPattern::Texture => "v8-texture",
                            F2QualificationPattern::Transitions => "v8-transitions",
                            F2QualificationPattern::Memory => "v8-memory",
                            F2QualificationPattern::Endurance => "endurance",
                            F2QualificationPattern::TransitionShock => "transition-shock",
                        }
                        .to_string(),
                        duration_ms: 1_000,
                        frame_count: 10,
                        checksum_count: 1,
                        compute_check_count: 0,
                        clock_avg: Some(*p5 as f32),
                        clock_p5: Some(*p5),
                        clock_p50: Some(*p5),
                        clock_p95: Some(*p5),
                        target_residency_pct: Some(100.0),
                        power_avg: Some(150.0),
                        power_p95: Some(155.0),
                        power_capped_fraction: Some(0.0),
                        temperature_avg: Some(60.0),
                        temperature_max: Some(62.0),
                        coverage_status: "pass".into(),
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn qualification_margin_uses_heavy_phase_p5_and_ignores_normal_noise() {
        let coverage = qualification_coverage_with_phase_p5(
            F2QualificationPattern::A,
            &[
                ("boost-edge", 1200),
                ("heavy-spike", 1940),
                ("texture-rop", 1935),
                ("mixed-game", 1945),
                ("power-closing", 1950),
            ],
        );
        assert_eq!(qualification_margin_p5(Some(&coverage)), Some(1942));
        let history = [1950, 1935];
        assert!(!qualification_margin_is_clock_drop(1935, &history, 1900));
        assert!(qualification_margin_is_clock_drop(1900, &history, 1900));
    }

    #[test]
    fn qualification_margin_falls_back_to_target_and_retry_budget_is_finite() {
        assert!(qualification_margin_is_clock_drop(1760, &[], 1800));
        assert!(!qualification_margin_is_clock_drop(1770, &[], 1800));
        assert_eq!(qualification_attempt_dwell_ms(60_000, 0), 60_000);
        assert_eq!(qualification_attempt_dwell_ms(60_000, 1), 90_000);
        assert_eq!(qualification_attempt_dwell_ms(60_000, 2), 90_000);
        assert!(qualification_should_retry_inconclusive(0));
        assert!(qualification_should_retry_inconclusive(1));
        assert!(!qualification_should_retry_inconclusive(2));
        assert!(apply_qualification_pattern_complete(0, 1));
        assert!(!apply_qualification_pattern_complete(1, 1));
        assert!(apply_qualification_pattern_complete(1, 2));
    }

    #[test]
    fn discovery_inconclusive_skips_clock_instead_of_aborting_forge() {
        assert_eq!(
            f2_discovery_decision(&F2Outcome::Inconclusive, false, false),
            F2DiscoveryDecision::NextClockAfterFailure
        );
    }

    #[test]
    fn qualification_gate_uses_all_v7_patterns_and_failure_moves_one_bin_up() {
        assert_eq!(
            qualification_gate_patterns(3),
            vec![
                F2QualificationPattern::Texture,
                F2QualificationPattern::Transitions,
                F2QualificationPattern::Memory,
            ]
        );
        assert_eq!(
            qualification_gate_patterns(1),
            vec![F2QualificationPattern::Texture]
        );
        assert_eq!(qualification_next_higher_candidate_index(3), Some(2));
        assert_eq!(qualification_next_higher_candidate_index(0), None);
    }

    #[test]
    fn reset_clean_qualification_rejection_completes_only_current_clock() {
        assert!(F2QualificationOutcome::Qualified.completes_clock());
        assert!(F2QualificationOutcome::Rejected("ClockDrop".into()).completes_clock());
        assert!(F2QualificationOutcome::Inconclusive.completes_clock());
        assert!(!F2QualificationOutcome::Cancelled.completes_clock());
        assert!(
            !F2QualificationOutcome::Aborted {
                stop_reason: "DeviceLost".into(),
                retain_boot_flag: true,
            }
            .completes_clock()
        );
    }

    #[test]
    fn qualification_purpose_selects_v7_pattern_and_carries_stock_goldens() {
        let goldens = RenderGoldens {
            power: 11,
            boost: 22,
            texrop: 33,
            cadence: 44,
            geometry: 55,
            stream: 66,
            stream_frame_reference_ms: 20,
        };
        let purpose = F2StressPurpose::V8Qualification(
            F2QualificationPattern::Transitions,
            goldens,
        );
        assert_eq!(
            purpose.qualifier_pattern(),
            Some(VfQualifierPattern::V8Transitions)
        );
        assert_eq!(purpose.render_goldens(), Some(goldens));
    }

    #[test]
    fn discovery_silent_error_is_terminal_and_device_loss_aborts_forge() {
        assert_eq!(
            f2_discovery_decision(&F2Outcome::SilentError, true, false),
            F2DiscoveryDecision::BoundaryFound
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::DeviceLost, true, false),
            F2DiscoveryDecision::AbortForge
        );
        assert_eq!(
            f2_discovery_decision(&F2Outcome::ResetFailed, false, false),
            F2DiscoveryDecision::AbortForge
        );
        assert!(f2_outcome_retains_boot_flag(&F2Outcome::DeviceLost));
        assert!(f2_outcome_retains_boot_flag(&F2Outcome::ResetFailed));
        assert!(!f2_outcome_retains_boot_flag(&F2Outcome::VerifyFailed));
    }

    #[test]
    fn power_cap_flag_is_only_fallback_when_numeric_limit_is_missing() {
        assert!(f2_near_power_limit(Some(150.0), None, Some(0.75)));
        assert!(!f2_near_power_limit(Some(150.0), None, Some(0.25)));
        assert!(
            !f2_near_power_limit(None, None, Some(1.0)),
            "missing p99 must never be replaced by the cap flag"
        );
        assert!(
            !f2_near_power_limit(Some(180.0), Some(200.0), Some(1.0)),
            "a valid 90% p99 reading must not be overridden by the cap-flag fallback"
        );
    }

    #[cfg(windows)]
    #[test]
    fn adaptive_power_bound_stride_respects_p5_voltage_and_offset_guards() {
        let curve: Vec<(usize, u32, u32)> = [900, 906, 912, 918, 925, 931]
            .into_iter()
            .enumerate()
            .map(|(index, voltage_mv)| (index, voltage_mv, 1770 + index as u32 * 3))
            .collect();
        let limits = PositiveOffsetLimits::hardware_frontier(900, 1800, 1770);
        let candidates =
            plan_anchored_undervolt_descent(&curve, 1800, None, &limits, usize::MAX).candidates;
        assert_eq!(
            f2_adaptive_power_bound_next_index(
                &candidates,
                0,
                1800,
                Some(1700),
                candidates[0].anchor.offset_mhz,
                limits.step_max_offset_mhz,
            ),
            4,
            "a >=90 MHz deficit may skip four physical bins"
        );
        assert_eq!(
            f2_adaptive_power_bound_next_index(
                &candidates,
                0,
                1800,
                Some(1740),
                candidates[0].anchor.offset_mhz,
                limits.step_max_offset_mhz,
            ),
            2,
            "a 45-89 MHz deficit may skip two physical bins"
        );
        assert_eq!(
            f2_adaptive_power_bound_next_index(
                &candidates,
                0,
                1800,
                Some(1770),
                candidates[0].anchor.offset_mhz,
                limits.step_max_offset_mhz,
            ),
            1,
            "near the target discovery remains adjacent"
        );

        let wide_curve: Vec<(usize, u32, u32)> = [880, 890, 900, 910, 920, 930]
            .into_iter()
            .enumerate()
            .map(|(index, voltage_mv)| (index, voltage_mv, 1770 + index as u32 * 3))
            .collect();
        let wide_candidates =
            plan_anchored_undervolt_descent(&wide_curve, 1800, None, &limits, usize::MAX).candidates;
        assert_eq!(
            f2_adaptive_power_bound_next_index(
                &wide_candidates,
                0,
                1800,
                Some(1700),
                wide_candidates[0].anchor.offset_mhz,
                limits.step_max_offset_mhz,
            ),
            2,
            "the 25 mV guard reduces a requested four-bin jump"
        );
    }

    #[cfg(windows)]
    #[test]
    fn adaptive_recovery_only_bisects_toward_the_safer_side() {
        assert_eq!(f2_recovery_midpoint(2, 6), Some(4));
        assert_eq!(f2_recovery_midpoint(4, 6), Some(5));
        assert_eq!(f2_recovery_midpoint(5, 6), None);
    }

    #[test]
    fn discovery_resume_skips_confirmed_bins_and_known_boundary_skips_descent() {
        let limits = PositiveOffsetLimits::hardware_frontier(850, 1900, 1700);
        let mut candidates =
            plan_anchored_undervolt_descent(&a_base(), 1755, None, &limits, usize::MAX).candidates;
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.anchor.voltage_mv)
                .collect::<Vec<_>>(),
            vec![900, 850]
        );
        let current = &candidates[0].anchor;
        assert!(f2_observation_matches_current_candidate(
            &candidates,
            current.voltage_mv,
            current.base_mhz,
            current.offset_mhz
        ));
        assert!(!f2_observation_matches_current_candidate(
            &candidates,
            current.voltage_mv,
            current.base_mhz - 15,
            current.offset_mhz + 15
        ));
        assert_eq!(
            resume_f2_candidates(&mut candidates, Some(900), None, None),
            Some(900)
        );
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.anchor.voltage_mv)
                .collect::<Vec<_>>(),
            vec![850]
        );
        resume_f2_candidates(&mut candidates, Some(900), Some(850), None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn failed_warm_start_retries_only_higher_unknown_bins() {
        let limits = PositiveOffsetLimits::hardware_frontier(850, 1900, 1700);
        let mut candidates =
            plan_anchored_undervolt_descent(&a_base(), 1755, None, &limits, usize::MAX).candidates;
        assert_eq!(
            resume_f2_candidates(&mut candidates, None, Some(850), None),
            Some(850)
        );
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.anchor.voltage_mv)
                .collect::<Vec<_>>(),
            vec![900]
        );
    }

    // ── anchored probe (plan_anchored_undervolt / select_anchor_bin / anchored_plan_lines) ────
    // Anchor-focused base: lower bins below target, higher bins above target so the plateau caps
    // engage. (idx, mV, base): 850/1700, 900/1740, 950/1770, 1000/1800, 1062/1845.
    fn a_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1700), (1, 900, 1740), (2, 950, 1770), (3, 1000, 1800), (4, 1062, 1845),
        ]
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
    fn apply_anchor_requires_exact_validated_bin() {
        assert_eq!(select_exact_apply_anchor_bin(&a_base(), 1755, 900), Some(1));
        assert_eq!(
            select_exact_apply_anchor_bin(&a_base(), 1755, 925),
            None,
            "apply must not silently fall to a lower-voltage bin"
        );
        assert_eq!(
            select_exact_apply_anchor_bin(&a_base(), 1700, 900),
            None,
            "an anchor already above the target does not need a positive raise"
        );
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

    // A durable Safe Loop blacklist (e.g. left by a prior run's crash/TDR, surviving a program
    // reopen/resume) that catches the NEXT lower-voltage descent candidate must be recognized by
    // `candidate_blacklisted`. The live descent (`run_confirmed_f2_clock_discovery`) keys its
    // "stop at this boundary and let the frontier continue" branch on exactly this predicate — a
    // blacklisted next candidate ⇒ BlacklistedBoundary (completed), NOT SafetyPrecheckFailed
    // (aborted). Regression guard for the whole-frontier-abort bug (2026-07-08 20:51 run).
    #[test]
    fn blacklisted_next_descent_candidate_is_a_boundary_trigger() {
        let plan = |mv: u32| PositiveOffsetPlan {
            index: 0,
            voltage_mv: mv,
            base_mhz: 1740,
            offset_mhz: 30,
            prev_offset_mhz: 0,
            step_delta_mhz: 30,
            effective_mhz: 1770,
        };
        // A prior run's SilentError/TDR blacklisted the 812 mV region at 1770 MHz.
        let mut rec = SafeLoopRecord::default();
        rec.blacklist.push(BlacklistRegion::around(pt(1770, 812), 0));
        // The next descent candidate (812 mV) is caught → descent stops ABOVE it (boundary).
        assert!(candidate_blacklisted(&rec, 1770, &plan(812)));
        // The last validated bin (818 mV) stays clean → it remains this clock's boundary.
        assert!(!candidate_blacklisted(&rec, 1770, &plan(818)));
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
        let r = confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(3), Some(&c), &cand_limits(), 1755,
        );
        assert!(r.unwrap().contains("single-step only"));
        // Unset steps is also refused.
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, None, Some(&c), &cand_limits(), 1755).is_some());
        // --steps 1 with a clean record → allowed.
        assert!(confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), Some(&c), &cand_limits(), 1755).is_none());
    }

    #[test]
    fn confirmed_refuses_when_no_candidate() {
        let r = confirmed_f2_refusal(&SafeLoopRecord::default(), false, Some(1), None, &cand_limits(), 1755,
        );
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
    fn f2_blacklist_is_scoped_to_one_clock_and_point() {
        let c = cand();
        let mut rec = SafeLoopRecord::default();
        rec.blacklist.push(BlacklistRegion::around(
            f2_intent(1755, &c),
            DEFAULT_BLACKLIST_RADIUS,
        ));
        assert!(candidate_blacklisted(&rec, 1755, &c));
        assert!(
            !candidate_blacklisted(&rec, 1770, &c),
            "one failed clock must not block the remaining multi-clock frontier"
        );
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
            F2DwellResult { outcome: self.dwell, avg_clock_mhz: 1815, p5_clock_mhz: 1815, p95_clock_mhz: 1815, power_w: 183.0,
                max_power_w: 191.0,
                power_p99_w: Some(189.0),
                power_capped_frac: 0.0,
                max_temp_c: Some(62.0),
                thermal_throttled: false,
                measured_voltage_min_mv: Some(949),
                measured_voltage_avg_mv: Some(950),
                measured_voltage_max_mv: Some(951),
                measured_voltage_sample_count: 20,
                render_frames: Some(900),
                render_fps: Some(60.0),
                duration_ms: 15_000,
                sample_count: 300,
                qualification_coverage: None,
            }
        }
        fn reset_to_stock(&mut self) -> Result<(), String> { self.log.push("reset"); self.reset.clone() }
        fn clear_boot_flag(&mut self) -> Result<(), String> { self.log.push("clear"); self.clear.clone() }
        fn blacklist_point(&mut self, counts_as_crash: bool) -> Result<(), String> {
            self.log.push(if counts_as_crash { "blacklist-crash" } else { "blacklist" });
            self.blacklist.clone()
        }
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
        assert_eq!(r.power_w, Some(183));
        assert_eq!(r.max_power_w, Some(191));
        assert_eq!(r.power_p99_w, Some(189.0));
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
        assert!(ops.log.contains(&"blacklist-crash"));
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
        assert!(ops.log.contains(&"blacklist"));
        assert!(!ops.log.contains(&"blacklist-crash"));
    }

    #[test]
    fn confirmed_silent_error_is_boundary_knowledge_not_a_crash() {
        let mut ops = MockOps::happy();
        ops.dwell = F2DwellOutcome::SilentError;
        let r = run_confirmed_f2_step(&mut ops);
        assert_eq!(r.outcome, F2Outcome::SilentError);
        assert!(r.blacklisted);
        assert!(r.boot_flag_cleared);
        assert!(ops.log.contains(&"blacklist"));
        assert!(!ops.log.contains(&"blacklist-crash"));
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
            assert!(matches!(
                *step,
                "arm" | "apply" | "verify" | "dwell" | "reset" | "clear" | "blacklist" | "blacklist-crash"
            ));
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
        vec![(0, 850, 1710), (1, 900, 1725), (2, 950, 1740), (3, 1000, 1748), (4, 1062, 1800),
        ]
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
    fn autonomous_discovery_has_no_six_or_three_step_cap() {
        let curve: Vec<(usize, u32, u32)> = (0..=9)
            .map(|i| (i, 800 + i as u32 * 10, 1815 + i as u32 * 15))
            .collect();
        let limits = PositiveOffsetLimits::hardware_frontier(800, 1950, 1815);
        let d = plan_anchored_undervolt_descent(
            &curve,
            1950,
            None,
            &limits,
            F2_SWEEP_DRYRUN_BUDGET,
        );
        assert_eq!(F2_SWEEP_DRYRUN_BUDGET, usize::MAX);
        assert_eq!(d.candidates.len(), 9);
        assert_eq!(d.candidates.first().unwrap().anchor.voltage_mv, 880);
        assert_eq!(d.candidates.last().unwrap().anchor.voltage_mv, 800);
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
    fn descent_plan_lines_show_unlimited_physical_span_and_no_write() {
        let limits = PositiveOffsetLimits::conservative(850, 1900);
        let d = plan_anchored_undervolt_descent(&d_base(), 1755, None, &limits, 3);
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = anchored_descent_plan_lines(&d, &limits, &pf, d.candidates.len()).join("\n");
        assert!(text.contains("ANCHORED"));
        assert!(text.contains("confirmed span"));
        assert!(text.contains("no arbitrary step cap"));
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
        // Within the caller-declared plan span with candidates + a clean record → allowed. The live
        // discovery passes its full physical candidate count here, not a fixed global cap.
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(1), 1, 3).is_none());
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(2), 2, 3).is_none());
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(3), 3, 3).is_none());
        // No candidates → refuse.
        assert!(confirmed_f2_multi_refusal(&rec, false, Some(2), 0, 3)
            .unwrap()
            .contains("no anchored candidates"));
        // Safe Mode / armed boot flag → refuse. A stale counter alone is not authoritative: actual
        // crash recovery leaves the boot flag armed or promotes the record to Safe Mode.
        let mut sm = SafeLoopRecord::default();
        sm.safe_mode = true;
        assert!(confirmed_f2_multi_refusal(&sm, false, Some(2), 2, 3).unwrap().contains("Safe Mode"));
        assert!(confirmed_f2_multi_refusal(&rec, true, Some(2), 2, 3).unwrap().contains("boot flag"));
        let mut stale_counter = SafeLoopRecord::default();
        stale_counter.consecutive_crashes = SAFE_MODE_CRASH_THRESHOLD;
        assert!(confirmed_f2_multi_refusal(&stale_counter, false, Some(2), 2, 3).is_none());
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
            MockMultiOps { scripts, cur: 0, log: Vec::new(),
            }
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
            F2DwellResult { outcome: self.s().dwell, avg_clock_mhz: 1815, p5_clock_mhz: 1815, p95_clock_mhz: 1815, power_w: 183.0,
                max_power_w: 191.0,
                power_p99_w: Some(189.0),
                power_capped_frac: 0.0,
                max_temp_c: Some(62.0),
                thermal_throttled: false,
                measured_voltage_min_mv: Some(949),
                measured_voltage_avg_mv: Some(950),
                measured_voltage_max_mv: Some(951),
                measured_voltage_sample_count: 20,
                render_frames: Some(900),
                render_fps: Some(60.0),
                duration_ms: 15_000,
                sample_count: 300,
                qualification_coverage: None,
            }
        }
        fn reset_to_stock(&mut self) -> Result<(), String> { self.log.push(format!("reset{}", self.cur)); self.s().reset.clone() }
        fn clear_boot_flag(&mut self) -> Result<(), String> { self.log.push(format!("clear{}", self.cur)); self.s().clear.clone() }
        fn blacklist_point(&mut self, counts_as_crash: bool) -> Result<(), String> {
            self.log.push(format!(
                "{}{}",
                if counts_as_crash { "blacklist-crash" } else { "blacklist" },
                self.cur
            ));
            self.s().blacklist.clone()
        }
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
        let mut ops = MockMultiOps::new(vec![CandScript::stable(), CandScript::stable(), CandScript::stable(),
        ]);
        let r = run_confirmed_f2_multi_step(&mut ops, usize::MAX);
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

    // ── F2 confidence opt-in: extra re-validations of the deepest point (mock; no hardware) ─────
    // A mock that scripts a SEQUENCE of per-PASS outcomes for a SINGLE re-validated candidate, so each
    // extra validation pass can return a different result (the production MockMultiOps keys scripts by
    // candidate index, which is identical across passes). `candidate_count() == 1` (one deepest point).
    struct RevalMockOps {
        passes: Vec<CandScript>,
        pass: usize,
        cur: usize,
        log: Vec<String>,
    }
    impl RevalMockOps {
        fn new(passes: Vec<CandScript>) -> Self {
            RevalMockOps { passes, pass: 0, cur: 0, log: Vec::new(),
            }
        }
        fn s(&self) -> &CandScript {
            // The pass cursor advanced past the script on `select`; the motor reads the just-selected pass.
            &self.passes[self.pass - 1]
        }
    }
    impl F2Ops for RevalMockOps {
        fn arm_boot_flag(&mut self) -> Result<(), String> { self.log.push(format!("arm{}", self.cur)); self.s().arm.clone() }
        fn apply_positive_offset(&mut self) -> Result<(), String> { self.log.push(format!("apply{}", self.cur)); self.s().apply.clone() }
        fn verify(&mut self) -> PositiveOffsetVerification { self.log.push(format!("verify{}", self.cur)); self.s().verify }
        fn dwell(&mut self) -> F2DwellResult {
            self.log.push(format!("dwell{}", self.cur));
            F2DwellResult { outcome: self.s().dwell, avg_clock_mhz: 1815, p5_clock_mhz: 1815, p95_clock_mhz: 1815, power_w: 183.0,
                max_power_w: 191.0,
                power_p99_w: Some(189.0),
                power_capped_frac: 0.0,
                max_temp_c: Some(62.0),
                thermal_throttled: false,
                measured_voltage_min_mv: Some(949),
                measured_voltage_avg_mv: Some(950),
                measured_voltage_max_mv: Some(951),
                measured_voltage_sample_count: 20,
                render_frames: Some(900),
                render_fps: Some(60.0),
                duration_ms: 15_000,
                sample_count: 300,
                qualification_coverage: None,
            }
        }
        fn reset_to_stock(&mut self) -> Result<(), String> { self.log.push(format!("reset{}", self.cur)); self.s().reset.clone() }
        fn clear_boot_flag(&mut self) -> Result<(), String> { self.log.push(format!("clear{}", self.cur)); self.s().clear.clone() }
        fn blacklist_point(&mut self, counts_as_crash: bool) -> Result<(), String> {
            self.log.push(format!(
                "{}{}",
                if counts_as_crash { "blacklist-crash" } else { "blacklist" },
                self.cur
            ));
            self.s().blacklist.clone()
        }
    }
    impl F2MultiStepOps for RevalMockOps {
        fn candidate_count(&self) -> usize { 1 }
        fn select(&mut self, i: usize) -> Result<(), String> {
            self.cur = i;
            let p = self.pass;
            self.pass += 1;
            self.log.push(format!("select{i}#{p}"));
            self.passes[p].precheck.clone()
        }
    }

    #[test]
    fn extra_validations_default_one_does_no_extra_passes() {
        // validation_passes == 1 ⇒ extra_passes == 0 ⇒ NO re-validation runs (today's behavior).
        let mut ops = RevalMockOps::new(vec![CandScript::stable(); 4]);
        let reports = run_confirmed_f2_extra_validations(&mut ops, 0, 0);
        assert!(reports.is_empty());
        assert!(ops.log.is_empty()); // never armed/applied/reset anything
    }

    #[test]
    fn extra_validations_revalidates_deepest_n_minus_one_times() {
        // validation_passes == 3 ⇒ 2 EXTRA re-validations of the deepest point, each a full reset-clean
        // pass; 2 reports returned (one observation per pass would be recorded by the caller).
        let mut ops = RevalMockOps::new(vec![CandScript::stable(); 3]);
        let reports = run_confirmed_f2_extra_validations(&mut ops, 0, 2);
        assert_eq!(reports.len(), 2);
        for r in &reports {
            assert!(r.validated);
            assert_eq!(r.reset_ok, Some(true)); // reset after EVERY pass
            assert!(r.boot_flag_cleared);
        }
        // Both passes selected the SAME deepest index (0) and ran the full motor each time.
        assert_eq!(
            ops.log,
            vec![
                "select0#0", "arm0", "apply0", "verify0", "dwell0", "reset0", "clear0",
                "select0#1", "arm0", "apply0", "verify0", "dwell0", "reset0", "clear0",
            ]
        );
    }

    #[test]
    fn extra_validations_stop_immediately_on_unstable_pass() {
        // Pass 1 stable, pass 2 Unstable ⇒ the loop STOPS (no pass 3), even though 3 extra were requested.
        let mut ops = RevalMockOps::new(vec![
            CandScript::stable(),
            CandScript::stable().with_dwell(F2DwellOutcome::Unstable),
            CandScript::stable(),
        ]);
        let reports = run_confirmed_f2_extra_validations(&mut ops, 0, 3);
        assert_eq!(reports.len(), 2); // stopped after the unstable pass; pass 3 never ran
        assert!(reports[0].validated);
        assert!(!reports[1].validated);
        assert!(matches!(reports[1].outcome, F2Outcome::Unstable));
        assert!(!ops.log.iter().any(|l| l == "select0#2")); // never started a 3rd pass
    }

    #[test]
    fn extra_validations_stop_on_precheck_refusal() {
        // A pass whose Safe Loop / blacklist precheck refuses STOPS further passes and writes nothing.
        let mut ops = RevalMockOps::new(vec![
            CandScript::stable(),
            CandScript { precheck: Err("blacklisted".to_string()), ..CandScript::stable() },
        ]);
        let reports = run_confirmed_f2_extra_validations(&mut ops, 0, 3);
        assert_eq!(reports.len(), 1); // only the first pass produced a report
        assert!(ops.log.iter().any(|l| l == "select0#1")); // precheck happened on pass 2
        // Exactly one full motor pass ran; pass 2 was refused before any write.
        assert_eq!(ops.log.iter().filter(|l| l.starts_with("arm")).count(), 1);
    }

    #[test]
    fn f2_max_validation_passes_is_bounded() {
        // The hard cap exists and is a small, bounded number (never unbounded hardware time).
        assert_eq!(F2_MAX_VALIDATION_PASSES, 20);
        // A request above the cap is REFUSED by the auto-sweep confirm gate (see run_anchored_target_sweep);
        // the gate compares args.validation_passes > F2_MAX_VALIDATION_PASSES and fails closed.
        let refused = 21usize;
        assert!(refused > F2_MAX_VALIDATION_PASSES);
    }

    // ── F2 MANUAL-PRIOR (explicit dev/known-GPU shortcut; pure planner + gate; no hardware) ─────
    // Anchor-at-875 fixture: the 875 mV bin needs a large raise (+210) to hold 1800 (base 1590); the
    // 1062 mV bin sits above target so the plateau cap engages; the 850 mV bin sits below the anchor
    // so the elastic case is exercised. (idx, mV, base_mhz).
    fn mp_base() -> Vec<(usize, u32, u32)> {
        vec![(0, 850, 1560), (1, 875, 1590), (2, 900, 1740), (3, 950, 1770), (4, 1000, 1800), (5, 1062, 1845),
        ]
    }

    #[test]
    fn parse_reads_manual_prior_flag_default_false() {
        // Manual-prior is opt-in: absent → false (NOT the default).
        assert!(!parse_undervolt_args(&os(&["undervolt-probe"])).unwrap().manual_prior);
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--manual-prior"])).unwrap().manual_prior);
        // It composes with the other flags without disturbing them.
        let a = parse_undervolt_args(&os(&[
            "undervolt-probe", "--target-mhz", "1800", "--start-mv", "875", "--steps", "1", "--manual-prior",
        ]))
        .unwrap();
        assert_eq!((a.target_mhz, a.start_mv, a.steps, a.manual_prior), (Some(1800), Some(875), Some(1), true));
    }

    #[test]
    fn manual_prior_limits_widen_only_the_offset_caps() {
        // Manual-prior widens ONLY the offset caps (abs + per-step to the same value); floor/ceiling
        // are unchanged, and the DEFAULT conservative caps (+30 / +15) are a separate, untouched thing.
        let m = PositiveOffsetLimits::manual_prior(612, 1950, 250);
        assert_eq!((m.abs_max_offset_mhz, m.step_max_offset_mhz), (250, 250));
        assert_eq!((m.hw_floor_mv, m.clock_ceiling_mhz), (612, 1950));
        let c = PositiveOffsetLimits::conservative(612, 1950);
        assert_eq!((c.abs_max_offset_mhz, c.step_max_offset_mhz), (30, 15));
    }

    #[test]
    fn manual_prior_selects_anchor_and_computes_required_offset() {
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1800, 875, &m);
        assert_eq!(p.selected_mv, Some(875));
        assert_eq!(p.base_mhz, Some(1590));
        assert_eq!(p.required_offset_mhz, Some(210)); // 1800 - 1590
        assert!(p.within_bounds);
        let plan = p.probe.plan.expect("within bounds → a plan");
        assert_eq!(
            (plan.anchor.voltage_mv, plan.anchor.offset_mhz, plan.anchor.effective_mhz),
            (875, 210, 1800)
        );
    }

    #[test]
    fn manual_prior_caps_higher_bins_and_keeps_lower_elastic() {
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let plan = plan_manual_prior_undervolt(&mp_base(), 1800, 875, &m).probe.plan.unwrap();
        // 1062 (base 1845 > 1800) capped DOWN; 850 (below the anchor) left elastic.
        assert_eq!(plan.capped_above_bins, 1);
        assert_eq!(plan.elastic_below_bins, 1);
        // No bin above target, and ONLY the anchor carries a positive offset.
        assert!(plan.entries.iter().all(|e| e.effective_mhz <= 1800));
        assert_eq!(plan.entries.iter().filter(|e| e.offset_mhz > 0).count(), 1);
    }

    #[test]
    fn manual_prior_rejects_required_offset_above_cap() {
        // Target 1900 @ 875 (base 1590) needs +310 > cap 250 → REFUSED (never clamped), no plan.
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1900, 875, &m);
        assert_eq!(p.required_offset_mhz, Some(310));
        assert!(!p.within_bounds);
        assert!(p.probe.plan.is_none());
        let note = p.note.as_deref().unwrap_or_default();
        assert!(note.contains("absolute cap") && note.contains("250"));
    }

    #[test]
    fn manual_prior_rejects_below_floor_bin() {
        // Floor 900: the 875 mV anchor is below the floor → REFUSED on the floor check (not clamped).
        let m = PositiveOffsetLimits::manual_prior(900, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1800, 875, &m);
        assert_eq!(p.selected_mv, Some(875));
        assert!(!p.within_bounds);
        assert!(p.note.as_deref().unwrap_or_default().contains("floor"));
    }

    #[test]
    fn manual_prior_rejects_when_nothing_to_anchor() {
        // start-mv 700 is below every real bin (lowest is 850 mV) → no anchor candidate, no plan.
        let m = PositiveOffsetLimits::manual_prior(600, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1800, 700, &m);
        assert_eq!(p.selected_mv, None);
        assert_eq!(p.required_offset_mhz, None);
        assert!(!p.within_bounds);
        assert!(p.note.is_some());
    }

    #[test]
    fn manual_prior_nonexact_start_mv_resolves_to_nearest_real_bin() {
        // 880 mV is not a real bin → resolves to the nearest real bin at/below it (875 mV).
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1800, 880, &m);
        assert_eq!(p.requested_start_mv, 880);
        assert_eq!(p.selected_mv, Some(875));
    }

    #[test]
    fn confirmed_manual_prior_requires_start_mv_and_single_step() {
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let cand = plan_manual_prior_undervolt(&mp_base(), 1800, 875, &m).probe.plan.unwrap().anchor;
        let rec = SafeLoopRecord::default();
        // Missing --start-mv → refuse.
        assert!(confirmed_manual_prior_refusal(&rec, false, None, Some(1), Some(&cand), &m, 1800)
            .unwrap()
            .contains("requires an explicit --start-mv"));
        // --steps > 1 → refuse (single-step only for first hardware validation).
        assert!(confirmed_manual_prior_refusal(&rec, false, Some(875), Some(2), Some(&cand), &m, 1800)
            .unwrap()
            .contains("single-step only"));
        // start-mv + steps 1 + a valid in-bounds candidate + clean record → allowed.
        assert!(confirmed_manual_prior_refusal(&rec, false, Some(875), Some(1), Some(&cand), &m, 1800).is_none());
    }

    #[test]
    fn manual_prior_plan_lines_show_warning_required_offset_and_no_write() {
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1800, 875, &m);
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = manual_prior_plan_lines(&p, &m, &pf).join("\n");
        assert!(text.contains("MANUAL-PRIOR"));
        assert!(text.contains("uses user-provided prior; not the default unknown-GPU discovery path"));
        assert!(text.contains("requested start-mv : 875"));
        assert!(text.contains("selected anchor"));
        assert!(text.contains("required offset    : +210"));
        assert!(text.contains("manual offset cap  : +250"));
        assert!(text.contains("DEFAULT discovery cap")); // shows the default cap stays unaffected
        assert!(text.contains("within bounds      : YES"));
        // No-op / no-write + no-persist/apply/promote semantics.
        assert!(text.contains("no Safe Loop arm"));
        assert!(text.contains("no apply"));
        assert!(text.contains("no dwell"));
        assert!(text.contains("no VF write"));
        assert!(text.contains("none persisted, applied, or promoted"));
    }

    #[test]
    fn manual_prior_plan_lines_report_over_cap_refusal_no_write() {
        let m = PositiveOffsetLimits::manual_prior(800, 1950, 250);
        let p = plan_manual_prior_undervolt(&mp_base(), 1900, 875, &m); // +310 > cap 250
        let pf = undervolt_preflight(&SafeLoopRecord::default(), false, &[]);
        let text = manual_prior_plan_lines(&p, &m, &pf).join("\n");
        assert!(text.contains("required offset    : +310"));
        assert!(text.contains("manual offset cap  : +250"));
        assert!(text.contains("within bounds      : NO"));
        assert!(text.contains("no VF write"));
    }

    #[test]
    fn manual_prior_does_not_affect_default_discovery_caps() {
        // The SAME bin/target manual-prior admits (+210) is REFUSED under the DEFAULT conservative
        // caps (+30) — proving manual-prior never widens default/autonomous discovery.
        let default_limits = PositiveOffsetLimits::conservative(800, 1950);
        let probe = plan_anchored_undervolt(&mp_base(), 1800, Some(875), &default_limits);
        assert!(probe.plan.is_none());
        assert!(probe.note.as_deref().unwrap_or_default().contains("+30"));
    }

    #[test]
    fn usage_lists_manual_prior() {
        let u = undervolt_usage();
        assert!(u.contains("--manual-prior"));
        assert!(u.contains("--start-mv")); // manual-prior requires it
        assert!(u.to_uppercase().contains("MANUAL-PRIOR"));
        assert!(!u.contains("candidate bins"));
    }
}
