//! Read-only verification of whether the live modern VF curve matches the applied
//! Nidavellir profile (Patch A: **curve-only**). It NEVER writes, reapplies, or runs
//! stress — it reads the modern ClkVfPoints curve (GetStatus + GET-control offsets)
//! and classifies.
//!
//! Comparison is **table-to-table** against the deterministic VF-table ceiling bin,
//! re-derived the SAME way `gpu_apply` derives the apply key (snap the measured/legacy
//! voltage to a real bin via `nearest_vf_bin_at_or_above`) — never the measured value.
//! Patch B adds a LOAD axis derived from the applied point's EXISTING synthetic-dwell
//! stats (no new stress run). Live real-game workload context remains future work.

use nidavellir_core::ipc::{
    ApplyVerificationStatus, CurveVerification, DwellQuality, LoadVerification, PowerSweepPoint,
    PowerSweepProgress,
};
#[cfg(windows)]
use tracing::info;

/// GetStatus clock tolerance for the SECONDARY corroboration count: ~one boost bin.
const TOL_MHZ: u32 = 15;

/// Evidence for one expected-flattened curve point (a point at/above the ceiling bin).
#[derive(Debug, Clone, Copy)]
struct PointEvidence {
    /// Applied frequency offset (kHz) from `vf_get_point_khz` — the PRIMARY signal.
    /// `None` = the offset readback failed for this point.
    offset_khz: Option<i32>,
    /// GetStatus actual freq (MHz) — SECONDARY corroboration / diagnostics only.
    freq_mhz: u32,
}

/// Classify the applied VF ceiling using the GET-control **offset readback** as the
/// primary criterion. Runtime QA proved GetStatus actual-freq is unreliable at idle —
/// it under-reports the plateau even when the flatten offsets are resident (see
/// `handoff.md`). A point at/above the deterministic ceiling bin counts as flattened
/// when it carries a non-zero applied frequency offset.
///
/// We verify **presence** of the flatten offset, not its exact value: the exact
/// expected offset is `target - stock_base_freq` per point and the per-point stock
/// base is not persisted, so an exact comparison isn't available. Presence of a
/// non-zero offset on the expected points is the strongest signal we can read.
///
/// Returns `(state, offset_present, freq_match, expected_n)`. `freq_match` (GetStatus)
/// is for logging only and never gates. Pure + testable.
fn classify_curve(
    target_mhz: u32,
    expected: &[PointEvidence],
    tol_mhz: u32,
) -> (CurveVerification, u32, u32, u32) {
    let expected_n = expected.len() as u32;
    let freq_match = expected
        .iter()
        .filter(|e| e.freq_mhz.abs_diff(target_mhz) <= tol_mhz)
        .count() as u32;
    if expected_n == 0 {
        // Ceiling above every bin / no plateau region → can't evaluate.
        return (CurveVerification::VerificationFailed, 0, freq_match, 0);
    }
    let readable = expected.iter().filter(|e| e.offset_khz.is_some()).count();
    if readable == 0 {
        // Primary evidence unreadable → report failure, don't assert a mismatch (safer).
        return (CurveVerification::VerificationFailed, 0, freq_match, expected_n);
    }
    let offset_present = expected
        .iter()
        .filter(|e| e.offset_khz.map_or(false, |o| o != 0))
        .count() as u32;
    // Require ≥90% of expected points to carry the flatten offset (tolerate the
    // boundary point — its stock base can already equal target → zero offset).
    let ratio = offset_present as f32 / expected_n as f32;
    let state = if offset_present >= 1 && ratio >= 0.9 {
        CurveVerification::VerifiedCurve
    } else {
        CurveVerification::LiveMismatch
    };
    (state, offset_present, freq_match, expected_n)
}

