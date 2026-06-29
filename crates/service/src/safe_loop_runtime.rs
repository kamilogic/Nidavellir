//! Service-side Safe Loop runtime: the privileged half of roadmap §4.
//!
//! On startup the service runs [`run_startup_recovery`] *before* anything else
//! touches hardware — it reads the on-disk boot-flag, classifies any post-reboot
//! bugcheck, and decides whether to recede, reapply the last good profile, or
//! drop to Safe Mode. A background thread keeps a liveness heartbeat fresh.
//!
//! All the decision logic lives in `nidavellir_core::safe_loop` (pure + tested);
//! this module only does the OS I/O (event log, threads) and logging.

use std::time::Duration;

use nidavellir_core::ipc::SafeLoopStatus;
use nidavellir_core::safe_loop::{
    self, CrashClass, RecoveryAction, SafeLoopRecord, SafeLoopStore,
};
use tracing::{info, warn};

/// How often the liveness heartbeat is refreshed.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

fn retain_boot_flag_until_reapply(action: &RecoveryAction) -> bool {
    matches!(
        action,
        RecoveryAction::BlacklistAndRecede { .. } | RecoveryAction::EnterSafeMode { .. }
    )
}

/// Run boot-time crash recovery and return the (persisted) updated record.
///
/// The returned [`TuningPoint`] is what *should* be (re)applied to hardware.
/// Actual application is wired in once tuning axes land (v0.3+); for now we log
/// the intent so the safety scaffold is observable end to end.
pub fn run_startup_recovery(store: &SafeLoopStore) -> SafeLoopRecord {
    let mut record = store.load_record();
    let mut boot_flag = store.read_boot_flag();

    // A graceful service stop (clean OS shutdown/restart, or an explicit stop) writes a one-shot
    // marker. If a boot-flag was still armed at that moment, the apply/forge it guarded was
    // interrupted by a *user-initiated restart*, not a crash — so disarm it without counting a
    // crash. The marker is always consumed here so it can never mask a later, genuine crash.
    let clean_shutdown = store.is_clean_shutdown_present();
    if let Err(e) = store.clear_clean_shutdown() {
        warn!("Safe Loop: failed to clear clean-shutdown marker: {e}");
    }
    if boot_flag.is_some() && clean_shutdown {
        info!(
            "Safe Loop: boot-flag was armed but the previous service stop was graceful — \
             treating as a clean interruption, not a crash"
        );
        if let Err(e) = store.clear_boot_flag() {
            warn!("Safe Loop: failed to disarm boot-flag after a clean shutdown: {e}");
        }
        boot_flag = None;
    }

    let bugcheck = if boot_flag.is_some() {
        read_last_bugcheck_class()
    } else {
        CrashClass::Unknown
    };

    let action = safe_loop::decide_recovery(boot_flag.as_ref(), bugcheck, &record);
    let target = safe_loop::apply_recovery(&mut record, &action);

    match &action {
        RecoveryAction::Idle => {
            info!("Safe Loop: clean boot, nothing to restore");
        }
        RecoveryAction::ApplyLastValidated { point } => {
            info!("Safe Loop: clean boot, reapplying last validated profile {point:?}");
        }
        RecoveryAction::BlacklistAndRecede {
            crashed,
            recede_to,
            class,
        } => {
            warn!(
                "Safe Loop: boot-flag was ARMED — last apply {crashed:?} crashed ({class:?}). \
                 Blacklisting region and receding to {recede_to:?} \
                 (consecutive crashes: {})",
                record.consecutive_crashes
            );
        }
        RecoveryAction::EnterSafeMode { .. } => {
            warn!(
                "Safe Loop: {} consecutive crashes — entering SAFE MODE (stock profile, hands off)",
                record.consecutive_crashes
            );
        }
        RecoveryAction::RemainSafeMode { .. } => {
            info!(
                "Safe Loop: clean boot while in Safe Mode — staying hands off (no new crash counted, \
                 {} on record). Use Reset all to release.",
                record.consecutive_crashes
            );
        }
    }

    let _ = target; // application is a v0.3+ concern; intent is logged above.

    // Keep an accounted crash flag armed until apply-on-boot has observed it and explicitly skipped
    // the persisted profile. Clearing it here would let the same crashing profile be reapplied later
    // in this startup sequence. Clean/idle actions have no crash profile to suppress.
    let retain_until_reapply = retain_boot_flag_until_reapply(&action);
    if !retain_until_reapply {
        if let Err(e) = store.clear_boot_flag() {
            warn!("Safe Loop: failed to clear boot-flag: {e}");
        }
    }
    if let Err(e) = store.save_record(&record) {
        warn!("Safe Loop: failed to persist record: {e}");
    }
    record
}

/// Spawn the liveness heartbeat writer (roadmap §4.3, layer 2).
pub fn spawn_heartbeat(store: SafeLoopStore) {
    std::thread::spawn(move || loop {
        if let Err(e) = store.write_heartbeat() {
            warn!("Safe Loop: heartbeat write failed: {e}");
        }
        std::thread::sleep(HEARTBEAT_INTERVAL);
    });
}

/// Build the read-only status snapshot for the UI.
pub fn status_snapshot(store: &SafeLoopStore) -> SafeLoopStatus {
    let record = store.load_record();
    SafeLoopStatus {
        state: record.state,
        safe_mode: record.safe_mode,
        consecutive_crashes: record.consecutive_crashes,
        crash_threshold: safe_loop::SAFE_MODE_CRASH_THRESHOLD,
        boot_flag_armed: store.is_boot_flag_armed(),
        last_validated: record.last_validated,
        blacklist: record.blacklist,
        recent_crashes: record.crash_log,
    }
}

/// Read the most recent BSOD bugcheck from the Windows System event log and
/// classify it. Returns [`CrashClass::Unknown`] when no event is found.
fn read_last_bugcheck_class() -> CrashClass {
    match read_last_bugcheck_message() {
        Some(msg) => match safe_loop::parse_bugcheck_code(&msg) {
            Some(code) => {
                let class = safe_loop::classify_bugcheck(code);
                info!("Safe Loop: last bugcheck 0x{code:X} → {class:?}");
                class
            }
            None => CrashClass::Unknown,
        },
        None => CrashClass::Unknown,
    }
}

#[cfg(windows)]
fn read_last_bugcheck_message() -> Option<String> {
    // Event 1001 from the WER-SystemErrorReporting provider is the "computer
    // rebooted from a bugcheck" record; its message carries the stop code.
    let ps = "Get-WinEvent -FilterHashtable @{LogName='System'; \
              ProviderName='Microsoft-Windows-WER-SystemErrorReporting'; Id=1001} \
              -MaxEvents 1 -ErrorAction SilentlyContinue | ForEach-Object { $_.Message }";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(not(windows))]
fn read_last_bugcheck_message() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidavellir_core::safe_loop::TuningPoint;

    #[test]
    fn crash_recovery_retains_flag_until_reapply_can_skip_bad_profile() {
        let point = TuningPoint::stock();
        assert!(retain_boot_flag_until_reapply(
            &RecoveryAction::BlacklistAndRecede {
                crashed: point.clone(),
                recede_to: point.clone(),
                class: CrashClass::Unknown,
            }
        ));
        assert!(retain_boot_flag_until_reapply(
            &RecoveryAction::EnterSafeMode {
                stock: point.clone()
            }
        ));
        assert!(!retain_boot_flag_until_reapply(&RecoveryAction::Idle));
        assert!(!retain_boot_flag_until_reapply(
            &RecoveryAction::ApplyLastValidated { point }
        ));
    }
}
