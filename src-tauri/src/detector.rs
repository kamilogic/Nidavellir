pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub gpu: Vec<GpuInfo>,
    pub ram: RamInfo,
    pub motherboard: MotherboardInfo,
}

pub struct CpuInfo {
    pub vendor: String,
    pub model: String,
    pub cores: u32,
    pub threads: u32,
    pub base_freq_mhz: u32,
    pub max_freq_mhz: u32,
}

pub struct GpuInfo {
    pub vendor: String,
    pub model: String,
    pub vram_mb: u32,
}

pub struct RamInfo {
    pub total_mb: u64,
    pub modules: Vec<RamModule>,
}

pub struct RamModule {
    pub size_mb: u32,
    pub speed_mts: u32,
}

pub struct MotherboardInfo {
    pub vendor: String,
    pub model: String,
    pub bios_version: String,
}

pub fn detect_all() -> HardwareInfo {
    HardwareInfo {
        cpu: CpuInfo {
            vendor: "Unknown".into(),
            model: "Unknown".into(),
            cores: 0,
            threads: 0,
            base_freq_mhz: 0,
            max_freq_mhz: 0,
        },
        gpu: vec![],
        ram: RamInfo {
            total_mb: 0,
            modules: vec![],
        },
        motherboard: MotherboardInfo {
            vendor: "Unknown".into(),
            model: "Unknown".into(),
            bios_version: "Unknown".into(),
        },
    }
}
