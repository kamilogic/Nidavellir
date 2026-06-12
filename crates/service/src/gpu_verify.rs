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
    /// VF curve point index (from `read_vf_curve_modern`). Diagnostic only.
    index: usize,
    /// VF-table voltage (mV) of this point. Diagnostic only.
    voltage_mv: u32,
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

/// Pure, read-only diagnostic evidence about the live applied curve (Patch 11C).
/// Derived from the SAME per-point evidence `classify_curve` consumes; it NEVER feeds
/// classification. Reveals the offset/plateau shape so we can tell normal GPU-Boost
/// overshoot apart from an offset-value miscalibration — without persisting stock base.
// `pub(crate)` so the future transient-ceiling probe (Phase 2B.2-b) can read the diagnostic.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CurveDiag {
    pub(crate) first_modified_bin: Option<u32>,
    pub(crate) first_modified_mv: Option<u32>,
    pub(crate) modified_bin_count: u32,
    pub(crate) expected_bin_count: u32,
    pub(crate) getstatus_freq_match_count: u32,
    pub(crate) getstatus_plateau_min_mhz: Option<u32>,
    pub(crate) getstatus_plateau_max_mhz: Option<u32>,
    pub(crate) max_target_overshoot_mhz: Option<i32>,
    pub(crate) max_target_undershoot_mhz: Option<i32>,
    pub(crate) first_modified_offset_khz: Option<i32>,
    pub(crate) anchor_offset_khz: Option<i32>,
    pub(crate) highest_bin_offset_khz: Option<i32>,
}

/// Compute the read-only curve diagnostic over the expected (≥ anchor) plateau points.
/// `anchor_idx` is the deterministic ceiling bin index. Pure + unit-testable; no I/O.
fn compute_curve_diag(target_mhz: u32, anchor_idx: usize, expected: &[PointEvidence], tol_mhz: u32) -> CurveDiag {
    let first_modified = expected.iter().find(|e| e.offset_khz.map_or(false, |o| o != 0));
    let plateau_min = expected.iter().map(|e| e.freq_mhz).min();
    let plateau_max = expected.iter().map(|e| e.freq_mhz).max();
    CurveDiag {
        first_modified_bin: first_modified.map(|e| e.index as u32),
        first_modified_mv: first_modified.map(|e| e.voltage_mv),
        modified_bin_count: expected
            .iter()
            .filter(|e| e.offset_khz.map_or(false, |o| o != 0))
            .count() as u32,
        expected_bin_count: expected.len() as u32,
        getstatus_freq_match_count: expected
            .iter()
            .filter(|e| e.freq_mhz.abs_diff(target_mhz) <= tol_mhz)
            .count() as u32,
        getstatus_plateau_min_mhz: plateau_min,
        getstatus_plateau_max_mhz: plateau_max,
        // `Some(0)` when a plateau exists but is flat at/below(above) target; `None` only
        // when there are no plateau points at all.
        max_target_overshoot_mhz: plateau_max.map(|mx| (mx as i32 - target_mhz as i32).max(0)),
        max_target_undershoot_mhz: plateau_min.map(|mn| (target_mhz as i32 - mn as i32).max(0)),
        first_modified_offset_khz: first_modified.and_then(|e| e.offset_khz),
        anchor_offset_khz: expected.iter().find(|e| e.index == anchor_idx).and_then(|e| e.offset_khz),
        highest_bin_offset_khz: expected.iter().max_by_key(|e| e.voltage_mv).and_then(|e| e.offset_khz),
    }
}

