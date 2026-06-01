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

/// Realize a core V/F point: lock the voltage and offset the clock so the GPU
/// runs `point.freq_mhz` at `point.voltage_mv`.
#[cfg(windows)]
pub fn apply_core(point: VfPoint) -> Result<(), String> {
    let base = curve_freq_at(point.voltage_mv).unwrap_or(point.freq_mhz);
    let offset = (point.freq_mhz as i64 - base as i64).clamp(-300, 400) as i32;
    nidavellir_gpu_nvapi::lock_core_voltage_mv(point.voltage_mv)?;
    nidavellir_gpu_nvapi::set_core_offset_mhz(offset)
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
