//! The Safe Loop — Nidavellir's "parachute before the jump" (roadmap §4).
//!
//! A reboot-surviving state machine that makes aggressive tuning recoverable:
//! before every apply it arms an on-disk boot-flag; a clean validation clears
//! it. On the next boot the service reads the flag first — if it is still armed,
//! the last apply must have crashed the machine, so we blacklist the region
//! around that point, recede to the last known-good profile, and after three
//! consecutive crashes fall back to Safe Mode (stock profile, hands off).
//!
//! This module is deliberately split into pure logic (state machine, bugcheck
//! classification, blacklist/recovery decisions — all unit-tested) and a thin
//! filesystem-backed [`SafeLoopStore`] for persistence that survives reboots.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Three consecutive crashes trips Safe Mode.
pub const SAFE_MODE_CRASH_THRESHOLD: u32 = 3;

/// Default blacklist radius (in per-axis steps) carved out around a crash point.
pub const DEFAULT_BLACKLIST_RADIUS: i64 = 1;

/// `0x101 CLOCK_WATCHDOG_TIMEOUT` — a core stopped responding; classic OC crash.
pub const BUGCHECK_CLOCK_WATCHDOG: u64 = 0x101;
/// `0x124 WHEA_UNCORRECTABLE_ERROR` — machine-check; classic undervolt/OC crash.
pub const BUGCHECK_WHEA_UNCORRECTABLE: u64 = 0x124;
/// `0x133 DPC_WATCHDOG_VIOLATION` — also commonly OC/driver instability.
pub const BUGCHECK_DPC_WATCHDOG: u64 = 0x133;

/// A point in tuning space: axis name → integer setting (mV offset, MHz, ratio…).
///
/// An empty map (or all-zero axes) is the *stock* point — the recovery target
/// that is always safe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TuningPoint {
    pub axes: BTreeMap<String, i64>,
}

impl TuningPoint {
    /// The always-safe stock point (no offsets applied).
    pub fn stock() -> Self {
        Self::default()
    }

    pub fn from_axes<I, S>(axes: I) -> Self
    where
        I: IntoIterator<Item = (S, i64)>,
        S: Into<String>,
    {
        Self {
            axes: axes.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// True when every axis is at its neutral (0) setting.
    pub fn is_stock(&self) -> bool {
        self.axes.values().all(|&v| v == 0)
    }

    /// Chebyshev (L∞) distance, treating any axis missing from either side as 0.
    /// Used to decide whether a candidate falls inside a blacklisted region.
    pub fn chebyshev(&self, other: &Self) -> i64 {
        let mut max = 0;
        for key in self.axes.keys().chain(other.axes.keys()) {
            let a = self.axes.get(key).copied().unwrap_or(0);
            let b = other.axes.get(key).copied().unwrap_or(0);
            max = max.max((a - b).abs());
        }
        max
    }
}

/// A carved-out region of tuning space known to be unstable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistRegion {
    pub center: TuningPoint,
    pub radius: i64,
}

impl BlacklistRegion {
    pub fn around(center: TuningPoint, radius: i64) -> Self {
        Self { center, radius }
    }

    /// A point is blacklisted when it lies within `radius` of the crash center.
    pub fn contains(&self, point: &TuningPoint) -> bool {
        self.center.chebyshev(point) <= self.radius
    }
}

/// The live phase of the loop (roadmap §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLoopState {
    Idle,
    Probing,
    Applying,
    Dwell,
    Validated,
    Unstable,
    SafeMode,
}

/// Events that drive the state machine forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeLoopEvent {
    /// Begin exploring a new candidate.
    StartProbe,
    /// Boot-flag armed, point pushed to hardware.
    Applied,
    /// Begin the dwell window with the stressor running.
    EnterDwell,
    /// Dwell completed cleanly — point is good.
    DwellPassed,
    /// WHEA correctable delta during dwell — revert without crashing.
    SoftFail,
    /// A hard crash was detected (post-reboot, boot-flag was armed).
    HardCrash,
    /// Crash threshold reached — drop to stock and stop.
    TripSafeMode,
    /// Operator/recovery reset back to idle.
    Reset,
}

