//! Apply a chosen GPU profile to the hardware and **persist** it so the Core
//! Service re-applies it on every boot (GPU offsets are volatile). Integrated
//! with the Safe Loop: the boot-flag is armed around an apply, so a profile
//! that crashes the machine is NOT re-applied on the next boot.
//!
//! Windows-only (NVAPI). Elsewhere these are inert.

use std::path::PathBuf;

use nidavellir_core::gpu_sweep::VfPoint;
use nidavellir_core::safe_loop::{default_data_dir, BootFlag, SafeLoopStore, TuningPoint};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// An F2 anchored-undervolt apply descriptor (persisted so apply-on-boot re-derives the same anchored
/// curve from the LIVE VF table). When present on an [`AppliedProfile`], the applied profile is an F2
/// undervolt and `core` is status metadata only — apply routes to [`apply_anchored_undervolt`], not
/// the F1 ceiling.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndervoltApply {
    /// Target clock to hold (anchor raised to this; higher-voltage bins capped down to it).
    pub target_mhz: u32,
    /// The VF-table bin voltage to anchor at (the deterministic apply key, `vf_table_voltage_mv`).
    pub anchor_mv: u32,
}

/// The profile currently applied (persisted to disk for apply-on-boot).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedProfile {
    pub label: String,
    pub core: Option<VfPoint>,
    pub mem_offset_mhz: Option<i32>,
    /// F2 anchored-undervolt descriptor. `Some` ⇒ this profile is an F2 undervolt (apply routes to the
    /// anchored writer; `core` remains status metadata). `#[serde(default)]` ⇒ legacy F1 payloads load
    /// as `None`.
    #[serde(default)]
    pub undervolt: Option<UndervoltApply>,
}

fn applied_path() -> PathBuf {
    default_data_dir().join("gpu_applied.json")
}

pub fn load_applied() -> Option<AppliedProfile> {
    let s = std::fs::read_to_string(applied_path()).ok()?;
    serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
}

fn save_applied(p: &AppliedProfile) {
    let _ = std::fs::create_dir_all(default_data_dir());
    if let Ok(j) = serde_json::to_string_pretty(p) {
        let _ = std::fs::write(applied_path(), j);
    }
}

fn clear_applied() {
    let _ = std::fs::remove_file(applied_path());
}

/// v17 sentinel: the fallback ladder is exhausted (or re-apply failed) — clear the persisted
/// profile so boot comes up stock instead of re-applying a point that failed in real use.
#[cfg(windows)]
pub(crate) fn sentinel_clear_applied() {
    clear_applied();
}

