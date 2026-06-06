//! Read-only verification of whether the live modern VF curve matches the applied
//! Nidavellir profile (Patch A: **curve-only**). It NEVER writes, reapplies, or runs
//! stress — it reads the modern ClkVfPoints curve (GetStatus + GET-control offsets)
//! and classifies.
//!
//! Comparison is **table-to-table** against the deterministic VF-table ceiling bin,
//! re-derived the SAME way `gpu_apply` derives the apply key (snap the measured/legacy
//! voltage to a real bin via `nearest_vf_bin_at_or_above`) — never the measured value.
//! Effective telemetry under load and workload context are out of scope here (later
//! patches B/C).

use nidavellir_core::ipc::{ApplyVerificationStatus, CurveVerification};
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
    info!(
        "apply_verify: label={:?} target={} vf_table_mv={} legacy_mv={} ceiling_idx={} \
         offset_match={}/{} getstatus_freq_match={}/{} curve_state={:?} status={:?}",
        label, core.freq_mhz, ceiling_mv, core.voltage_mv, ceiling_idx, offset_present,
        expected_n, freq_match, expected_n, state, state
    );
    make_status(
        state,
        label,
        Some(core.freq_mhz),
        Some(ceiling_mv),
        Some(core.voltage_mv),
        Some(offset_present),
        Some(expected_n),
        message,
    )
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
}