/// Pure state-transition function. Unknown transitions leave the state intact,
/// so a stray event can never push the loop into an unsafe phase.
pub fn transition(state: SafeLoopState, event: SafeLoopEvent) -> SafeLoopState {
    use SafeLoopEvent as E;
    use SafeLoopState as S;
    match (state, event) {
        (_, E::TripSafeMode) => S::SafeMode,
        (_, E::Reset) => S::Idle,
        (_, E::HardCrash) => S::Unstable,
        (S::Idle, E::StartProbe) => S::Probing,
        (S::Probing, E::Applied) => S::Applying,
        (S::Applying, E::EnterDwell) => S::Dwell,
        (S::Dwell, E::DwellPassed) => S::Validated,
        (S::Dwell, E::SoftFail) => S::Unstable,
        // After a result, the next probe restarts the cycle.
        (S::Validated, E::StartProbe) | (S::Unstable, E::StartProbe) => S::Probing,
        (other, _) => other,
    }
}

/// How a bugcheck code maps to a stability verdict (roadmap §4.3, layer 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashClass {
    /// Bugcheck strongly associated with overclock/undervolt instability.
    OcInstability,
    /// A real BSOD, but not a typical OC signature (driver, disk, etc.).
    Unrelated,
    /// No bugcheck code available (e.g. a freeze with no dump).
    Unknown,
}

/// Classify a Windows bugcheck (stop) code.
pub fn classify_bugcheck(code: u64) -> CrashClass {
    match code {
        0 => CrashClass::Unknown,
        BUGCHECK_CLOCK_WATCHDOG | BUGCHECK_WHEA_UNCORRECTABLE | BUGCHECK_DPC_WATCHDOG => {
            CrashClass::OcInstability
        }
        _ => CrashClass::Unrelated,
    }
}

/// Extract the bugcheck code from a Windows "BugCheck" event-log message.
///
/// The WER message reads e.g. `The computer has rebooted from a bugcheck.
/// The bugcheck was: 0x00000124 (0x..., ...)`. We take the first `0x` hex token,
/// which is always the stop code.
pub fn parse_bugcheck_code(text: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("0x")?;
    let rest = &lower[idx + 2..];
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if hex.is_empty() {
        return None;
    }
    u64::from_str_radix(&hex, 16).ok()
}

/// Layer-1 detection (roadmap §4.3): a rise in WHEA correctable errors during
/// the dwell window is a *soft fail* — instability before the hard crash.
pub fn whea_soft_fail(count_before: u32, count_after: u32) -> bool {
    count_after > count_before
}

/// Everything the loop must remember across reboots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLoopRecord {
    pub state: SafeLoopState,
    pub consecutive_crashes: u32,
    pub last_validated: Option<TuningPoint>,
    pub blacklist: Vec<BlacklistRegion>,
    pub safe_mode: bool,
    /// Most recent crash classifications, newest last (capped).
    pub crash_log: Vec<CrashClass>,
}

impl Default for SafeLoopRecord {
    fn default() -> Self {
        Self {
            state: SafeLoopState::Idle,
            consecutive_crashes: 0,
            last_validated: None,
            blacklist: Vec::new(),
            safe_mode: false,
            crash_log: Vec::new(),
        }
    }
}

impl SafeLoopRecord {
    /// Is this candidate inside any known-unstable region?
    pub fn is_blacklisted(&self, point: &TuningPoint) -> bool {
        self.blacklist.iter().any(|r| r.contains(point))
    }

    /// Record a freshly validated point and reset the crash streak.
    pub fn mark_validated(&mut self, point: TuningPoint) {
        self.last_validated = Some(point);
        self.consecutive_crashes = 0;
        self.state = SafeLoopState::Validated;
    }

    /// The point recovery should fall back to: last good, else stock.
    pub fn recovery_target(&self) -> TuningPoint {
        self.last_validated.clone().unwrap_or_else(TuningPoint::stock)
    }
}

/// The boot-flag: written to disk *before* an apply, deleted *after* a clean
/// validation. Its mere presence at boot means the last apply crashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootFlag {
    pub intent: TuningPoint,
    pub phase: String,
    pub timestamp: String,
}