/// Narrow stock-equivalent ceiling check (Phase 2B.2-c.1). A flatten whose target equals
/// the stock boost top legitimately needs ZERO offset on bins whose stock base is already
/// at the target — the offset-presence ratio then under-counts and `classify_curve` reports
/// `LiveMismatch` even though the ceiling effect is fully present (first bounded run:
/// target=1935, offsets 20/27, plateau exactly 1935..1935, overshoot 0).
///
/// Accept ONLY when every condition holds (else `(false, 0)` — never a partial rescue):
/// 1. The caller supplied the observed stock/cluster boost top AND the target is AT or BELOW
///    it by at most `tol_mhz` (~one boost bin): `target <= top && top - target <= tol`. This
///    is DIRECTIONAL — a target ABOVE the stock top is an overclock, never stock-equivalent,
///    and is rejected even within tolerance. Below-boost (by > tol) targets never take this path.
/// 2. Every expected bin's offset is readable (an unreadable bin explains nothing).
/// 3. NO bin reads above target — overshoot is rejected even inside `tol_mhz`.
/// 4. Every offset-carrying bin reads within `tol_mhz` below target (plateau AT target).
/// 5. Every zero-offset bin reads EXACTLY at target: offset 0 means GetStatus shows the
///    unmodified stock base, and a correct flatten writes `target - base`, so base==target
///    is the only stock-equivalent explanation. A zero-offset bin even 15 MHz under target
///    is an unexplained skip and rejects the whole set.
///
/// GetStatus idle noise can only cause a false REJECT (fail closed), never a false accept:
/// a silently-failed apply leaves below-top bins at their stock base, which violates 4/5.
/// Returns `(accepted, zero_offset_bins_at_target)`. Pure + testable.
fn is_stock_equivalent_ceiling(
    target_mhz: u32,
    stock_top_mhz: Option<u32>,
    expected: &[PointEvidence],
    tol_mhz: u32,
) -> (bool, u32) {
    let Some(top) = stock_top_mhz else { return (false, 0) };
    // Directional: only AT or just BELOW the stock top (never above — that's an overclock).
    if target_mhz > top || top - target_mhz > tol_mhz || expected.is_empty() {
        return (false, 0);
    }
    let mut stock_bins = 0u32;
    for e in expected {
        let Some(offset) = e.offset_khz else { return (false, 0) };
        if e.freq_mhz > target_mhz || target_mhz - e.freq_mhz > tol_mhz {
            return (false, 0);
        }
        if offset == 0 {
            if e.freq_mhz != target_mhz {
                return (false, 0);
            }
            stock_bins += 1;
        }
    }
    (true, stock_bins)
}

/// Bundled result of evaluating a live VF ceiling: the curve verdict (offset-presence gate,
/// UNCHANGED) plus its 11C diagnostic. Lets the persisted-profile verifier (today) and the
/// future transient-ceiling probe (Phase 2B.2-b) share ONE classification path.
// `pub(crate)` so the future transient-ceiling probe (Phase 2B.2-b) can reuse this path.
#[derive(Debug, Clone)]
pub(crate) struct LiveCeilingEval {
    pub(crate) state: CurveVerification,
    pub(crate) offset_present: u32,
    pub(crate) freq_match: u32,
    pub(crate) expected_n: u32,
    pub(crate) diag: CurveDiag,
    /// Phase 2B.2-c.1: true ONLY when the normal gate said `LiveMismatch` but the strict
    /// stock-equivalent boost-top conditions all hold (see [`is_stock_equivalent_ceiling`]).
    /// Service-internal acceptance for the frontier probe — NOT an IPC state; `state` is
    /// reported unchanged.
    pub(crate) stock_equivalent: bool,
    /// Zero-offset expected bins proven already at target in stock (diagnostic).
    pub(crate) stock_equivalent_bins: u32,
}

/// Pure: run the (unchanged) offset-presence `classify_curve` gate + the 11C diagnostic over
/// an ALREADY-built evidence set. No I/O — this is the unit the Phase 2B.2-a tests target.
/// `stock_top_mhz` (Phase 2B.2-c.1) enables the narrow stock-equivalent re-evaluation of a
/// `LiveMismatch` for boost-top targets; pass `None` (the persisted-profile verifier does)
/// to keep classification byte-identical to the pre-c.1 behavior.
fn eval_ceiling_evidence(
    target_mhz: u32,
    anchor_idx: usize,
    expected: &[PointEvidence],
    tol_mhz: u32,
    stock_top_mhz: Option<u32>,
) -> LiveCeilingEval {
    let (state, offset_present, freq_match, expected_n) =
        classify_curve(target_mhz, expected, tol_mhz);
    let diag = compute_curve_diag(target_mhz, anchor_idx, expected, tol_mhz);
    // Narrow stock-equivalent path: ONLY a LiveMismatch is re-evaluated — VerifiedCurve
    // needs no rescue and VerificationFailed (unreadable/empty evidence) must stay failed.
    let (stock_equivalent, stock_equivalent_bins) =
        if state == CurveVerification::LiveMismatch {
            is_stock_equivalent_ceiling(target_mhz, stock_top_mhz, expected, tol_mhz)
        } else {
            (false, 0)
        };
    LiveCeilingEval {
        state,
        offset_present,
        freq_match,
        expected_n,
        diag,
        stock_equivalent,
        stock_equivalent_bins,
    }
}