/// Remove all persisted *learning* files for a full "forget everything" reset: the F2 observation
/// frontier and the legacy single-clock knowledge. Best-effort — a missing file is success. Returns
/// human-readable errors for files that existed but could not be removed (empty ⇒ all clear).
///
/// Deliberately does NOT touch `gpu_applied.json`, the boot-flag, `safe_loop.json`, or
/// `forge_state.json`; those are handled by [`reset`] / the caller so each concern stays explicit.
///
/// SAFETY INVARIANT: `condemnation_ledger.jsonl` must NEVER be added here (or to any other reset
/// path). It is the append-only memory of real hard failures; the 2026-07-15 manual reset wiped
/// the 1890@900 Endurance condemnation with `safe_loop.json` and the pair was re-attempted the
/// next day. Only an explicit manual rehabilitation entry may lift a condemnation.
pub fn clear_all_learning() -> Vec<String> {
    let base = default_data_dir();
    let targets = [
        base.join(nidavellir_core::f2_observation::F2_OBSERVATIONS_FILE),
        base.join("gpu_knowledge.json"),
    ];
    let mut errors = Vec::new();
    for path in targets {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    errors
}

#[cfg(windows)]
fn curve_freq_at(voltage_mv: u32) -> Option<u32> {
    let c = nidavellir_gpu_nvapi::read_curve().ok()?;
    c.points
        .iter()
        .filter(|p| p.voltage_mv <= voltage_mv)
        .max_by_key(|p| p.voltage_mv)
        .map(|p| p.freq_mhz)
}

/// Realize a core V/F point by FLATTENING the curve. We do NOT hard-lock the
/// voltage: under a heavy (≈power-cap) game load a hard voltage lock removes the
/// card's power management and TDRs.
///
/// Preferred path (Pascal+ on a modern driver): the **elastic VF ceiling** — set
/// per-point frequency offsets so every curve point at or above `point.voltage_mv`
/// is flattened to `point.freq_mhz`, leaving lower-voltage points free. The GPU
/// keeps full power-management elasticity (it can still drop clocks/voltage on
/// light load) yet never boosts past the validated point — the true Afterburner
/// curve-flatten. Fallback (older driver / no modern curve API): a global clock
/// offset + an NVML max-clock cap (less elastic, but works everywhere).
/// Choose the VF-ceiling threshold for an apply. The profile's `voltage_mv` is a
/// MEASURED dwell value (a sparse sensor max), NOT a deterministic curve point, so
/// we snap it to a real VF-table bin (the lowest table voltage at/above it) — the
/// ceiling must land on an actual curve voltage (see `decisions.md`: voltage split).
/// Returns `(ceiling_mv, legacy_fallback)`; `legacy_fallback` is true only when no
/// bin could be resolved (empty/unknown curve) and the raw measured value is used as
/// a last resort. Pure + testable without hardware.
fn choose_ceiling_mv(curve: &[(usize, u32, u32)], measured_mv: u32) -> (u32, bool) {
    match nidavellir_gpu_nvapi::nearest_vf_bin_at_or_above(curve, measured_mv) {
        Some((_, table_mv)) => (table_mv, false),
        None => (measured_mv, true),
    }
}

#[cfg(windows)]
pub fn apply_core(point: VfPoint) -> Result<(), String> {
    if nidavellir_gpu_nvapi::vf_curve_supported() {
        // Snap the MEASURED voltage to a deterministic VF-table bin and key the
        // ceiling on that — never on the raw measured value. The measured number is
        // kept only as descriptive telemetry on the point.
        let curve = nidavellir_gpu_nvapi::read_vf_curve_modern();
        let (ceiling_mv, legacy) = choose_ceiling_mv(&curve, point.voltage_mv);
        if legacy {
            warn!(
                "voltage_semantics: unable to map measured {} mV to a VF-table bin \
                 (empty/unknown curve); apply uses measured value as legacy ceiling",
                point.voltage_mv
            );
        } else {
            info!(
                "voltage_semantics: using vf_table_voltage_mv={ceiling_mv} \
                 measured_voltage_mv={} target={} MHz",
                point.voltage_mv, point.freq_mhz
            );
        }
        match nidavellir_gpu_nvapi::apply_vf_ceiling(ceiling_mv, point.freq_mhz) {
            Ok(n) => {
                info!(
                    "VF ceiling: {n} pts achatados para {} MHz acima de {} mV (elástico)",
                    point.freq_mhz, ceiling_mv
                );
                return Ok(());
            }
            Err(e) => warn!("VF ceiling falhou ({e}); usando fallback offset+cap"),
        }
    }
    // Fallback: offset the clock up so the GPU reaches freq at the lower voltage,
    // then hard-cap the max clock so it never boosts past the validated point.
    let base = curve_freq_at(point.voltage_mv).unwrap_or(point.freq_mhz);
    let offset = (point.freq_mhz as i64 - base as i64).clamp(-300, 400) as i32;
    nidavellir_gpu_nvapi::set_core_offset_mhz(offset)?;
    if let Err(e) = nidavellir_core::nvml_gpu::lock_core_clock_max_mhz(point.freq_mhz) {
        warn!("core clock cap at {} MHz failed (continuing): {e}", point.freq_mhz);
    }
    Ok(())
}

/// Apply a profile (core point and/or memory offset) and persist it. Arms the
/// Safe Loop boot-flag around the apply; clears it after a short survival window
/// (a crash leaves it armed → not re-applied next boot).
#[cfg(windows)]
pub fn apply_and_persist(
    label: String,
    core: Option<VfPoint>,
    mem_offset_mhz: Option<i32>,
    store: &SafeLoopStore,
) -> Result<(), String> {
    let mut intent = TuningPoint::default();
    if let Some(c) = core {
        intent.axes.insert("gpu_freq_mhz".into(), c.freq_mhz as i64);
        intent.axes.insert("gpu_voltage_mv".into(), c.voltage_mv as i64);
    }
    if let Some(m) = mem_offset_mhz {
        intent.axes.insert("gpu_mem_offset_mhz".into(), m as i64);
    }
    store.arm_boot_flag(&BootFlag::new(intent, "gpu_apply"))
        .map_err(|e| format!("GPU apply: failed to arm Safe Loop before write: {e}"))?;

    if let Some(c) = core {
        apply_core(c)?;
    }
    if let Some(m) = mem_offset_mhz {
        nidavellir_gpu_nvapi::set_mem_offset_mhz(m)?;
    }

    save_applied(&AppliedProfile { label, core, mem_offset_mhz, undervolt: None });

    // Clear the boot-flag after a short survival window on a background thread.
    let store = store.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(8));
        let _ = store.clear_boot_flag();
    });
    Ok(())
}

