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

/// Run boot-time crash recovery and return the (persisted) updated record.
///
/// The returned [`TuningPoint`] is what *should* be (re)applied to hardware.
/// Actual application is wired in once tuning axes land (v0.3+); for now we log
/// the intent so the safety scaffold is observable end to end.
pub fn run_startup_recovery(store: &SafeLoopStore) -> SafeLoopRecord {
    let mut record = store.load_record();
    let boot_flag = store.read_boot_flag();

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
    }

    let _ = target; // application is a v0.3+ concern; intent is logged above.

    // The crash has been accounted for in `record`; disarm so the *next* boot
    // starts clean unless a new apply re-arms it.
    if let Err(e) = store.clear_boot_flag() {
        warn!("Safe Loop: failed to clear boot-flag: {e}");
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
