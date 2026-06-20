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
//! This module is the no-hardware / dry-run-first foundation:
//! - a pure search skeleton ([`plan_undervolt_probe`]) that descends real voltage bins and computes
//!   the bounded positive offset each bin needs to hold the focus target, stopping at the first
//!   bound/floor violation;
//! - a pure Safe Loop preflight ([`undervolt_preflight`]) that REFUSES an unsafe state (read-only);
//! - the dry-run plan formatter ([`undervolt_plan_lines`]); and
//! - the `undervolt-probe` entry ([`run_undervolt_probe`]), whose `--confirm` branch FAILS CLOSED
//!   (confirmed hardware F2 is not implemented in this patch).
//!
//! Safety invariants honored here: no profile apply/persist/promotion, no multi-target loop, no
//! autonomous crash-seeking, no power-limit/TDP/clock-lock change, and the dry-run reads Safe Loop
//! state READ-ONLY (it never arms the boot flag, mutates the record, applies, dwells, or writes VF).

use std::ffi::OsString;

use nidavellir_core::safe_loop::{SafeLoopRecord, SafeLoopStore, TuningPoint};
use nidavellir_gpu_nvapi::{plan_bounded_positive_offset, PositiveOffsetLimits, PositiveOffsetPlan};

#[cfg(windows)]
use tracing::{info, warn};

/// Default number of descent steps (candidate bins) when `--steps` is omitted. Small by design —
/// F2 v1 is a single focus target with a small bounded raise.
const F2_DEFAULT_STEPS: usize = 4;

/// Tolerance (MHz) for the plan/verifier self-consistency check (one boost bin).
#[cfg(windows)]
const F2_VERIFY_TOL_MHZ: u32 = 15;

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

/// Parsed `undervolt-probe` flags (syntax only; missing/non-numeric values FAIL CLOSED). `--confirm`
/// is handled separately by the caller (and refused this patch).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndervoltArgs {
    pub target_mhz: Option<u32>,
    pub start_mv: Option<u32>,
    pub steps: Option<usize>,
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
/// blacklisted region. Returns the verdict; mutates nothing. The (future) confirmed path must honor
/// a `!safe` verdict before any hardware write.
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

    let plan = plan_undervolt_probe(&sane, focus_target, args.start_mv, &limits, max_steps);

    // Read-only Safe Loop preflight over the planned points (axis keys match build-frontier's intent).
    let record = store.load_record();
    let boot_flag_armed = store.is_boot_flag_armed();
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
        // F2 confirmed mode is NOT implemented in this patch — fail closed; touch no hardware.
        //
        // TODO(F2 confirmed): the supervised one-step path must, in order:
        //   1. honor the preflight verdict (refuse on safe_mode / armed boot flag / blacklisted point);
        //   2. ARM the Safe Loop boot flag (BootFlag::new with the F2 intent) BEFORE any positive write;
        //   3. apply ONE bounded positive offset via apply_bounded_positive_offset;
        //   4. dwell under load, then verify via verify_positive_offset;
        //   5. reset_to_stock and clear the boot flag ONLY after a clean dwell + reset;
        //   6. on a crash/TDR/reboot, the still-armed boot flag lets startup recovery blacklist the
        //      point and recede to last-known-good; STOP on the first crash/TDR/instability/verifier
        //      failure (no multi-target loop, no autonomous crash-seeking).
        // None of that runs here.
        println!(
            "undervolt-probe: --confirm REFUSED — confirmed F2 undervolt-probe is not implemented in \
             this patch. No Safe Loop arm, no apply, no dwell, no VF write performed."
        );
        warn!("undervolt-probe: --confirm refused (F2 confirmed mode not implemented) — no hardware touched");
        return;
    }
    println!("(dry-run — F2 confirmed mode is not implemented in this patch; nothing was written)");
    info!(
        "undervolt-probe: DRY-RUN — target={} MHz floor={} mV ceiling={} MHz steps={} candidates={} \
         preflight_safe={} — no Safe Loop arm, no apply, no dwell, no VF write.",
        focus_target, floor_mv, boost_top, max_steps, plan.points.len(), preflight.safe
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
        // Defaults when absent.
        let d = parse_undervolt_args(&os(&["undervolt-probe"])).unwrap();
        assert_eq!((d.target_mhz, d.start_mv, d.steps), (None, None, None));
        // Missing / non-numeric values fail closed.
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--target-mhz"])).is_err());
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--steps", "x"])).is_err());
        assert!(parse_undervolt_args(&os(&["undervolt-probe", "--start-mv", "abc"])).is_err());
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
}
