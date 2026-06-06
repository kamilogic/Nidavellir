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

/// The profile currently applied (persisted to disk for apply-on-boot).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedProfile {
    pub label: String,
    pub core: Option<VfPoint>,
    pub mem_offset_mhz: Option<i32>,
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
    let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_apply"));

    if let Some(c) = core {
        apply_core(c)?;
    }
    if let Some(m) = mem_offset_mhz {
        nidavellir_gpu_nvapi::set_mem_offset_mhz(m)?;
    }

    save_applied(&AppliedProfile { label, core, mem_offset_mhz });

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
    clear_applied();
    let _ = store.clear_boot_flag();
    let _ = nidavellir_core::nvml_gpu::reset_core_clock_lock();
    // Zero the modern V/F curve offsets too (reset_all uses the legacy path and
    // won't clear ceiling offsets written via ClkVfPoints).
    if nidavellir_gpu_nvapi::vf_curve_supported() {
        let n = nidavellir_gpu_nvapi::reset_vf_curve();
        info!("VF curve reset: {n} pts zerados");
    }
    nidavellir_gpu_nvapi::reset_all()
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
pub fn reset(_store: &SafeLoopStore) -> Result<(), String> {
    Err("GPU apply is Windows-only".into())
}

/// Re-apply the persisted profile at service startup. Skips if the Safe Loop
/// boot-flag is armed (last apply crashed) or Safe Mode is active.
#[cfg(windows)]
pub fn reapply_on_boot(store: &SafeLoopStore) {
    if store.is_boot_flag_armed() {
        warn!("GPU apply-on-boot: boot-flag armed (prior crash) — not re-applying");
        return;
    }
    if store.load_record().safe_mode {
        warn!("GPU apply-on-boot: Safe Mode active — not re-applying");
        return;
    }
    let Some(ap) = load_applied() else {
        return;
    };
    info!("GPU apply-on-boot: re-applying '{}'", ap.label);
    if let Err(e) = apply_and_persist(ap.label, ap.core, ap.mem_offset_mhz, store) {
        warn!("GPU apply-on-boot failed: {e}");
    }
}

#[cfg(not(windows))]
pub fn reapply_on_boot(_store: &SafeLoopStore) {}

#[cfg(test)]
mod tests {
    use super::choose_ceiling_mv;

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