/// Read-only (NVAPI GET-control offset readback): build the live per-point evidence at/above
/// the ceiling bin, then classify it via [`eval_ceiling_evidence`]. Reusable by the
/// persisted-profile verifier (today) and the transient-ceiling probe (Phase 2B.2-b).
/// `stock_top_mhz`: the observed stock/cluster boost top, enabling the narrow
/// stock-equivalent path for boost-top targets (Phase 2B.2-c.1) — the persisted-profile
/// verifier passes `None` (classification unchanged). NEVER writes / applies / stresses.
#[cfg(windows)]
pub(crate) fn classify_live_ceiling(
    live: &[(usize, u32, u32)],
    ceiling_idx: usize,
    ceiling_mv: u32,
    target_mhz: u32,
    tol_mhz: u32,
    stock_top_mhz: Option<u32>,
) -> LiveCeilingEval {
    // PRIMARY evidence: the GET-control offset readback per expected point. GetStatus
    // actual freq is kept only as secondary corroboration (see classify_curve docs).
    let expected: Vec<PointEvidence> = live
        .iter()
        .filter(|(_, mv, _)| *mv >= ceiling_mv)
        .map(|(i, mv, freq)| PointEvidence {
            index: *i,
            voltage_mv: *mv,
            offset_khz: nidavellir_gpu_nvapi::vf_get_point_khz(*i),
            freq_mhz: *freq,
        })
        .collect();
    eval_ceiling_evidence(target_mhz, ceiling_idx, &expected, tol_mhz, stock_top_mhz)
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
        // Read-only diagnostic (Patch 11C) — populated only on the windows verify path.
        first_modified_bin: None,
        first_modified_mv: None,
        modified_bin_count: None,
        expected_bin_count: None,
        getstatus_freq_match_count: None,
        getstatus_plateau_min_mhz: None,
        getstatus_plateau_max_mhz: None,
        max_target_overshoot_mhz: None,
        max_target_undershoot_mhz: None,
        first_modified_offset_khz: None,
        anchor_offset_khz: None,
        highest_bin_offset_khz: None,
        live_voltage_mv: None,
        live_clock_mhz: None,
        live_power_w: None,
        live_utilization_pct: None,
        live_temperature_c: None,
        live_power_limit_w: None,
        live_power_capped: None,
        diagnostic_message: None,
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

/// A single read-only live telemetry snapshot (Patch 11C). Telemetry ONLY — captured
/// once at verification time, never a sampling loop and NOT load verification. Any
/// unavailable field stays `None` (never a fake zero).
#[cfg(windows)]
#[derive(Debug, Clone, Default)]
struct LiveSnapshot {
    voltage_mv: Option<u32>,
    clock_mhz: Option<u32>,
    power_w: Option<f32>,
    utilization_pct: Option<f32>,
    temperature_c: Option<f32>,
    power_limit_w: Option<f32>,
    power_capped: Option<bool>,
}

/// Read one read-only live snapshot: NVAPI measured core voltage + the first NVML GPU
/// reading (clock/power/util/temp/cap). No writes, no stress, no sampling loop.
#[cfg(windows)]
fn read_live_snapshot() -> LiveSnapshot {
    let voltage_mv = nidavellir_gpu_nvapi::read_core_voltage_mv();
    let nvml = nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml();
    let g = nvml.first();
    LiveSnapshot {
        voltage_mv,
        clock_mhz: g.and_then(|r| r.core_clock_mhz),
        power_w: g.and_then(|r| r.power_w),
        utilization_pct: g.and_then(|r| r.utilization_pct).map(|u| u as f32),
        temperature_c: g.and_then(|r| r.temperature_c),
        power_limit_w: g.and_then(|r| r.power_limit_w),
        power_capped: g.and_then(|r| r.power_capped),
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
    // Classify the live curve against the deterministic ceiling bin (offset-presence gate)
    // and compute the 11C diagnostic via the shared helper — the same path the future
    // transient-ceiling probe (Phase 2B.2-b) will use.
    // `stock_top_mhz = None`: the persisted-profile verifier never uses the narrow
    // stock-equivalent path — its classification is byte-identical to pre-c.1.
    let LiveCeilingEval { state, offset_present, freq_match, expected_n, diag, .. } =
        classify_live_ceiling(&live, ceiling_idx, ceiling_mv, core.freq_mhz, TOL_MHZ, None);
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

    // ── Read-only live diagnostic (Patch 11C): offset/plateau shape + one telemetry
    //    snapshot. `diag` came from `classify_live_ceiling` (the SAME evidence); it NEVER
    //    affects `state`/classification.
    let snap = read_live_snapshot();
    let diag_msg = match state {
        CurveVerification::VerifiedCurve => format!(
            "Curve offsets resident ({}/{} plateau bins). Live voltage is telemetry, NOT a hard \
             cap (may sit above the {} mV VF anchor). GetStatus plateau {:?}..{:?} MHz vs target \
             {} is diagnostic; live boost may exceed it.",
            diag.modified_bin_count, diag.expected_bin_count, ceiling_mv,
            diag.getstatus_plateau_min_mhz, diag.getstatus_plateau_max_mhz, core.freq_mhz
        ),
        _ => format!(
            "Curve offsets NOT fully resident ({}/{}); see status. Live snapshot is telemetry only.",
            diag.modified_bin_count, diag.expected_bin_count
        ),
    };
    st.first_modified_bin = diag.first_modified_bin;
    st.first_modified_mv = diag.first_modified_mv;
    st.modified_bin_count = Some(diag.modified_bin_count);
    st.expected_bin_count = Some(diag.expected_bin_count);
    st.getstatus_freq_match_count = Some(diag.getstatus_freq_match_count);
    st.getstatus_plateau_min_mhz = diag.getstatus_plateau_min_mhz;
    st.getstatus_plateau_max_mhz = diag.getstatus_plateau_max_mhz;
    st.max_target_overshoot_mhz = diag.max_target_overshoot_mhz;
    st.max_target_undershoot_mhz = diag.max_target_undershoot_mhz;
    st.first_modified_offset_khz = diag.first_modified_offset_khz;
    st.anchor_offset_khz = diag.anchor_offset_khz;
    st.highest_bin_offset_khz = diag.highest_bin_offset_khz;
    st.live_voltage_mv = snap.voltage_mv;
    st.live_clock_mhz = snap.clock_mhz;
    st.live_power_w = snap.power_w;
    st.live_utilization_pct = snap.utilization_pct;
    st.live_temperature_c = snap.temperature_c;
    st.live_power_limit_w = snap.power_limit_w;
    st.live_power_capped = snap.power_capped;
    st.diagnostic_message = Some(diag_msg);
    info!(
        "apply_verify_diag: label={:?} target={} anchor_mv={} first_modified_bin={:?} \
         first_modified_mv={:?} modified_bins={}/{} getstatus_freq_match={}/{} \
         plateau_mhz={:?}..{:?} max_overshoot={:?} max_undershoot={:?} \
         offset_khz[first={:?} anchor={:?} highest={:?}] live[voltage_mv={:?} clock_mhz={:?} \
         power_w={:?} util_pct={:?} temp_c={:?} limit_w={:?} capped={:?}]",
        label, core.freq_mhz, ceiling_mv, diag.first_modified_bin, diag.first_modified_mv,
        diag.modified_bin_count, diag.expected_bin_count, diag.getstatus_freq_match_count,
        diag.expected_bin_count, diag.getstatus_plateau_min_mhz, diag.getstatus_plateau_max_mhz,
        diag.max_target_overshoot_mhz, diag.max_target_undershoot_mhz,
        diag.first_modified_offset_khz, diag.anchor_offset_khz, diag.highest_bin_offset_khz,
        snap.voltage_mv, snap.clock_mhz, snap.power_w, snap.utilization_pct,
        snap.temperature_c, snap.power_limit_w, snap.power_capped
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
        first_modified_bin: None,
        first_modified_mv: None,
        modified_bin_count: None,
        expected_bin_count: None,
        getstatus_freq_match_count: None,
        getstatus_plateau_min_mhz: None,
        getstatus_plateau_max_mhz: None,
        max_target_overshoot_mhz: None,
        max_target_undershoot_mhz: None,
        first_modified_offset_khz: None,
        anchor_offset_khz: None,
        highest_bin_offset_khz: None,
        live_voltage_mv: None,
        live_clock_mhz: None,
        live_power_w: None,
        live_utilization_pct: None,
        live_temperature_c: None,
        live_power_limit_w: None,
        live_power_capped: None,
        diagnostic_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(offset_khz: Option<i32>, freq_mhz: u32) -> PointEvidence {
        PointEvidence { index: 0, voltage_mv: 900, offset_khz, freq_mhz }
    }

    fn evx(index: usize, voltage_mv: u32, offset_khz: Option<i32>, freq_mhz: u32) -> PointEvidence {
        PointEvidence { index, voltage_mv, offset_khz, freq_mhz }
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

    // ── Patch 11C: read-only curve diagnostic (pure) ─────────────────────────────
    #[test]
    fn diag_no_offsets_reports_none_modified() {
        // Offsets readable but all zero → nothing modified; safe Nones, no panic.
        let expected: Vec<PointEvidence> =
            (0..5).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(0), 1900)).collect();
        let d = compute_curve_diag(1785, 60, &expected, TOL_MHZ);
        assert_eq!(d.modified_bin_count, 0);
        assert_eq!(d.expected_bin_count, 5);
        assert_eq!(d.first_modified_bin, None);
        assert_eq!(d.first_modified_mv, None);
        assert_eq!(d.first_modified_offset_khz, None);
    }

    #[test]
    fn diag_first_offset_at_anchor() {
        let expected = vec![
            evx(62, 850, Some(-90_000), 1785),
            evx(63, 875, Some(-120_000), 1785),
            evx(64, 1062, Some(-160_000), 1785),
        ];
        let d = compute_curve_diag(1785, 62, &expected, TOL_MHZ);
        assert_eq!(d.first_modified_bin, Some(62));
        assert_eq!(d.first_modified_mv, Some(850));
        assert_eq!(d.first_modified_offset_khz, Some(-90_000));
        assert_eq!(d.anchor_offset_khz, Some(-90_000));
        assert_eq!(d.highest_bin_offset_khz, Some(-160_000)); // highest voltage = 1062 mV
        assert_eq!((d.modified_bin_count, d.expected_bin_count), (3, 3));
    }

    #[test]
    fn diag_sparse_offsets_counts_correctly() {
        let expected = vec![
            evx(62, 850, Some(0), 1900),
            evx(63, 875, Some(-100_000), 1785),
            evx(64, 950, Some(0), 1900),
            evx(65, 1062, Some(-150_000), 1785),
        ];
        let d = compute_curve_diag(1785, 62, &expected, TOL_MHZ);
        assert_eq!(d.modified_bin_count, 2);
        assert_eq!(d.first_modified_bin, Some(63));
        assert_eq!(d.anchor_offset_khz, Some(0)); // the anchor bin (62) itself carries zero
    }

    #[test]
    fn diag_all_offsets_modified() {
        let expected: Vec<PointEvidence> =
            (0..6).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(-100_000), 1785)).collect();
        let d = compute_curve_diag(1785, 60, &expected, TOL_MHZ);
        assert_eq!((d.modified_bin_count, d.expected_bin_count), (6, 6));
        assert_eq!(d.first_modified_bin, Some(60));
    }

    #[test]
    fn diag_plateau_exact_target_zero_over_undershoot() {
        let expected: Vec<PointEvidence> =
            (0..4).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(-90_000), 1785)).collect();
        let d = compute_curve_diag(1785, 60, &expected, TOL_MHZ);
        assert_eq!(d.getstatus_plateau_min_mhz, Some(1785));
        assert_eq!(d.getstatus_plateau_max_mhz, Some(1785));
        assert_eq!(d.max_target_overshoot_mhz, Some(0));
        assert_eq!(d.max_target_undershoot_mhz, Some(0));
        assert_eq!(d.getstatus_freq_match_count, 4);
    }

    #[test]
    fn diag_overshoot_detected() {
        // Plateau reads up to 1830 while target is 1785 → 45 MHz overshoot (the suspect).
        let expected = vec![
            evx(60, 850, Some(-80_000), 1815),
            evx(61, 875, Some(-80_000), 1830),
            evx(62, 1062, Some(-120_000), 1830),
        ];
        let d = compute_curve_diag(1785, 60, &expected, TOL_MHZ);
        assert_eq!(d.getstatus_plateau_max_mhz, Some(1830));
        assert_eq!(d.max_target_overshoot_mhz, Some(45));
        assert_eq!(d.max_target_undershoot_mhz, Some(0));
    }

    #[test]
    fn diag_undershoot_detected() {
        let expected = vec![
            evx(60, 850, Some(-90_000), 1740),
            evx(61, 875, Some(-90_000), 1785),
        ];
        let d = compute_curve_diag(1785, 60, &expected, TOL_MHZ);
        assert_eq!(d.getstatus_plateau_min_mhz, Some(1740));
        assert_eq!(d.max_target_undershoot_mhz, Some(45));
        assert_eq!(d.max_target_overshoot_mhz, Some(0));
    }

    #[test]
    fn diag_empty_is_safe() {
        let d = compute_curve_diag(1785, 60, &[], TOL_MHZ);
        assert_eq!(d, CurveDiag::default());
        assert_eq!((d.modified_bin_count, d.expected_bin_count), (0, 0));
        assert_eq!(d.max_target_overshoot_mhz, None);
    }

    #[test]
    fn live_voltage_above_anchor_does_not_downgrade_curve() {
        // The diagnostic + live snapshot are independent of classification: a curve whose
        // plateau points carry the flatten offset stays VerifiedCurve regardless of any
        // (high) measured voltage — measured voltage is never an input to classify_curve.
        let expected: Vec<PointEvidence> = (0..10).map(|_| ev(Some(-120_000), 1500)).collect();
        let (s, _, _, _) = classify_curve(1770, &expected, TOL_MHZ);
        assert_eq!(s, CurveVerification::VerifiedCurve);
    }

    // ── Phase 2B.2-a: shared live-ceiling evaluation (pure) ──────────────────────
    #[test]
    fn eval_ceiling_verified_bundles_state_and_diag() {
        // All plateau points carry the flatten offset → VerifiedCurve; diag reflects it.
        let expected: Vec<PointEvidence> =
            (0..10).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(-120_000), 1785)).collect();
        let e = eval_ceiling_evidence(1785, 60, &expected, TOL_MHZ, None);
        assert_eq!(e.state, CurveVerification::VerifiedCurve);
        assert_eq!((e.offset_present, e.expected_n), (10, 10));
        assert_eq!(e.diag.modified_bin_count, 10);
        assert_eq!(e.diag.first_modified_bin, Some(60));
    }

    #[test]
    fn eval_ceiling_mismatch_when_offsets_absent() {
        // Offsets readable but all zero (reset / never flattened) → LiveMismatch.
        let expected: Vec<PointEvidence> =
            (0..10).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(0), 1900)).collect();
        let e = eval_ceiling_evidence(1785, 60, &expected, TOL_MHZ, None);
        assert_eq!(e.state, CurveVerification::LiveMismatch);
        assert_eq!(e.offset_present, 0);
    }

    #[test]
    fn eval_ceiling_failed_when_unreadable_or_empty() {
        // Unreadable offsets → VerificationFailed (safer than asserting a mismatch).
        let unreadable: Vec<PointEvidence> =
            (0..5).map(|i| evx(60 + i, 850, None, 1785)).collect();
        assert_eq!(
            eval_ceiling_evidence(1785, 60, &unreadable, TOL_MHZ, None).state,
            CurveVerification::VerificationFailed
        );
        // Empty plateau set → VerificationFailed.
        assert_eq!(
            eval_ceiling_evidence(1785, 60, &[], TOL_MHZ, None).state,
            CurveVerification::VerificationFailed
        );
    }

    #[test]
    fn eval_ceiling_plateau_spread_is_diagnostic_only() {
        // Plateau overshoots target (1830 vs 1785) but offsets ARE present → still Verified;
        // the spread surfaces in `diag`, never in the verdict (offset-presence is the gate).
        let expected: Vec<PointEvidence> =
            (0..10).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(-100_000), 1830)).collect();
        let e = eval_ceiling_evidence(1785, 60, &expected, TOL_MHZ, None);
        assert_eq!(e.state, CurveVerification::VerifiedCurve);
        assert_eq!(e.diag.max_target_overshoot_mhz, Some(45));
    }

    #[test]
    fn eval_ceiling_voltage_does_not_affect_classification() {
        // Same offsets/freqs, different VF-table voltages → identical verdict (measured/table
        // voltage is never a classification input).
        let low: Vec<PointEvidence> =
            (0..6).map(|i| evx(60 + i, 850, Some(-90_000), 1785)).collect();
        let high: Vec<PointEvidence> =
            (0..6).map(|i| evx(60 + i, 1062, Some(-90_000), 1785)).collect();
        assert_eq!(
            eval_ceiling_evidence(1785, 60, &low, TOL_MHZ, None).state,
            eval_ceiling_evidence(1785, 60, &high, TOL_MHZ, None).state
        );
    }

    // ── Phase 2B.2-c.1: stock-equivalent boost-top ceiling (pure) ────────────────
    /// The first bounded supervised run (2026-06-11): target=1935 (= stock boost top),
    /// offsets 20/27 present, GetStatus plateau exactly 1935..1935, overshoot 0 — the
    /// 7 zero-offset bins were top bins already at 1935 in stock.
    fn boost_top_evidence() -> Vec<PointEvidence> {
        let mut v: Vec<PointEvidence> = (0..20)
            .map(|i| evx(38 + i, 1075 + i as u32 * 3, Some(15_000 + i as i32 * 1_000), 1935))
            .collect();
        v.extend((0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), 1935)));
        v
    }

    #[test]
    fn stock_equivalent_boost_top_is_accepted() {
        // Reproduces the rejected first-run probe: normal gate stays LiveMismatch
        // (20/27 < 90%) but every missing offset is explained by a stock-at-target bin.
        let e = eval_ceiling_evidence(1935, 38, &boost_top_evidence(), TOL_MHZ, Some(1935));
        assert_eq!(e.state, CurveVerification::LiveMismatch);
        assert!(e.stock_equivalent);
        assert_eq!(e.stock_equivalent_bins, 7);
        assert_eq!((e.offset_present, e.expected_n), (20, 27));
    }

    #[test]
    fn stock_equivalent_rejected_when_plateau_misses_target() {
        // Zero-offset bins read 1905, not the 1935 target → the missing offsets are
        // UNexplained (a correct flatten would have written +30 MHz there) → mismatch.
        let mut v: Vec<PointEvidence> =
            (0..20).map(|i| evx(38 + i, 1075 + i as u32 * 3, Some(20_000), 1935)).collect();
        v.extend((0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), 1905)));
        let e = eval_ceiling_evidence(1935, 38, &v, TOL_MHZ, Some(1935));
        assert_eq!(e.state, CurveVerification::LiveMismatch);
        assert!(!e.stock_equivalent);
    }

    #[test]
    fn stock_equivalent_rejected_on_any_overshoot() {
        // One bin reads ABOVE target — even within the GetStatus tolerance → rejected.
        let mut v = boost_top_evidence();
        v[5].freq_mhz = 1945; // +10 MHz overshoot, < TOL_MHZ — still not acceptable
        let e = eval_ceiling_evidence(1935, 38, &v, TOL_MHZ, Some(1935));
        assert_eq!(e.state, CurveVerification::LiveMismatch);
        assert!(!e.stock_equivalent);
    }

    #[test]
    fn stock_equivalent_rejected_below_boost_top() {
        // Same weak-offset evidence shape, but the target sits a clock step below the
        // stock top → the narrow path must NOT apply (the normal gate governs).
        let mut v: Vec<PointEvidence> =
            (0..20).map(|i| evx(38 + i, 1075 + i as u32 * 3, Some(20_000), 1875)).collect();
        v.extend((0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), 1875)));
        let e = eval_ceiling_evidence(1875, 38, &v, TOL_MHZ, Some(1935));
        assert_eq!(e.state, CurveVerification::LiveMismatch);
        assert!(!e.stock_equivalent);
    }

    #[test]
    fn stock_equivalent_rejected_when_zero_offset_bin_not_exactly_at_target() {
        // A zero-offset bin at 1920 (within GetStatus tolerance of 1935) is still NOT
        // explained: a correct flatten would have written a +15 MHz offset there.
        let mut v = boost_top_evidence();
        v[26].freq_mhz = 1920; // a zero-offset bin: within tol, but not exactly at target
        let e = eval_ceiling_evidence(1935, 38, &v, TOL_MHZ, Some(1935));
        assert!(!e.stock_equivalent);
    }

    #[test]
    fn stock_equivalent_requires_stock_top_and_readable_offsets() {
        // No stock-top info (the persisted-profile verifier path) → never applies.
        let e = eval_ceiling_evidence(1935, 38, &boost_top_evidence(), TOL_MHZ, None);
        assert!(!e.stock_equivalent);
        // An unreadable offset among the expected bins → never applies (and an
        // all-unreadable set stays VerificationFailed, which is never re-evaluated).
        let mut v = boost_top_evidence();
        v[3].offset_khz = None;
        let e2 = eval_ceiling_evidence(1935, 38, &v, TOL_MHZ, Some(1935));
        assert_eq!(e2.state, CurveVerification::LiveMismatch);
        assert!(!e2.stock_equivalent);
    }

    #[test]
    fn stock_equivalent_flag_never_set_on_normal_verified() {
        // ≥90% offsets present → normal VerifiedCurve; the narrow path is not consulted
        // even when the target equals the stock top.
        let v: Vec<PointEvidence> =
            (0..10).map(|i| evx(60 + i, 850 + i as u32 * 10, Some(-120_000), 1785)).collect();
        let e = eval_ceiling_evidence(1785, 60, &v, TOL_MHZ, Some(1785));
        assert_eq!(e.state, CurveVerification::VerifiedCurve);
        assert!(!e.stock_equivalent);
        assert_eq!(e.stock_equivalent_bins, 0);
    }

    #[test]
    fn stock_equivalent_fully_degenerate_all_zero_at_target_accepts() {
        // The maximal-departure case: the ceiling bin snapped high into the stock boost-top
        // plateau, so EVERY expected bin is already at target in stock → a correct flatten
        // writes target-base = 0 on all of them. offset_present == 0 → classify_curve says
        // LiveMismatch, but all bins read EXACTLY at target with readable (zero) offsets, so
        // the stock-equivalent path accepts. A failed apply here is hardware-IDENTICAL to a
        // correct flatten (every bin is at target either way), so dwelling is no riskier than
        // stock. Pins this as INTENDED behavior.
        let v: Vec<PointEvidence> =
            (0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), 1935)).collect();
        let e = eval_ceiling_evidence(1935, 58, &v, TOL_MHZ, Some(1935));
        assert_eq!(e.state, CurveVerification::LiveMismatch); // normal gate: 0/7 offsets
        assert_eq!((e.offset_present, e.expected_n), (0, 7));
        assert!(e.stock_equivalent);
        assert_eq!(e.stock_equivalent_bins, 7);
    }

    #[test]
    fn stock_equivalent_tol_boundary_offset_bin_exactly_15_below_accepts() {
        // An offset-carrying bin exactly tol (15 MHz) below target is within plateau
        // tolerance → accepted; 16 below → rejected (condition 4 boundary, accept side).
        let at_tol: Vec<PointEvidence> = {
            let mut v: Vec<PointEvidence> =
                (0..19).map(|i| evx(38 + i, 1075 + i as u32 * 3, Some(20_000), 1935)).collect();
            v.push(evx(57, 1130, Some(18_000), 1920)); // exactly 15 below target
            v.extend((0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), 1935)));
            v
        };
        let e = eval_ceiling_evidence(1935, 38, &at_tol, TOL_MHZ, Some(1935));
        assert!(e.stock_equivalent, "offset bin exactly 15 MHz below target must accept");

        let past_tol: Vec<PointEvidence> = {
            let mut v = at_tol.clone();
            v[19].freq_mhz = 1919; // 16 below target → out of tolerance
            v
        };
        let e2 = eval_ceiling_evidence(1935, 38, &past_tol, TOL_MHZ, Some(1935));
        assert!(!e2.stock_equivalent, "offset bin 16 MHz below target must reject");
    }

    #[test]
    fn stock_equivalent_target_tol_boundary_15_below_top_accepts_16_rejects() {
        // Condition 1 (accept side): a target exactly tol below the stock top enters the
        // path; 16 below does not. Evidence here is fully valid (all bins at target) so the
        // only variable under test is the target-vs-top gate.
        let evidence = |target: u32| -> Vec<PointEvidence> {
            (0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), target)).collect()
        };
        // 1920 == 1935 - 15 → within tol → accepts.
        let e = eval_ceiling_evidence(1920, 58, &evidence(1920), TOL_MHZ, Some(1935));
        assert!(e.stock_equivalent, "target 15 MHz below stock top must accept");
        // 1919 == 1935 - 16 → out of tol → rejects (normal gate governs).
        let e2 = eval_ceiling_evidence(1919, 58, &evidence(1919), TOL_MHZ, Some(1935));
        assert!(!e2.stock_equivalent, "target 16 MHz below stock top must reject");
    }

    #[test]
    fn stock_equivalent_directional_rejects_target_above_stock_top() {
        // Directional hardening: a target ABOVE the stock top is an overclock, never
        // stock-equivalent — rejected even 1 MHz over and even within tol. The frontier
        // never emits such a target (first-run regime is PowerLimited), but the verifier
        // must not depend on that.
        let evidence = |target: u32| -> Vec<PointEvidence> {
            (0..7).map(|i| evx(58 + i, 1136 + i as u32 * 2, Some(0), target)).collect()
        };
        // 1 MHz above the stock top → rejected.
        let e1 = eval_ceiling_evidence(1936, 58, &evidence(1936), TOL_MHZ, Some(1935));
        assert!(!e1.stock_equivalent, "target 1 MHz above stock top must reject");
        // Well above but still within the old symmetric abs_diff window (would have passed
        // before the directional fix) → still rejected now.
        let e2 = eval_ceiling_evidence(1945, 58, &evidence(1945), TOL_MHZ, Some(1935));
        assert!(!e2.stock_equivalent, "target above stock top within tol must still reject");
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