impl BootFlag {
    pub fn new(intent: TuningPoint, phase: impl Into<String>) -> Self {
        Self {
            intent,
            phase: phase.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// What the service should do on boot, decided purely from persisted state
/// (roadmap §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Boot-flag was clear and nothing is validated yet — nothing to restore.
    Idle,
    /// Boot-flag clear; reapply the last known-good profile.
    ApplyLastValidated { point: TuningPoint },
    /// Boot-flag armed: blacklist the crash region and recede to known-good.
    BlacklistAndRecede {
        crashed: TuningPoint,
        recede_to: TuningPoint,
        class: CrashClass,
    },
    /// Crash threshold tripped — apply stock and stop touching anything.
    EnterSafeMode { stock: TuningPoint },
}

/// Pure recovery decision. `bugcheck` is the classification from the post-reboot
/// minidump/event analysis (layer 3); pass [`CrashClass::Unknown`] when none.
///
/// This does **not** mutate `record`; call [`apply_recovery`] to commit the
/// resulting state change once the decision is made.
pub fn decide_recovery(
    boot_flag: Option<&BootFlag>,
    bugcheck: CrashClass,
    record: &SafeLoopRecord,
) -> RecoveryAction {
    match boot_flag {
        Some(flag) => {
            // The apply that armed this flag never reached a clean validation.
            let crashes = record.consecutive_crashes.saturating_add(1);
            if crashes >= SAFE_MODE_CRASH_THRESHOLD {
                RecoveryAction::EnterSafeMode {
                    stock: TuningPoint::stock(),
                }
            } else {
                RecoveryAction::BlacklistAndRecede {
                    crashed: flag.intent.clone(),
                    recede_to: record.recovery_target(),
                    class: bugcheck,
                }
            }
        }
        None => {
            if record.safe_mode {
                RecoveryAction::EnterSafeMode {
                    stock: TuningPoint::stock(),
                }
            } else if let Some(point) = record.last_validated.clone() {
                RecoveryAction::ApplyLastValidated { point }
            } else {
                RecoveryAction::Idle
            }
        }
    }
}

/// Commit the effects of a recovery decision to the persisted record. Returns
/// the point that should be applied to hardware.
pub fn apply_recovery(record: &mut SafeLoopRecord, action: &RecoveryAction) -> TuningPoint {
    match action {
        RecoveryAction::Idle => {
            record.state = SafeLoopState::Idle;
            TuningPoint::stock()
        }
        RecoveryAction::ApplyLastValidated { point } => {
            record.state = SafeLoopState::Validated;
            point.clone()
        }
        RecoveryAction::BlacklistAndRecede {
            crashed,
            recede_to,
            class,
        } => {
            record.consecutive_crashes = record.consecutive_crashes.saturating_add(1);
            record
                .blacklist
                .push(BlacklistRegion::around(crashed.clone(), DEFAULT_BLACKLIST_RADIUS));
            push_capped(&mut record.crash_log, *class, 32);
            record.state = SafeLoopState::Unstable;
            recede_to.clone()
        }
        RecoveryAction::EnterSafeMode { stock } => {
            record.consecutive_crashes = record.consecutive_crashes.saturating_add(1);
            record.safe_mode = true;
            record.state = SafeLoopState::SafeMode;
            stock.clone()
        }
    }
}

fn push_capped<T>(v: &mut Vec<T>, item: T, cap: usize) {
    v.push(item);
    if v.len() > cap {
        let overflow = v.len() - cap;
        v.drain(0..overflow);
    }
}

// ---------------------------------------------------------------------------
// Persistence — reboot-surviving on-disk state under %ProgramData%\Nidavellir.
// ---------------------------------------------------------------------------

const BOOT_FLAG_FILE: &str = "boot_flag.json";
const RECORD_FILE: &str = "safe_loop.json";
const HEARTBEAT_FILE: &str = "heartbeat.txt";

/// Filesystem-backed store for the Safe Loop. Default location is
/// `%ProgramData%\Nidavellir` (writable by the SYSTEM/admin service and
/// preserved across reboots); tests point `base` at a temp directory.
#[derive(Debug, Clone)]
pub struct SafeLoopStore {
    base: PathBuf,
}

impl SafeLoopStore {
    /// Store rooted at the machine-wide data directory.
    pub fn system() -> Self {
        Self::new(default_data_dir())
    }

    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base
    }

    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base)
    }

    pub fn boot_flag_path(&self) -> PathBuf {
        self.base.join(BOOT_FLAG_FILE)
    }

    pub fn record_path(&self) -> PathBuf {
        self.base.join(RECORD_FILE)
    }

    pub fn heartbeat_path(&self) -> PathBuf {
        self.base.join(HEARTBEAT_FILE)
    }

    /// Arm the boot-flag before applying a point.
    pub fn arm_boot_flag(&self, flag: &BootFlag) -> std::io::Result<()> {
        self.ensure_dir()?;
        let json = serde_json::to_string_pretty(flag)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.boot_flag_path(), json)
    }

    /// Read the boot-flag if armed.
    pub fn read_boot_flag(&self) -> Option<BootFlag> {
        let data = std::fs::read_to_string(self.boot_flag_path()).ok()?;
        serde_json::from_str(strip_bom(&data)).ok()
    }

    pub fn is_boot_flag_armed(&self) -> bool {
        self.boot_flag_path().exists()
    }

    /// Clear the boot-flag after a clean validation.
    pub fn clear_boot_flag(&self) -> std::io::Result<()> {
        match std::fs::remove_file(self.boot_flag_path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Load the persisted record, or the default if none/unreadable.
    pub fn load_record(&self) -> SafeLoopRecord {
        std::fs::read_to_string(self.record_path())
            .ok()
            .and_then(|d| serde_json::from_str(strip_bom(&d)).ok())
            .unwrap_or_default()
    }

    pub fn save_record(&self, record: &SafeLoopRecord) -> std::io::Result<()> {
        self.ensure_dir()?;
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.record_path(), json)
    }

    /// Write a liveness heartbeat (roadmap §4.3, layer 2).
    pub fn write_heartbeat(&self) -> std::io::Result<()> {
        self.ensure_dir()?;
        std::fs::write(self.heartbeat_path(), chrono::Utc::now().to_rfc3339())
    }
}

