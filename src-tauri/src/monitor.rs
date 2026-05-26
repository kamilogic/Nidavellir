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
const RAM_VOLTAGE_CACHE_TTL: Duration = Duration::from_secs(30);

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
    pub voltage_mv: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WheaEvent {
    pub timestamp: Option<String>,
    pub event_id: Option<u32>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WheaInfo {
    pub error_count: u32,
    pub last_error: Option<String>,
    pub events: Vec<WheaEvent>,
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
    cached_ram_voltage: Option<(Instant, Option<u32>)>,
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
            cached_ram_voltage: None,
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
        MemorySensors { used_mb: used, total_mb: total, used_pct: pct, voltage_mv: self.read_ram_voltage_cached() }
    }

    fn read_ram_voltage_cached(&mut self) -> Option<u32> {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_ram_voltage {
            if now.duration_since(*ts) < RAM_VOLTAGE_CACHE_TTL {
                return *val;
            }
        }
        let val = read_ram_voltage();
        self.cached_ram_voltage = Some((now, val));
        val
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
    // Win32_Processor.CurrentClockSpeed is the most reliable dynamic clock source
    if let Ok(output) = std::process::Command::new("wmic")
        .args(["cpu", "get", "CurrentClockSpeed", "/format:list"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(val) = line.trim().strip_prefix("CurrentClockSpeed=") {
                    if let Ok(mhz) = val.trim().parse::<u32>() {
                        if mhz > 0 {
                            return Some(mhz);
                        }
                    }
                }
            }
        }
    }
    // Fallback: PercentProcessorPerformance * base_freq (requires perf counters)
    if base_freq_mhz > 0 {
        if let Ok(output) = std::process::Command::new("wmic")
            .args([
                "path",
                "Win32_PerfFormattedData_Counters_ProcessorInformation",
                "where",
                "Name='_Total'",
                "get",
                "PercentProcessorPerformance",
                "/format:list",
            ])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if let Some(val) = line.trim().strip_prefix("PercentProcessorPerformance=") {
                        if let Ok(pct) = val.trim().parse::<f64>() {
                            if pct > 0.0 {
                                return Some((base_freq_mhz as f64 * pct / 100.0) as u32);
                            }
                        }
                    }
                }
            }
        }
    }
    // Last resort: static base freq from registry
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey_with_flags(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", KEY_READ)
        .ok()
        .and_then(|k| k.get_value("~MHz").ok())
}

fn read_ram_voltage() -> Option<u32> {
    let output = std::process::Command::new("wmic")
        .args(["memorychip", "get", "ConfiguredVoltage", "/format:list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(val) = line.trim().strip_prefix("ConfiguredVoltage=") {
            if let Ok(mv) = val.trim().parse::<u32>() {
                if mv > 0 {
                    return Some(mv);
                }
            }
        }
    }
    None
}

fn read_cpu_voltage() -> Option<u32> {
    None // voltage injected from shared driver in lib.rs
}

fn check_whea_errors() -> WheaInfo {
    let output = std::process::Command::new("wevtutil")
        .args(["qe", "Microsoft-Windows-Kernel-WHEA/Operational", "/c:20", "/rd:true", "/f:text"])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut events: Vec<WheaEvent> = Vec::new();

            let mut cur_timestamp: Option<String> = None;
            let mut cur_event_id: Option<u32> = None;
            let mut desc_lines: Vec<String> = Vec::new();
            let mut in_description = false;
            let mut has_event = false;

            let flush = |events: &mut Vec<WheaEvent>,
                          ts: &mut Option<String>,
                          eid: &mut Option<u32>,
                          desc: &mut Vec<String>,
                          has: &mut bool| {
                if *has {
                    events.push(WheaEvent {
                        timestamp: ts.take(),
                        event_id: eid.take(),
                        description: if desc.is_empty() { None } else { Some(desc.join(" ")) },
                    });
                    desc.clear();
                    *has = false;
                }
            };

            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Event[") {
                    flush(&mut events, &mut cur_timestamp, &mut cur_event_id, &mut desc_lines, &mut has_event);
                    has_event = true;
                    in_description = false;
                } else if in_description {
                    // Stop description on blank line or a new known field
                    if trimmed.is_empty()
                        || trimmed.starts_with("Log Name:")
                        || trimmed.starts_with("Source:")
                        || trimmed.starts_with("Event ID:")
                        || trimmed.starts_with("Task:")
                        || trimmed.starts_with("Level:")
                        || trimmed.starts_with("Opcode:")
                        || trimmed.starts_with("Keyword:")
                        || trimmed.starts_with("User:")
                        || trimmed.starts_with("Computer:")
                    {
                        in_description = false;
                    } else {
                        desc_lines.push(trimmed.to_string());
                    }
                }
                if !in_description {
                    if let Some(val) = trimmed.strip_prefix("Date:") {
                        cur_timestamp = Some(val.trim().to_string());
                    } else if let Some(val) = trimmed.strip_prefix("Event ID:") {
                        cur_event_id = val.trim().parse().ok();
                    } else if let Some(val) = trimmed.strip_prefix("Description:") {
                        desc_lines.clear();
                        let inline = val.trim();
                        if !inline.is_empty() {
                            desc_lines.push(inline.to_string());
                        }
                        in_description = true;
                    }
                }
            }
            flush(&mut events, &mut cur_timestamp, &mut cur_event_id, &mut desc_lines, &mut has_event);

            let error_count = events.len() as u32;
            let last_error = events.first()
                .and_then(|e| e.description.clone())
                .or_else(|| events.first().and_then(|e| e.timestamp.as_ref().map(|t| format!("Error at {t}"))));
            WheaInfo { error_count, last_error, events }
        }
        _ => WheaInfo { error_count: 0, last_error: None, events: vec![] },
    }
}
