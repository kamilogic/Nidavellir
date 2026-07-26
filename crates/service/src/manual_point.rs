use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use nidavellir_core::ipc::ManualDiagnosticPointStatus;
use nidavellir_core::safe_loop::{BootFlag, SafeLoopStore, TuningPoint};

pub(crate) type ManualPointStatusSlot = Arc<Mutex<ManualDiagnosticPointStatus>>;

#[derive(Clone)]
pub struct ManualPointHandle {
    status: ManualPointStatusSlot,
}

impl Default for ManualPointHandle {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(ManualDiagnosticPointStatus {
                note: "No manual diagnostic point is applied.".into(),
                ..ManualDiagnosticPointStatus::default()
            })),
        }
    }
}

impl ManualPointHandle {
    pub fn status(&self) -> ManualDiagnosticPointStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| ManualDiagnosticPointStatus {
                note: "Manual point status is unavailable.".into(),
                ..ManualDiagnosticPointStatus::default()
            })
    }

    pub(crate) fn status_slot(&self) -> ManualPointStatusSlot {
        Arc::clone(&self.status)
    }

    pub fn apply(
        &mut self,
        store: &SafeLoopStore,
        target_mhz: u32,
        requested_voltage_mv: u32,
    ) -> Result<ManualDiagnosticPointStatus, String> {
        if self.status().active {
            return Err(
                "A manual diagnostic point is already active; return to stock first".into(),
            );
        }
        if !(300..=4_000).contains(&target_mhz) {
            return Err("Target clock must be between 300 and 4000 MHz".into());
        }
        if !(500..=1_250).contains(&requested_voltage_mv) {
            return Err("Requested voltage must be between 500 and 1250 mV".into());
        }

        let record = store.load_record();
        if record.safe_mode {
            return Err(
                "Safe Mode is active; recover the GPU before applying a manual point".into(),
            );
        }
        if store.is_boot_flag_armed() {
            return Err("Safe Loop recovery is armed; return the GPU to stock before applying a manual point".into());
        }

        let (resolved_voltage_mv, offset_mhz) =
            crate::gpu_undervolt::resolve_manual_diagnostic_point(
                target_mhz,
                requested_voltage_mv,
            )?;

        reset_hardware()?;
        crate::gpu_apply::clear_applied();

        if let Err(error) = apply_resolved_point(
            store,
            target_mhz,
            resolved_voltage_mv,
            offset_mhz,
            "manual_diagnostic_point",
        ) {
            let recovery = reset_and_disarm(store);
            return Err(match recovery {
                Ok(()) => format!("Manual point apply failed ({error}); GPU returned to stock"),
                Err(reset_error) => format!(
                    "Manual point apply failed ({error}); stock recovery also failed ({reset_error})"
                ),
            });
        }

        replace_status(&self.status, ManualDiagnosticPointStatus {
            active: true,
            target_mhz: Some(target_mhz),
            requested_voltage_mv: Some(requested_voltage_mv),
            resolved_voltage_mv: Some(resolved_voltage_mv),
            applied_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_millis() as u64),
            verified: true,
            note: "Temporary point is active with a max-clock ceiling and verified voltage lock. Start Game Trace before launching the workload.".into(),
        })?;
        Ok(self.status())
    }

    pub fn reset(&mut self, store: &SafeLoopStore) -> Result<ManualDiagnosticPointStatus, String> {
        reset_and_disarm(store)?;
        self.mark_reset();
        Ok(self.status())
    }

    pub fn mark_reset(&mut self) {
        let _ = replace_status(&self.status, ManualDiagnosticPointStatus {
            note: "GPU is at stock; no manual diagnostic point is active.".into(),
            ..ManualDiagnosticPointStatus::default()
        });
    }
}

pub(crate) fn replace_status(
    slot: &ManualPointStatusSlot,
    status: ManualDiagnosticPointStatus,
) -> Result<(), String> {
    *slot
        .lock()
        .map_err(|_| "Manual point status lock is poisoned".to_string())? = status;
    Ok(())
}

pub(crate) fn apply_resolved_point(
    store: &SafeLoopStore,
    target_mhz: u32,
    resolved_voltage_mv: u32,
    offset_mhz: i32,
    source: &str,
) -> Result<(), String> {
    let intent = TuningPoint::from_axes([
        ("gpu_freq_mhz", target_mhz as i64),
        ("gpu_vf_bin_mv", resolved_voltage_mv as i64),
        ("gpu_offset_mhz", offset_mhz as i64),
    ]);
    store
        .arm_boot_flag(&BootFlag::new(intent, source))
        .map_err(|error| format!("Manual point: failed to arm Safe Loop before write: {error}"))?;
    crate::gpu_undervolt::apply_anchored_undervolt(target_mhz, resolved_voltage_mv)
}

pub(crate) fn reset_hardware() -> Result<(), String> {
    let clock_error = nidavellir_core::nvml_gpu::reset_core_clock_lock().err();
    let vf_error = nidavellir_gpu_nvapi::reset_all().err();
    if clock_error.is_some() || vf_error.is_some() {
        return Err(format!(
            "clock-cap={}; VF/global={}",
            clock_error.as_deref().unwrap_or("ok"),
            vf_error.as_deref().unwrap_or("ok")
        ));
    }
    Ok(())
}

pub(crate) fn reset_and_disarm(store: &SafeLoopStore) -> Result<(), String> {
    reset_hardware()?;
    crate::gpu_apply::clear_applied();
    store.clear_boot_flag().map_err(|error| {
        format!("GPU reset completed but Safe Loop flag could not be cleared: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_point_starts_inactive() {
        let status = ManualPointHandle::default().status();
        assert!(!status.active);
        assert!(!status.verified);
        assert!(status.target_mhz.is_none());
    }

    #[test]
    fn mark_reset_clears_the_selected_point() {
        let mut handle = ManualPointHandle::default();
        replace_status(
            &handle.status,
            ManualDiagnosticPointStatus {
                active: true,
                target_mhz: Some(1800),
                ..ManualDiagnosticPointStatus::default()
            },
        )
        .unwrap();
        handle.mark_reset();
        assert!(!handle.status().active);
        assert!(handle.status().target_mhz.is_none());
    }
}
