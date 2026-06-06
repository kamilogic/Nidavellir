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

/// Per-point plateau-match clock tolerance: ~one boost bin (the sweep's EXPLORE_STEP).
const TOL_MHZ: u32 = 15;

/// Classify the live modern VF curve against the expected flattened plateau.
///
/// `live` = `(index, voltage_mv, freq_mhz)` from `read_vf_curve_modern()` (GetStatus).
/// Expectation: every point with `voltage_mv >= ceiling_mv` is flattened to
/// `target_mhz`. Returns `(state, matched, expected)`. Pure + table-to-table.
fn classify_curve(
    target_mhz: u32,
    ceiling_mv: u32,
    live: &[(usize, u32, u32)],
    tol_mhz: u32,
) -> (CurveVerification, u32, u32) {
    if live.is_empty() {
        return (CurveVerification::VerificationFailed, 0, 0);
    }
    let expected_n = live.iter().filter(|(_, mv, _)| *mv >= ceiling_mv).count() as u32;
    if expected_n == 0 {
        // Ceiling above every bin → can't locate the plateau region to evaluate.
        return (CurveVerification::VerificationFailed, 0, 0);
    }
    let matched = live
        .iter()
        .filter(|(_, mv, _)| *mv >= ceiling_mv)
        .filter(|(_, _, freq)| freq.abs_diff(target_mhz) <= tol_mhz)
        .count() as u32;
    // Require ≥90% of the expected-flattened points to read the target (tolerate the
    // boundary point / sensor noise), and at least one real match.
    let ratio = matched as f32 / expected_n as f32;
    let state = if matched >= 1 && ratio >= 0.9 {
        CurveVerification::VerifiedCurve
    } else {
        CurveVerification::LiveMismatch
    };
    (state, matched, expected_n)
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
    let (state, matched, expected) = classify_curve(core.freq_mhz, ceiling_mv, &live, TOL_MHZ);
    // Offset corroboration (logged only; not gating in Patch A): how many of the
    // expected-flattened points carry a non-zero applied frequency offset.
    let offsets_nonzero = live
        .iter()
        .filter(|(_, mv, _)| *mv >= ceiling_mv)
        .filter_map(|(i, _, _)| nidavellir_gpu_nvapi::vf_get_point_khz(*i))
        .filter(|o| *o != 0)
        .count();
    let message = match state {
        CurveVerification::VerifiedCurve => format!(
            "live curve matches: {matched}/{expected} plateau points at {} MHz",
            core.freq_mhz
        ),
        CurveVerification::LiveMismatch => format!(
            "live curve mismatch: only {matched}/{expected} plateau points at {} MHz",
            core.freq_mhz
        ),
        _ => "verification incomplete".to_string(),
    };
    info!(
        "apply_verify: label={:?} target={} vf_table_mv={} legacy_mv={} ceiling_idx={} \
         matched={}/{} offsets_nonzero={} curve_state={:?} status={:?}",
        label, core.freq_mhz, ceiling_mv, core.voltage_mv, ceiling_idx, matched, expected,
        offsets_nonzero, state, state
    );
    make_status(
        state,
        label,
        Some(core.freq_mhz),
        Some(ceiling_mv),
        Some(core.voltage_mv),
        Some(matched),
        Some(expected),
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

    // (index, voltage_mv, freq_mhz) like read_vf_curve_modern(). A flattened curve:
    // every point at/above `ceiling_mv` reads the `target`.
    fn flattened(target: u32, ceiling_mv: u32) -> Vec<(usize, u32, u32)> {
        vec![
            (0, 800, 1500),
            (1, 837, 1650),
            (2, ceiling_mv, target),
            (3, 900, target),
            (4, 1062, target),
        ]
    }

    #[test]
    fn exact_match_is_verified() {
        let live = flattened(1770, 875);
        let (s, m, e) = classify_curve(1770, 875, &live, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerifiedCurve);
        assert_eq!((m, e), (3, 3));
    }

    #[test]
    fn within_one_bin_is_verified() {
        // Plateau points +15 MHz (one boost bin) → still inside tolerance.
        let mut live = flattened(1770, 875);
        for p in live.iter_mut().filter(|(_, mv, _)| *mv >= 875) {
            p.2 = 1785;
        }
        let (s, _, _) = classify_curve(1770, 875, &live, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerifiedCurve);
    }

    #[test]
    fn stock_like_curve_is_live_mismatch() {
        // Rising (stock) freqs above the ceiling, none near target → mismatch.
        let live = vec![(0, 800, 1500), (1, 875, 1850), (2, 950, 1920), (3, 1062, 1980)];
        let (s, m, _) = classify_curve(1770, 875, &live, TOL_MHZ);
        assert_eq!(s, CurveVerification::LiveMismatch);
        assert_eq!(m, 0);
    }

    #[test]
    fn empty_curve_is_verification_failed() {
        let (s, _, _) = classify_curve(1770, 875, &[], TOL_MHZ);
        assert_eq!(s, CurveVerification::VerificationFailed);
    }

    #[test]
    fn ceiling_above_all_points_is_verification_failed() {
        let live = flattened(1770, 875);
        let (s, _, _) = classify_curve(1770, 5000, &live, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerificationFailed);
    }

    #[test]
    fn single_unflattened_plateau_point_is_mismatch() {
        // One expected point that does NOT match must not pass on the boundary rule.
        let live = vec![(0, 800, 1500), (1, 875, 1900)];
        let (s, m, e) = classify_curve(1770, 875, &live, TOL_MHZ);
        assert_eq!((m, e), (0, 1));
        assert_eq!(s, CurveVerification::LiveMismatch);
    }

    #[test]
    fn one_outlier_in_large_plateau_tolerated() {
        // 10 plateau points, one far off → still ≥90% match → Verified.
        let mut live: Vec<(usize, u32, u32)> =
            (0..10usize).map(|i| (i, 875 + i as u32, 1770)).collect();
        live[9].2 = 1900;
        let (s, m, e) = classify_curve(1770, 875, &live, TOL_MHZ);
        assert_eq!((m, e), (9, 10));
        assert_eq!(s, CurveVerification::VerifiedCurve);
    }
}
