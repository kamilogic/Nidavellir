use serde::Serialize;
use winreg::enums::*;
use winreg::RegKey;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub gpu: Vec<GpuInfo>,
    pub ram: RamInfo,
    pub motherboard: MotherboardInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuInfo {
    pub vendor: String,
    pub model: String,
    pub cores: u32,
    pub threads: u32,
    pub base_freq_mhz: u32,
    pub max_freq_mhz: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub vendor: String,
    pub model: String,
    pub vram_mb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RamInfo {
    pub total_mb: u64,
    pub modules: Vec<RamModule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RamModule {
    pub size_mb: u32,
    pub speed_mts: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MotherboardInfo {
    pub vendor: String,
    pub model: String,
    pub bios_version: String,
}

pub fn detect_all() -> HardwareInfo {
    HardwareInfo {
        cpu: detect_cpu(),
        gpu: detect_gpu(),
        ram: detect_ram(),
        motherboard: detect_motherboard(),
    }
}

fn read_reg_str(key: &RegKey, path: &str, name: &str) -> Option<String> {
    key.open_subkey_with_flags(path, KEY_READ)
        .ok()
        .and_then(|sub| sub.get_value(name).ok())
}

fn read_reg_u32(key: &RegKey, path: &str, name: &str) -> Option<u32> {
    key.open_subkey_with_flags(path, KEY_READ)
        .ok()
        .and_then(|sub| sub.get_value(name).ok())
}

fn detect_cpu() -> CpuInfo {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let cpu_path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

    let model = read_reg_str(&hklm, cpu_path, "ProcessorNameString")
        .unwrap_or_else(|| "Unknown".into());

    // Registry `~MHz` reports the rated base clock (not turbo).
    // For turbo/max, we query WMI; fall back to base if unavailable.
    let base_freq = read_reg_u32(&hklm, cpu_path, "~MHz").unwrap_or(0);
    let max_freq = query_cpu_max_clock_wmic().unwrap_or(base_freq);

    let vendor = if model.to_lowercase().contains("intel") {
        "Intel".to_string()
    } else if model.to_lowercase().contains("amd") {
        "AMD".to_string()
    } else {
        read_reg_str(&hklm, cpu_path, "VendorIdentifier")
            .unwrap_or_else(|| "Unknown".into())
    };

    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let cores = read_reg_u32(&hklm, cpu_path, "CoreCount").unwrap_or(threads);

    CpuInfo {
        vendor,
        model,
        cores,
        threads,
        base_freq_mhz: base_freq,
        max_freq_mhz: max_freq,
    }
}

fn query_cpu_max_clock_wmic() -> Option<u32> {
    // Win32_Processor.MaxClockSpeed reports the advertised max (base clock on many CPUs,
    // but BIOS sometimes reports the boost clock). Better than max/2.
    let output = std::process::Command::new("wmic")
        .args(["cpu", "get", "MaxClockSpeed", "/format:list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(val) = line.trim().strip_prefix("MaxClockSpeed=") {
            if let Ok(mhz) = val.trim().parse::<u32>() {
                if mhz > 0 {
                    return Some(mhz);
                }
            }
        }
    }
    None
}

fn detect_gpu() -> Vec<GpuInfo> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let gpu_base = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    let Ok(base) = hklm.open_subkey_with_flags(gpu_base, KEY_READ) else {
        return vec![];
    };

    // nvidia-smi: most accurate for NVIDIA (avoids uint32 WMI overflow)
    let smi_vram = query_vram_nvidia_smi();
    // Registry: try QWORD keys (handles >4GB correctly for NVIDIA)
    let reg_vram = query_vram_registry();
    // WMIC AdapterRAM: uint32 fallback, wraps to 0 for cards >4GB
    let wmic_vram = query_vram_wmic();

    let mut gpus = Vec::new();
    for i in 0..32 {
        let path = format!("{i:04}");
        let Ok(sub) = base.open_subkey_with_flags(&path, KEY_READ) else {
            continue;
        };
        let Ok(model): Result<String, _> = sub.get_value("DriverDesc") else {
            continue;
        };

        let vendor = if model.to_lowercase().contains("nvidia") {
            "NVIDIA"
        } else if model.to_lowercase().contains("amd") || model.to_lowercase().contains("radeon") {
            "AMD"
        } else if model.to_lowercase().contains("intel") {
            "Intel"
        } else if model.to_lowercase().contains("microsoft")
            || model.to_lowercase().contains("basic")
        {
            continue;
        } else {
            "Unknown"
        }
        .to_string();

        let normalized = model.trim().to_lowercase();
        // Priority: nvidia-smi > registry QWORD > wmic uint32 (filters 0 to avoid overflow artifacts)
        let vram_mb = smi_vram
            .get(&normalized)
            .copied()
            .or_else(|| {
                reg_vram.get(&normalized).copied()
                    .filter(|&b| b > 0)
                    .map(|b| (b / (1024 * 1024)) as u32)
            })
            .or_else(|| wmic_vram.get(&normalized).copied().filter(|&v| v > 0))
            .unwrap_or(0);

        gpus.push(GpuInfo {
            vendor,
            model: model.trim().to_string(),
            vram_mb,
        });
    }
    gpus
}

fn query_vram_wmic() -> std::collections::HashMap<String, u32> {
    let mut map = std::collections::HashMap::new();
    // Use /format:list to avoid comma-in-name parsing issues
    let output = std::process::Command::new("wmic")
        .args([
            "path", "Win32_VideoController",
            "get", "Name,AdapterRAM",
            "/format:list",
        ])
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut current_name = String::new();
            let mut current_vram: u64 = 0;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    if !current_name.is_empty() && current_vram > 0 {
                        map.insert(current_name.clone(), (current_vram / (1024 * 1024)) as u32);
                    }
                    current_name.clear();
                    current_vram = 0;
                    continue;
                }
                if let Some(val) = line.strip_prefix("Name=") {
                    current_name = val.trim().to_lowercase();
                } else if let Some(val) = line.strip_prefix("AdapterRAM=") {
                    current_vram = val.trim().parse().unwrap_or(0);
                }
            }
            // Last entry if no trailing blank line
            if !current_name.is_empty() && current_vram > 0 {
                map.insert(current_name, (current_vram / (1024 * 1024)) as u32);
            }
        }
    }
    map
}