/// Strip a leading UTF-8 BOM so files touched by external editors still parse.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// `%ProgramData%\Nidavellir`, falling back to a temp dir if the env is unset.
pub fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Nidavellir")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_follow_the_happy_path() {
        let mut s = SafeLoopState::Idle;
        s = transition(s, SafeLoopEvent::StartProbe);
        assert_eq!(s, SafeLoopState::Probing);
        s = transition(s, SafeLoopEvent::Applied);
        assert_eq!(s, SafeLoopState::Applying);
        s = transition(s, SafeLoopEvent::EnterDwell);
        assert_eq!(s, SafeLoopState::Dwell);
        s = transition(s, SafeLoopEvent::DwellPassed);
        assert_eq!(s, SafeLoopState::Validated);
    }

    #[test]
    fn soft_fail_and_crash_paths() {
        assert_eq!(
            transition(SafeLoopState::Dwell, SafeLoopEvent::SoftFail),
            SafeLoopState::Unstable
        );
        // A hard crash from any state lands in Unstable…
        assert_eq!(
            transition(SafeLoopState::Applying, SafeLoopEvent::HardCrash),
            SafeLoopState::Unstable
        );
        // …and the safe-mode trip overrides everything.
        assert_eq!(
            transition(SafeLoopState::Dwell, SafeLoopEvent::TripSafeMode),
            SafeLoopState::SafeMode
        );
    }

    #[test]
    fn unknown_transition_is_a_no_op() {
        assert_eq!(
            transition(SafeLoopState::Idle, SafeLoopEvent::DwellPassed),
            SafeLoopState::Idle
        );
    }

    #[test]
    fn bugcheck_classification() {
        assert_eq!(classify_bugcheck(0x124), CrashClass::OcInstability);
        assert_eq!(classify_bugcheck(0x101), CrashClass::OcInstability);
        assert_eq!(classify_bugcheck(0x133), CrashClass::OcInstability);
        assert_eq!(classify_bugcheck(0x50), CrashClass::Unrelated);
        assert_eq!(classify_bugcheck(0), CrashClass::Unknown);
    }

    #[test]
    fn bugcheck_text_parsing() {
        let msg = "The computer has rebooted from a bugcheck. The bugcheck was: \
                   0x00000124 (0x0000000000000000, 0xffff). A dump was saved.";
        assert_eq!(parse_bugcheck_code(msg), Some(0x124));
        assert_eq!(parse_bugcheck_code("0x101"), Some(0x101));
        assert_eq!(parse_bugcheck_code("no code here"), None);
    }

    #[test]
    fn whea_delta_is_soft_fail() {
        assert!(whea_soft_fail(2, 5));
        assert!(!whea_soft_fail(5, 5));
        assert!(!whea_soft_fail(5, 4));
    }

    #[test]
    fn chebyshev_and_blacklist_region() {
        let a = TuningPoint::from_axes([("vcore", -50), ("pl1", 120)]);
        let b = TuningPoint::from_axes([("vcore", -60), ("pl1", 120)]);
        assert_eq!(a.chebyshev(&b), 10);

        let region = BlacklistRegion::around(a.clone(), 15);
        assert!(region.contains(&b));
        let far = TuningPoint::from_axes([("vcore", -200), ("pl1", 120)]);
        assert!(!region.contains(&far));
    }

    #[test]
    fn missing_axis_treated_as_zero() {
        let a = TuningPoint::from_axes([("vcore", -30)]);
        let stock = TuningPoint::stock();
        assert_eq!(a.chebyshev(&stock), 30);
        assert!(stock.is_stock());
        assert!(!a.is_stock());
    }

    #[test]
    fn recovery_clean_boot_reapplies_last_validated() {
        let mut rec = SafeLoopRecord::default();
        let good = TuningPoint::from_axes([("vcore", -40)]);
        rec.mark_validated(good.clone());

        let action = decide_recovery(None, CrashClass::Unknown, &rec);
        assert_eq!(action, RecoveryAction::ApplyLastValidated { point: good.clone() });
        assert_eq!(apply_recovery(&mut rec, &action), good);
    }

    #[test]
    fn recovery_clean_boot_nothing_validated_is_idle() {
        let rec = SafeLoopRecord::default();
        assert_eq!(
            decide_recovery(None, CrashClass::Unknown, &rec),
            RecoveryAction::Idle
        );
    }

    #[test]
    fn recovery_armed_flag_blacklists_and_recedes() {
        let mut rec = SafeLoopRecord::default();
        let good = TuningPoint::from_axes([("vcore", -40)]);
        rec.mark_validated(good.clone());
        let crashed = TuningPoint::from_axes([("vcore", -80)]);
        let flag = BootFlag::new(crashed.clone(), "probing");

        let action = decide_recovery(Some(&flag), CrashClass::OcInstability, &rec);
        assert_eq!(
            action,
            RecoveryAction::BlacklistAndRecede {
                crashed: crashed.clone(),
                recede_to: good.clone(),
                class: CrashClass::OcInstability,
            }
        );
        let applied = apply_recovery(&mut rec, &action);
        assert_eq!(applied, good);
        assert_eq!(rec.consecutive_crashes, 1);
        assert!(rec.is_blacklisted(&crashed));
        assert_eq!(rec.crash_log, vec![CrashClass::OcInstability]);
    }

    #[test]
    fn three_consecutive_crashes_trip_safe_mode() {
        let mut rec = SafeLoopRecord::default();
        rec.consecutive_crashes = 2; // two prior crashes already on record
        let crashed = TuningPoint::from_axes([("vcore", -80)]);
        let flag = BootFlag::new(crashed, "probing");

        let action = decide_recovery(Some(&flag), CrashClass::OcInstability, &rec);
        assert_eq!(
            action,
            RecoveryAction::EnterSafeMode {
                stock: TuningPoint::stock()
            }
        );
        let applied = apply_recovery(&mut rec, &action);
        assert!(applied.is_stock());
        assert!(rec.safe_mode);
        assert_eq!(rec.state, SafeLoopState::SafeMode);
        assert_eq!(rec.consecutive_crashes, 3);
    }

    #[test]
    fn safe_mode_persists_on_clean_boot() {
        let mut rec = SafeLoopRecord::default();
        rec.safe_mode = true;
        assert_eq!(
            decide_recovery(None, CrashClass::Unknown, &rec),
            RecoveryAction::EnterSafeMode {
                stock: TuningPoint::stock()
            }
        );
    }

    #[test]
    fn store_roundtrips_flag_and_record() {
        let dir = std::env::temp_dir().join(format!("nidavellir-sl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SafeLoopStore::new(&dir);

        assert!(!store.is_boot_flag_armed());
        let flag = BootFlag::new(TuningPoint::from_axes([("vcore", -75)]), "probing");
        store.arm_boot_flag(&flag).unwrap();
        assert!(store.is_boot_flag_armed());
        assert_eq!(store.read_boot_flag().unwrap().intent, flag.intent);

        let mut rec = SafeLoopRecord::default();
        rec.mark_validated(TuningPoint::from_axes([("vcore", -40)]));
        store.save_record(&rec).unwrap();
        assert_eq!(store.load_record(), rec);

        store.clear_boot_flag().unwrap();
        assert!(!store.is_boot_flag_armed());
        store.clear_boot_flag().unwrap(); // idempotent

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_record_tolerates_utf8_bom() {
        let dir = std::env::temp_dir().join(format!("nidavellir-sl-bom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SafeLoopStore::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let json = "\u{feff}{\"intent\":{\"axes\":{\"vcore\":-80}},\"phase\":\"probing\",\"timestamp\":\"2026-05-31T05:00:00Z\"}";
        std::fs::write(store.boot_flag_path(), json).unwrap();
        let flag = store.read_boot_flag().expect("BOM-prefixed flag should still parse");
        assert_eq!(flag.intent, TuningPoint::from_axes([("vcore", -80)]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_record_defaults_when_missing() {
        let dir = std::env::temp_dir().join(format!("nidavellir-sl-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SafeLoopStore::new(&dir);
        assert_eq!(store.load_record(), SafeLoopRecord::default());
    }
}
