use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const BOOT_FLAG_PATH: &str = "C:\\ProgramData\\Nidavellir\\boot_flag";
const BOOT_FLAG_CLEAN: &str = "CLEAN";
const BOOT_FLAG_CRASH: &str = "CRASH";

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
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            boot_flag_path: PathBuf::from(BOOT_FLAG_PATH),
            sys: sysinfo::System::new(),
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
            whea: check_whea_errors(),
            boot_status: self.check_boot_status(),
        }
    }

    fn read_cpu_sensors(&mut self) -> CpuSensors {
        self.sys.refresh_cpu_usage();
        CpuSensors {
            utilization_pct: self.sys.global_cpu_usage() as f64,
            clock_mhz: read_cpu_clock(),
        }
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
                Ok(content) => content.trim() == BOOT_FLAG_CRASH,
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

fn read_cpu_clock() -> Option<u32> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
    hklm.open_subkey_with_flags(path, KEY_READ).ok()
        .and_then(|k| k.get_value("~MHz").ok())
}

fn check_whea_errors() -> WheaInfo {
    let output = std::process::Command::new("wevtutil")
        .args(["qe", "Microsoft-Windows-Kernel-WHEA/Operational", "/c:5", "/rd:true", "/f:text"])
        .output();
    match &output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let error_count = text.matches("Event ID").count() as u32;
            let last_error = text.lines()
                .skip_while(|l| !l.contains("Event ID"))
                .skip(1)
                .find(|l| l.contains(":"))
                .or_else(|| text.lines().find(|l| l.contains("Error")))
                .map(|l| l.trim().to_string());
            WheaInfo { error_count, last_error }
        }
        _ => WheaInfo { error_count: 0, last_error: None },
    }
}
