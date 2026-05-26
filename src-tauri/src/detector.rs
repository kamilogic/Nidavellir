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

    let max_freq = read_reg_u32(&hklm, cpu_path, "~MHz").unwrap_or(0);

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
        base_freq_mhz: max_freq / 2,
        max_freq_mhz: max_freq,
    }
}

fn detect_gpu() -> Vec<GpuInfo> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let gpu_base = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    let Ok(base) = hklm.open_subkey_with_flags(gpu_base, KEY_READ) else {
        return vec![];
    };

    let vram_map = query_vram_wmic();

    let mut gpus = Vec::new();
    for i in 0..32 {
        let path = format!("{i:04}");
        let Ok(sub) = base.open_subkey_with_flags(&path, KEY_READ) else {
            continue;
        };
        let model: Option<String> = sub.get_value("DriverDesc").ok();
        let vram_reg: Option<u32> = sub.get_value("HardwareInformation.AdapterRAM").ok();
        let vram_wmic = model.as_ref().and_then(|m| vram_map.get(m.as_str())).copied();

        if let Some(model) = model {
            let vendor = if model.to_lowercase().contains("nvidia") {
                "NVIDIA"
            } else if model.to_lowercase().contains("amd") || model.to_lowercase().contains("radeon")
            {
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

            let vram_mb = vram_wmic
                .or_else(|| vram_reg.map(|b| b / (1024 * 1024)))
                .unwrap_or(0);

            gpus.push(GpuInfo {
                vendor,
                model: model.trim().to_string(),
                vram_mb,
            });
        }
    }
    gpus
}

fn query_vram_wmic() -> std::collections::HashMap<String, u32> {
    let mut map = std::collections::HashMap::new();
    let output = std::process::Command::new("wmic")
        .args(["path", "Win32_VideoController", "get", "Name,AdapterRAM", "/format:csv"])
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines().skip(1) {
                let mut parts = line.split(',');
                let _node = parts.next();
                let name = parts.next().unwrap_or("").trim().to_string();
                let vram_str = parts.next().unwrap_or("0").trim().to_string();
                let vram_bytes: u64 = vram_str.parse().unwrap_or(0);
                if !name.is_empty() && vram_bytes > 0 {
                    map.insert(name, (vram_bytes / (1024 * 1024)) as u32);
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