/// Apply an F2 anchored-undervolt profile (`target_mhz` held at the `anchor_mv` VF bin) and persist it.
/// Mirrors [`apply_and_persist`] but writes the anchored undervolt instead of the F1 ceiling: arms the
/// Safe Loop boot-flag around the write (a crash leaves it armed → not re-applied next boot), writes via
/// the fail-closed [`crate::gpu_undervolt::apply_anchored_undervolt`] (which resets to stock on any
/// non-verified outcome), persists `gpu_applied.json` with the [`UndervoltApply`] descriptor, then clears
/// the boot-flag after a short survival window. Preserves any existing memory offset.
#[cfg(windows)]
pub fn apply_and_persist_undervolt(
    label: String,
    target_mhz: u32,
    anchor_mv: u32,
    mem_offset_mhz: Option<i32>,
    store: &SafeLoopStore,
) -> Result<(), String> {
    let mut intent = TuningPoint::default();
    intent.axes.insert("gpu_freq_mhz".into(), target_mhz as i64);
    intent.axes.insert("gpu_voltage_mv".into(), anchor_mv as i64);
    if let Some(m) = mem_offset_mhz {
        intent.axes.insert("gpu_mem_offset_mhz".into(), m as i64);
    }
    store.arm_boot_flag(&BootFlag::new(intent, "gpu_apply_undervolt"))
        .map_err(|e| format!("F2 apply: failed to arm Safe Loop before write: {e}"))?;

    // Fail-closed: a non-verified write has already reset to stock inside this call, so nothing is left
    // applied. The boot-flag stays armed on the error path (same as the F1 apply) — safe; reset clears it.
    crate::gpu_undervolt::apply_anchored_undervolt(target_mhz, anchor_mv)?;
    if let Some(m) = mem_offset_mhz {
        if let Err(e) = nidavellir_gpu_nvapi::set_mem_offset_mhz(m) {
            crate::gpu_power_sweep::reset_to_stock();
            let reset = nidavellir_gpu_nvapi::reset_all();
            return Err(match reset {
                Ok(()) => format!("F2 apply: memory offset failed ({e}); GPU reset to stock"),
                Err(reset_err) => format!(
                    "F2 apply: memory offset failed ({e}); stock reset also failed ({reset_err})"
                ),
            });
        }
    }

    save_applied(&AppliedProfile {
        label,
        // Existing IPC/UI status shape: expose the deterministic target + anchor while the
        // `undervolt` descriptor remains the authoritative apply-on-boot route.
        core: Some(VfPoint { freq_mhz: target_mhz, voltage_mv: anchor_mv }),
        mem_offset_mhz,
        undervolt: Some(UndervoltApply { target_mhz, anchor_mv }),
    });

    // Clear the boot-flag after a short survival window on a background thread.
    let store = store.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(8));
        let _ = store.clear_boot_flag();
    });
    Ok(())
}

/// Reset the GPU to stock and forget the persisted profile.
#[cfg(windows)]
pub fn reset(store: &SafeLoopStore) -> Result<(), String> {
    let clock_error = nidavellir_core::nvml_gpu::reset_core_clock_lock().err();
    let gpu_error = nidavellir_gpu_nvapi::reset_all().err();
    if clock_error.is_some() || gpu_error.is_some() {
        return Err(format!(
            "GPU reset incomplete: clock-cap={}; VF/global={}",
            clock_error.as_deref().unwrap_or("ok"),
            gpu_error.as_deref().unwrap_or("ok")
        ));
    }
    clear_applied();
    // Release the Safe Loop latch so tuning is allowed again: leave Safe Mode and zero the crash
    // streak, while PRESERVING learning (blacklist, last_validated, crash history). Without this the
    // operator's "Reset all" cannot clear a latched Safe Mode — the reset only ever touched the
    // boot-flag and hardware, never this record, so `safe_mode` was a one-way latch.
    let mut record = store.load_record();
    record.clear_recovery_latch();
    let record_err = store
        .save_record(&record)
        .err()
        .map(|e| format!("GPU reset completed but Safe Loop record could not be cleared: {e}"));
    let flag_err = store
        .clear_boot_flag()
        .err()
        .map(|e| format!("GPU reset completed but Safe Loop flag could not be cleared: {e}"));
    record_err.or(flag_err).map_or(Ok(()), Err)
}

#[cfg(not(windows))]
pub fn apply_and_persist(
    _label: String,
    _core: Option<VfPoint>,
    _mem: Option<i32>,
    _store: &SafeLoopStore,
) -> Result<(), String> {
    Err("GPU apply is Windows-only".into())
}

#[cfg(not(windows))]
pub fn apply_and_persist_undervolt(
    _label: String,
    _target_mhz: u32,
    _anchor_mv: u32,
    _mem: Option<i32>,
    _store: &SafeLoopStore,
) -> Result<(), String> {
    Err("GPU apply is Windows-only".into())
}