/// Clock tolerance (MHz) for the load-state p5 check: ~two boost bins.
const LOAD_CLOCK_TOL_MHZ: u32 = 30;
/// Clock match tolerance (MHz) when confirming the applied point. Apply sets
/// `core.freq_mhz = point.clock_mhz`, so this is effectively an exact check.
const APPLIED_MATCH_TOL_MHZ: u32 = 5;
/// Plausible measured GPU voltage range (mV) for the load-state sanity check.
const VOLT_SANE_MIN_MV: u32 = 500;
const VOLT_SANE_MAX_MV: u32 = 1250;

/// Find the `PowerSweepPoint` for the applied profile. Primary: the named slot the
/// label maps to (with a clock sanity check). Fallback: a UNIQUE `points` entry whose
/// clock matches the target. Ambiguous / no match → `None` (never guess). Pure.
fn find_applied_point(
    progress: &PowerSweepProgress,
    label: &str,
    target_mhz: u32,
) -> Option<PowerSweepPoint> {
    let by_label = match label {
        "Godforge" => progress.godforge,
        "Brokkr's Best" => progress.brokkrs,
        "Deep Calm" => progress.deep_calm,
        _ => None,
    };
    if let Some(p) = by_label {
        if p.clock_mhz.abs_diff(target_mhz) <= APPLIED_MATCH_TOL_MHZ {
            return Some(p);
        }
    }
    // Fallback: a single points entry near the target clock; ambiguous → give up.
    let mut it = progress
        .points
        .iter()
        .filter(|p| p.clock_mhz.abs_diff(target_mhz) <= APPLIED_MATCH_TOL_MHZ);
    let first = it.next().copied();
    if it.next().is_some() {
        return None;
    }
    first
}

/// Classify the LOAD axis from the applied point's EXISTING synthetic-dwell stats.
/// Pure; never runs a stress test. `point` = the dwell stats for the applied profile
/// (`None` if no confident match). Returns `(load_state, reason)`.
///
/// Rules: load is only evaluated when the curve is verified; `p5_clock` is the primary
/// sustained-clock signal (min_clock stays diagnostic); measured voltage is telemetry
/// only (never an apply key) and only downgrades when implausible.
fn classify_load(
    curve_state: CurveVerification,
    point: Option<&PowerSweepPoint>,
    target_mhz: u32,
    clock_tol_mhz: u32,
) -> (LoadVerification, String) {
    if curve_state != CurveVerification::VerifiedCurve {
        return (LoadVerification::NotEvaluated, "curve not verified".into());
    }
    let Some(p) = point else {
        return (LoadVerification::NotEvaluated, "no matching dwell stats".into());
    };
    if !p.stable {
        return (
            LoadVerification::LoadMismatch,
            "applied point was not a stable measurement".into(),
        );
    }
    // Power sanity → implausible telemetry cannot support a load claim.
    if p.power_w <= 0.0 || p.max_power_w < p.power_w || !(0.0..=1.0).contains(&p.power_capped_frac) {
        return (
            LoadVerification::LoadVerificationFailed,
            "implausible power telemetry".into(),
        );
    }
    // Telemetry-quality gate (require ≥ Medium).
    match p.telemetry_quality {
        None => {
            return (
                LoadVerification::TelemetryInsufficient,
                "legacy point without dwell quality".into(),
            )
        }
        Some(DwellQuality::Low) | Some(DwellQuality::Unavailable) => {
            return (
                LoadVerification::TelemetryInsufficient,
                "telemetry_quality low/unavailable".into(),
            )
        }
        _ => {}
    }
    // Measured voltage is telemetry only; implausible → insufficient (not a fail).
    if let Some(mv) = p.max_measured_voltage_mv {
        if !(VOLT_SANE_MIN_MV..=VOLT_SANE_MAX_MV).contains(&mv) {
            return (
                LoadVerification::TelemetryInsufficient,
                "measured voltage out of sane range".into(),
            );
        }
    }
    // Sustained-clock check — p5 required and primary.
    let Some(p5) = p.p5_clock_mhz else {
        return (
            LoadVerification::TelemetryInsufficient,
            "no p5_clock_mhz in dwell stats".into(),
        );
    };
    if p5 + clock_tol_mhz < target_mhz {
        return (
            LoadVerification::LoadMismatch,
            format!("p5_clock {p5} below target {target_mhz} (tol {clock_tol_mhz} MHz)"),
        );
    }
    (
        LoadVerification::VerifiedUnderLoad,
        format!("synthetic-dwell stats support the point (p5_clock {p5} >= {target_mhz}-{clock_tol_mhz})"),
    )
}

