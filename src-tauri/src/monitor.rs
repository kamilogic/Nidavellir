use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BOOT_FLAG_PATH: &str = "C:\\ProgramData\\Nidavellir\\boot_flag";
const BOOT_FLAG_CLEAN: &str = "CLEAN";
const BOOT_FLAG_CRASH: &str = "CRASH";

/// TTL for expensive Windows queries (wmic/wevtutil). Sensor refresh hits at 2s,
/// so anything cheaper-to-cache-than-recompute should outlive several frames.
const WMIC_CACHE_TTL: Duration = Duration::from_secs(5);
const WHEA_CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize)]
pub struct SensorReadings {
    pub cpu: CpuSensors,
    pub memory: MemorySensors,
    pub whea: WheaInfo,
    pub boot_status: BootStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuSensors {
    pub utilization_pct: f64,
    pub clock_mhz: Option<u32>,
    pub voltage_mv: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemorySensors {
    pub used_mb: u64,
    pub total_mb: u64,
    pub used_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WheaInfo {
    pub error_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootStatus {
    pub previous_boot_crashed: bool,
    pub bugcheck_code: Option<u32>,
}

pub struct Monitor {
    running: Arc<AtomicBool>,
    boot_flag_path: PathBuf,
    sys: sysinfo::System,
    base_freq_mhz: u32,
    // Caches so we don't spawn `wmic`/`wevtutil` on every 2s sensor tick.
    cached_clock: Option<(Instant, Option<u32>)>,
    cached_whea: Option<(Instant, WheaInfo)>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        let base_freq_mhz = read_cpu_base_freq();
        Self {
            running: Arc::new(AtomicBool::new(false)),
            boot_flag_path: PathBuf::from(BOOT_FLAG_PATH),
            sys: sysinfo::System::new(),
            base_freq_mhz,
            cached_clock: None,
            cached_whea: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn read_sensors(&mut self) -> SensorReadings {
        SensorReadings {
            cpu: self.read_cpu_sensors(),
            memory: self.read_memory_sensors(),
            whea: self.read_whea_cached(),
            boot_status: self.check_boot_status(),
        }
    }

    fn read_cpu_sensors(&mut self) -> CpuSensors {
        self.sys.refresh_cpu_usage();
        CpuSensors {
            utilization_pct: self.sys.global_cpu_usage() as f64,
            clock_mhz: self.read_clock_cached(),
            voltage_mv: read_cpu_voltage(),
        }
    }

    fn read_clock_cached(&mut self) -> Option<u32> {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_clock {
            if now.duration_since(*ts) < WMIC_CACHE_TTL {
                return *val;
            }
        }
        let val = read_cpu_clock_wmi(self.base_freq_mhz);
        self.cached_clock = Some((now, val));
        val
    }

    fn read_whea_cached(&mut self) -> WheaInfo {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_whea {
            if now.duration_since(*ts) < WHEA_CACHE_TTL {
                return val.clone();
            }
        }
        let val = check_whea_errors();
        self.cached_whea = Some((now, val.clone()));
        val
    }

    fn read_memory_sensors(&mut self) -> MemorySensors {
        self.sys.refresh_memory();
        let total = self.sys.total_memory() / (1024 * 1024);
        let used = self.sys.used_memory() / (1024 * 1024);
        let pct = if total > 0 { (used as f64 / total as f64) * 100.0 } else { 0.0 };
        MemorySensors { used_mb: used, total_mb: total, used_pct: pct }
    }

    fn check_boot_status(&self) -> BootStatus {
        let flag_path = &self.boot_flag_path;
        let crashed = if flag_path.exists() {
            match std::fs::read_to_string(flag_path) {
                Ok(content) => content.trim().starts_with(BOOT_FLAG_CRASH),
                Err(_) => false,
            }
        } else {
            false
        };
        if crashed {
            let _ = std::fs::write(flag_path, BOOT_FLAG_CLEAN);
        }
        BootStatus { previous_boot_crashed: crashed, bugcheck_code: None }
    }

    pub fn mark_boot_clean(&self) {
        let _ = std::fs::create_dir_all(self.boot_flag_path.parent().unwrap());
        let _ = std::fs::write(&self.boot_flag_path, BOOT_FLAG_CLEAN);
    }

    pub fn mark_boot_crash(&self, code: Option<u32>) {
        let _ = std::fs::create_dir_all(self.boot_flag_path.parent().unwrap());
        let content = match code {
            Some(c) => format!("{BOOT_FLAG_CRASH}\n{c}"),
            None => BOOT_FLAG_CRASH.to_string(),
        };
        let _ = std::fs::write(&self.boot_flag_path, content);
    }
}

fn read_cpu_base_freq() -> u32 {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
    hklm.open_subkey_with_flags(path, KEY_READ).ok()
        .and_then(|k| k.get_value("~MHz").ok())
        .unwrap_or(0)
}

fn read_cpu_clock_wmi(base_freq_mhz: u32) -> Option<u32> {
    // Try WMI PercentProcessorPerformance first
    if base_freq_mhz > 0 {
        let output = std::process::Command::new("wmic")
            .args([
                "path",
                "Win32_PerfFormattedData_Counters_ProcessorInformation",
                "get",
                "PercentProcessorPerformance",
                "/format:list",
            ])
            .output()
            .ok()?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("PercentProcessorPerformance=") {
                    if let Ok(pct) = val.trim().parse::<f64>() {
                        let mhz = (base_freq_mhz as f64 * pct / 100.0) as u32;
                        return Some(mhz);
                    }
                }
            }
        }
    }
    // Fallback: read current frequency from Registry ~MHz
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey_with_flags(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", KEY_READ)
        .ok()
        .and_then(|k| k.get_value("~MHz").ok())
}

fn read_cpu_voltage() -> Option<u32> {
    None // voltage injected from shared driver in lib.rs
}

fn check_whea_errors() -> WheaInfo {
    // Use /f:text for simpler line-based output (XML is unreliable to parse line-by-line)
    let output = std::process::Command::new("wevtutil")
        .args(["qe", "Microsoft-Windows-Kernel-WHEA/Operational", "/c:10", "/rd:true", "/f:text"])
        .output();
    match &output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut error_count: u32 = 0;
            let mut event_lines: Vec<String> = Vec::new();
            let mut in_event = false;
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // In /f:text format, events are separated by blank lines
                // and start with "Event[0]:", "Event[1]:", etc.
                if trimmed.starts_with("Event[") {
                    error_count += 1;
                    in_event = true;
                    event_lines.push(trimmed.to_string());
                } else if in_event {
                    event_lines.push(trimmed.to_string());
                }
            }
            // Find a non-empty, non-zero description from the last event
            let last_error = event_lines.iter()
                .find(|l| {
                    !l.contains("Event[") // skip the header
                    && !l.contains(": 0") // skip zero values
                    && l.contains(':')
                })
                .map(|l| {
                    let parts: Vec<&str> = l.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        format!("{}: {}", parts[0].trim(), parts[1].trim())
                    } else {
                        l.to_string()
                    }
                })
                // If no field found, show the last event header
                .or_else(|| event_lines.last().map(|l| l.to_string()));
            WheaInfo { error_count, last_error }
        }
        _ => WheaInfo { error_count: 0, last_error: None },
    }
}