fn query_vram_registry() -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let gpu_base = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let Ok(base) = hklm.open_subkey_with_flags(gpu_base, KEY_READ) else {
        return map;
    };
    for i in 0..32 {
        let path = format!("{i:04}");
        let Ok(sub) = base.open_subkey_with_flags(&path, KEY_READ) else {
            continue;
        };
        let Ok(model): Result<String, _> = sub.get_value("DriverDesc") else {
            continue;
        };
        let normalized = model.trim().to_lowercase();
        // qwMemorySize is a NVIDIA-specific QWORD that stores the correct size for >4GB cards.
        // Fall back to AdapterRAM as QWORD, then as DWORD (which wraps at 4GB).
        let vram: Option<u64> = sub.get_value::<u64, _>("HardwareInformation.qwMemorySize").ok()
            .or_else(|| sub.get_value::<u64, _>("HardwareInformation.AdapterRAM").ok())
            .or_else(|| sub.get_value::<u32, _>("HardwareInformation.AdapterRAM").ok().map(|b| b as u64));
        if let Some(bytes) = vram {
            map.insert(normalized, bytes);
        }
    }
    map
}

fn query_vram_nvidia_smi() -> std::collections::HashMap<String, u32> {
    let mut map = std::collections::HashMap::new();
    let Ok(output) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output() else { return map; };
    if !output.status.success() { return map; }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        // Format: "NVIDIA GeForce RTX 3060 Ti, 8192"
        if let Some(comma) = line.rfind(',') {
            let name = line[..comma].trim().to_lowercase();
            if let Ok(mib) = line[comma + 1..].trim().parse::<u32>() {
                if mib > 0 {
                    map.insert(name, mib);
                }
            }
        }
    }
    map
}

fn detect_ram() -> RamInfo {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    let total_mb = sys.total_memory() / 1024;

    RamInfo {
        total_mb,
        modules: vec![],
    }
}

fn detect_motherboard() -> MotherboardInfo {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let bios_path = r"HARDWARE\DESCRIPTION\System\BIOS";

    MotherboardInfo {
        vendor: read_reg_str(&hklm, bios_path, "BaseBoardManufacturer")
            .unwrap_or_else(|| "Unknown".into()),
        model: read_reg_str(&hklm, bios_path, "BaseBoardProduct")
            .unwrap_or_else(|| "Unknown".into()),
        bios_version: read_reg_str(&hklm, bios_path, "BIOSVersion")
            .unwrap_or_else(|| "Unknown".into()),
    }
}