#[cfg(not(windows))]
pub fn reset(_store: &SafeLoopStore) -> Result<(), String> {
    Err("GPU apply is Windows-only".into())
}

/// Re-apply the persisted profile at service startup. Skips if the Safe Loop
/// boot-flag is armed (last apply crashed), Safe Mode is active, or an interrupted Forge still
/// requires explicit operator acknowledgement.
#[cfg(windows)]
pub fn reapply_on_boot(store: &SafeLoopStore) {
    // v13 (audit N1): the NVML clock ceiling is driver-resident and would survive a service
    // restart WITHOUT a reboot. Release it unconditionally BEFORE the early-return guards so
    // every service start begins from a known clock-lock state (idempotent; the success branch
    // re-sets it via the apply below).
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    if store.is_boot_flag_armed() {
        warn!("GPU apply-on-boot: boot-flag armed (prior crash) — not re-applying");
        if let Err(e) = store.clear_boot_flag() {
            warn!("GPU apply-on-boot: failed to disarm accounted recovery flag: {e}");
        }
        return;
    }
    let record = store.load_record();
    if record.safe_mode {
        warn!("GPU apply-on-boot: Safe Mode active — not re-applying");
        return;
    }
    if record.pending_forge_incident.is_some() {
        warn!("GPU apply-on-boot: Forge incident requires acknowledgement — staying at stock");
        return;
    }
    let Some(ap) = load_applied() else {
        return;
    };
    info!("GPU apply-on-boot: re-applying '{}'", ap.label);
    let res = match ap.undervolt {
        // F2 undervolt: re-derive + re-write the anchored curve from the LIVE VF table (fail-closed).
        Some(uv) => apply_and_persist_undervolt(
            ap.label,
            uv.target_mhz,
            uv.anchor_mv,
            ap.mem_offset_mhz,
            store,
        ),
        // Legacy F1 flatten-down profile.
        None => apply_and_persist(ap.label, ap.core, ap.mem_offset_mhz, store),
    };
    if let Err(e) = res {
        warn!("GPU apply-on-boot failed: {e}");
    }
}

#[cfg(not(windows))]
pub fn reapply_on_boot(_store: &SafeLoopStore) {}

#[cfg(test)]
mod tests {
    use super::{choose_ceiling_mv, AppliedProfile, UndervoltApply};
    use nidavellir_core::gpu_sweep::VfPoint;

    #[test]
    fn applied_profile_roundtrips_undervolt_descriptor() {
        let p = AppliedProfile {
            label: "Godforge".into(),
            core: Some(VfPoint { freq_mhz: 1800, voltage_mv: 875 }),
            mem_offset_mhz: None,
            undervolt: Some(UndervoltApply { target_mhz: 1800, anchor_mv: 875 }),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: AppliedProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.undervolt, Some(UndervoltApply { target_mhz: 1800, anchor_mv: 875 }));
        assert_eq!(back.core, Some(VfPoint { freq_mhz: 1800, voltage_mv: 875 }));
    }

    #[test]
    fn legacy_applied_profile_json_defaults_undervolt_none() {
        // A profile persisted before Phase 2 has no `undervolt` key → defaults None, so apply-on-boot
        // keeps the legacy F1 flatten path. Backward-compatible.
        let legacy = r#"{"label":"Brokkr's Best","core":{"freq_mhz":1800,"voltage_mv":906},"mem_offset_mhz":null}"#;
        let p: AppliedProfile = serde_json::from_str(legacy).unwrap();
        assert!(p.undervolt.is_none(), "missing key must default to legacy F1 behavior");
        assert!(p.core.is_some());
    }

    // (index, voltage_mv, freq_mhz) — shape of read_vf_curve_modern().
    fn curve() -> Vec<(usize, u32, u32)> {
        vec![(0, 800, 1700), (1, 837, 1750), (2, 850, 1770), (3, 1062, 1900)]
    }

    #[test]
    fn ceiling_prefers_vf_table_bin_over_measured() {
        // Measured 843 (between bins) must snap UP to the real 850 table bin, not 843.
        let (mv, legacy) = choose_ceiling_mv(&curve(), 843);
        assert_eq!(mv, 850);
        assert!(!legacy);
        // An exact-bin measurement stays on its bin.
        assert_eq!(choose_ceiling_mv(&curve(), 837), (837, false));
    }

    #[test]
    fn ceiling_falls_back_to_measured_only_when_no_curve() {
        // No deterministic curve available (legacy/unknown) → use measured, flag legacy.
        let (mv, legacy) = choose_ceiling_mv(&[], 843);
        assert_eq!(mv, 843);
        assert!(legacy);
    }
}