/// Derive the effective headline state (for the log / a UI summary) from the two axes:
/// load verification can UPGRADE a verified curve to "verified under load", but absent
/// or weak load data never DOWNGRADES the curve verdict. Pure + testable.
fn effective_status(curve: CurveVerification, load: LoadVerification) -> &'static str {
    match (curve, load) {
        (CurveVerification::VerifiedCurve, LoadVerification::VerifiedUnderLoad) => {
            "verified_under_load"
        }
        (CurveVerification::VerifiedCurve, _) => "verified_curve",
        (CurveVerification::LiveMismatch, _) => "live_mismatch",
        (CurveVerification::VerificationFailed, _) => "verification_failed",
        (CurveVerification::MetadataOnly, _) => "metadata_only",
        (CurveVerification::NotApplicable, _) => "not_applicable",
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn make_status(
    status: CurveVerification,
    label: Option<String>,
    target_mhz: Option<u32>,
    vf_table_voltage_mv: Option<u32>,
    legacy_voltage_mv: Option<u32>,
    matched_points: Option<u32>,
    expected_points: Option<u32>,
    message: impl Into<String>,
) -> ApplyVerificationStatus {
    ApplyVerificationStatus {
        live_curve_match: status == CurveVerification::VerifiedCurve,
        status,
        label,
        target_mhz,
        vf_table_voltage_mv,
        legacy_voltage_mv,
        matched_points,
        expected_points,
        message: message.into(),
        // Load axis is filled in separately (only on a decided curve verdict).
        load_state: None,
        load_reason: None,
        telemetry_match: None,
        p5_clock_mhz: None,
        min_clock_mhz: None,
        avg_measured_voltage_mv: None,
        min_measured_voltage_mv: None,
        max_measured_voltage_mv: None,
        voltage_sample_count: None,
        voltage_quality: None,
        telemetry_quality: None,
    }
}

/// Fill the LOAD axis on a status whose curve verdict is already decided. Read-only:
/// loads the persisted forge result (`forge_state.json`) to locate the applied point's
/// dwell stats, classifies the load state, and copies diagnostic telemetry. Never
/// mutates GPU state.
#[cfg(windows)]
fn fill_load_axis(st: &mut ApplyVerificationStatus, curve_state: CurveVerification, target_mhz: u32) {
    let point = if curve_state == CurveVerification::VerifiedCurve {
        crate::gpu_power_sweep::load_restored_progress()
            .as_ref()
            .and_then(|pr| find_applied_point(pr, st.label.as_deref().unwrap_or(""), target_mhz))
    } else {
        None
    };
    let (load_state, reason) =
        classify_load(curve_state, point.as_ref(), target_mhz, LOAD_CLOCK_TOL_MHZ);
    st.load_state = Some(load_state);
    st.load_reason = Some(reason);
    st.telemetry_match = Some(load_state == LoadVerification::VerifiedUnderLoad);
    if let Some(p) = &point {
        st.p5_clock_mhz = p.p5_clock_mhz;
        st.min_clock_mhz = p.min_clock_mhz;
        st.avg_measured_voltage_mv = p.avg_measured_voltage_mv;
        st.min_measured_voltage_mv = p.min_measured_voltage_mv;
        st.max_measured_voltage_mv = p.max_measured_voltage_mv;
        st.voltage_sample_count = p.voltage_sample_count;
        st.voltage_quality = p.voltage_quality;
        st.telemetry_quality = p.telemetry_quality;
    }
}

/// Read-only verification of the applied profile against the live modern VF curve.
/// Reads only (GetStatus + GET-control offsets); never mutates GPU state.
#[cfg(windows)]
pub fn verify_applied_curve() -> ApplyVerificationStatus {
    let Some(ap) = crate::gpu_apply::load_applied() else {
        return make_status(
            CurveVerification::NotApplicable,
            None,
            None,
            None,
            None,
            None,
            None,
            "no applied profile recorded",
        );
    };
    let label = if ap.label.is_empty() { None } else { Some(ap.label.clone()) };
    let Some(core) = ap.core else {
        return make_status(
            CurveVerification::MetadataOnly,
            label,
            None,
            None,
            None,
            None,
            None,
            "applied profile has no core point (memory-only) — nothing to verify",
        );
    };
    if !nidavellir_gpu_nvapi::vf_curve_supported() {
        return make_status(
            CurveVerification::VerificationFailed,
            label,
            Some(core.freq_mhz),
            None,
            Some(core.voltage_mv),
            None,
            None,
            "modern VF curve API unsupported on this GPU/driver",
        );
    }
    let live = nidavellir_gpu_nvapi::read_vf_curve_modern();
    if live.is_empty() {
        return make_status(
            CurveVerification::VerificationFailed,
            label,
            Some(core.freq_mhz),
            None,
            Some(core.voltage_mv),
            None,
            None,
            "modern VF curve readback returned no points",
        );
    }
    // Re-derive the deterministic ceiling bin the SAME way apply does (snap the
    // measured/legacy voltage to a real VF-table bin) — never the measured value.
    let Some((ceiling_idx, ceiling_mv)) =
        nidavellir_gpu_nvapi::nearest_vf_bin_at_or_above(&live, core.voltage_mv)
    else {
        return make_status(
            CurveVerification::VerificationFailed,
            label,
            Some(core.freq_mhz),
            None,
            Some(core.voltage_mv),
            None,
            None,
            "could not map applied voltage to a VF-table bin",
        );
    };
    // PRIMARY evidence: the GET-control offset readback per expected point. GetStatus
    // actual freq is kept only as secondary corroboration (see classify_curve docs).
    let expected: Vec<PointEvidence> = live
        .iter()
        .filter(|(_, mv, _)| *mv >= ceiling_mv)
        .map(|(i, _, freq)| PointEvidence {
            offset_khz: nidavellir_gpu_nvapi::vf_get_point_khz(*i),
            freq_mhz: *freq,
        })
        .collect();
    let (state, offset_present, freq_match, expected_n) =
        classify_curve(core.freq_mhz, &expected, TOL_MHZ);
    let message = match state {
        CurveVerification::VerifiedCurve => format!(
            "live curve matches: {offset_present}/{expected_n} plateau points carry the flatten \
             offset (GetStatus freq match {freq_match}/{expected_n}, diagnostic)"
        ),
        CurveVerification::LiveMismatch => format!(
            "live curve mismatch: only {offset_present}/{expected_n} plateau points carry the \
             flatten offset (GetStatus freq match {freq_match}/{expected_n})"
        ),
        _ => format!(
            "verification incomplete: could not read VF offsets for the {expected_n} plateau points"
        ),
    };
    let mut st = make_status(
        state,
        label.clone(),
        Some(core.freq_mhz),
        Some(ceiling_mv),
        Some(core.voltage_mv),
        Some(offset_present),
        Some(expected_n),
        message,
    );
    // LOAD axis (Patch B): derive from the applied point's existing dwell stats.
    fill_load_axis(&mut st, state, core.freq_mhz);
    let load = st.load_state.unwrap_or(LoadVerification::NotEvaluated);
    let headline = effective_status(state, load);
    info!(
        "apply_verify: label={:?} target={} vf_table_mv={} legacy_mv={} ceiling_idx={} \
         curve_state={:?} offset_match={}/{} getstatus_freq_match={}/{} load_state={:?} \
         p5_clock={:?} telemetry_quality={:?} voltage_quality={:?} status={}",
        label, core.freq_mhz, ceiling_mv, core.voltage_mv, ceiling_idx, state, offset_present,
        expected_n, freq_match, expected_n, load, st.p5_clock_mhz, st.telemetry_quality,
        st.voltage_quality, headline
    );
    st
}

#[cfg(not(windows))]
pub fn verify_applied_curve() -> ApplyVerificationStatus {
    ApplyVerificationStatus {
        status: CurveVerification::VerificationFailed,
        label: None,
        target_mhz: None,
        vf_table_voltage_mv: None,
        legacy_voltage_mv: None,
        matched_points: None,
        expected_points: None,
        live_curve_match: false,
        message: "applied-curve verification is Windows-only".into(),
        load_state: None,
        load_reason: None,
        telemetry_match: None,
        p5_clock_mhz: None,
        min_clock_mhz: None,
        avg_measured_voltage_mv: None,
        min_measured_voltage_mv: None,
        max_measured_voltage_mv: None,
        voltage_sample_count: None,
        voltage_quality: None,
        telemetry_quality: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(offset_khz: Option<i32>, freq_mhz: u32) -> PointEvidence {
        PointEvidence { offset_khz, freq_mhz }
    }

    #[test]
    fn offsets_present_but_getstatus_freq_mismatch_is_verified() {
        // The real idle case (offsets_nonzero=63/65, getstatus 31/65): every plateau
        // point carries a flatten offset but GetStatus reports a non-target freq.
        // Offset readback is primary → VerifiedCurve.
        let expected: Vec<PointEvidence> = (0..10).map(|_| ev(Some(-120_000), 1500)).collect();
        let (s, present, freq_match, n) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerifiedCurve);
        assert_eq!((present, freq_match, n), (10, 0, 10));
    }

    #[test]
    fn missing_offsets_is_live_mismatch() {
        // Offsets readable but all zero (reset / never flattened) → mismatch.
        let expected: Vec<PointEvidence> = (0..10).map(|_| ev(Some(0), 1900)).collect();
        let (s, present, _, _) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!(s, CurveVerification::LiveMismatch);
        assert_eq!(present, 0);
    }

    #[test]
    fn partial_offsets_below_threshold_is_live_mismatch() {
        // 5/10 carry the offset → 0.5 < 0.9 → mismatch.
        let mut expected: Vec<PointEvidence> = (0..5).map(|_| ev(Some(-100_000), 1770)).collect();
        expected.extend((0..5).map(|_| ev(Some(0), 1900)));
        let (s, present, _, n) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!((present, n), (5, 10));
        assert_eq!(s, CurveVerification::LiveMismatch);
    }

    #[test]
    fn large_plateau_one_missing_offset_is_verified() {
        // 9/10 carry the offset → ≥90% → Verified (boundary point already at target).
        let mut expected: Vec<PointEvidence> = (0..9).map(|_| ev(Some(-90_000), 1770)).collect();
        expected.push(ev(Some(0), 1770));
        let (s, present, _, n) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!((present, n), (9, 10));
        assert_eq!(s, CurveVerification::VerifiedCurve);
    }

    #[test]
    fn empty_expected_set_is_verification_failed() {
        let (s, _, _, n) = classify_curve(1770, &[], TOL_MHZ);
        assert_eq!(s, CurveVerification::VerificationFailed);
        assert_eq!(n, 0);
    }

    #[test]
    fn unreadable_offsets_is_verification_failed() {
        // Every offset read failed (None) → can't evaluate primary evidence → failed,
        // not mismatch (safer).
        let expected: Vec<PointEvidence> = (0..6).map(|_| ev(None, 1770)).collect();
        let (s, present, _, n) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerificationFailed);
        assert_eq!((present, n), (0, 6));
    }

    // ── Patch B: load classification ───────────────────────────────────────────
    // A "good" applied dwell point: stable, plausible power, Medium telemetry, p5 set.
    fn good_point(clock: u32, p5: u32) -> PowerSweepPoint {
        PowerSweepPoint {
            voltage_mv: 843,
            clock_mhz: clock,
            power_w: 180.0,
            max_power_w: 185.0,
            power_capped_frac: 0.2,
            stable: true,
            perf_per_watt: clock as f64 / 180.0,
            p5_clock_mhz: Some(p5),
            min_clock_mhz: Some(p5.saturating_sub(20)),
            avg_measured_voltage_mv: Some(862),
            max_measured_voltage_mv: Some(869),
            voltage_quality: Some(DwellQuality::Medium),
            telemetry_quality: Some(DwellQuality::Medium),
            ..Default::default()
        }
    }

    #[test]
    fn verified_curve_with_good_stats_is_verified_under_load() {
        let p = good_point(1770, 1765);
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::VerifiedUnderLoad);
    }

    #[test]
    fn verified_curve_with_no_point_is_not_evaluated() {
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, None, 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::NotEvaluated);
    }

    #[test]
    fn verified_curve_with_low_quality_is_telemetry_insufficient() {
        let mut p = good_point(1770, 1765);
        p.telemetry_quality = Some(DwellQuality::Low);
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::TelemetryInsufficient);
    }

    #[test]
    fn legacy_point_without_quality_is_telemetry_insufficient() {
        let mut p = good_point(1770, 1765);
        p.telemetry_quality = None; // pre-dwell-stats persisted point
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::TelemetryInsufficient);
    }

    #[test]
    fn verified_curve_with_low_p5_is_load_mismatch() {
        // p5 1720 vs target 1770, tol 30 → 1720+30=1750 < 1770 → mismatch.
        let p = good_point(1770, 1720);
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::LoadMismatch);
    }

    #[test]
    fn unstable_point_is_load_mismatch() {
        let mut p = good_point(1770, 1765);
        p.stable = false;
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::LoadMismatch);
    }

    #[test]
    fn implausible_power_is_load_verification_failed() {
        let mut p = good_point(1770, 1765);
        p.power_w = 0.0;
        let (l, _) = classify_load(CurveVerification::VerifiedCurve, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::LoadVerificationFailed);
    }

    #[test]
    fn live_mismatch_curve_skips_load_eval() {
        let p = good_point(1770, 1765);
        let (l, _) = classify_load(CurveVerification::LiveMismatch, Some(&p), 1770, LOAD_CLOCK_TOL_MHZ);
        assert_eq!(l, LoadVerification::NotEvaluated);
    }

    #[test]
    fn find_applied_point_by_label_and_ambiguity() {
        let mut prog = PowerSweepProgress::default();
        prog.brokkrs = Some(good_point(1770, 1765));
        // label → named slot
        let found = find_applied_point(&prog, "Brokkr's Best", 1770);
        assert_eq!(found.map(|p| p.clock_mhz), Some(1770));
        // unknown label, no points → None
        assert!(find_applied_point(&prog, "Mystery", 1770).is_none());
        // ambiguous fallback (two near-target points, no matching slot) → None
        let mut prog2 = PowerSweepProgress::default();
        prog2.points = vec![good_point(1770, 1765), good_point(1771, 1766)];
        assert!(find_applied_point(&prog2, "Mystery", 1770).is_none());
    }

    #[test]
    fn status_derivation_rules() {
        use CurveVerification::*;
        use LoadVerification::*;
        assert_eq!(effective_status(VerifiedCurve, VerifiedUnderLoad), "verified_under_load");
        assert_eq!(effective_status(VerifiedCurve, NotEvaluated), "verified_curve");
        assert_eq!(effective_status(VerifiedCurve, TelemetryInsufficient), "verified_curve");
        assert_eq!(effective_status(LiveMismatch, VerifiedUnderLoad), "live_mismatch");
        assert_eq!(effective_status(LiveMismatch, NotEvaluated), "live_mismatch");
    }
}
