use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineFingerprint {
    pub hash: String,
    pub cpu_model: String,
    pub gpu_models: Vec<String>,
    pub board: String,
    pub bios_version: String,
}

pub fn compute_fingerprint(hw: &crate::detector::HardwareInfo) -> MachineFingerprint {
    let gpu_models: Vec<String> = hw.gpu.iter().map(|g| g.model.clone()).collect();
    let board = format!("{} {}", hw.motherboard.vendor, hw.motherboard.model);
    let payload = format!(
        "{}|{}|{}|{}",
        hw.cpu.model,
        gpu_models.join(","),
        board,
        hw.motherboard.bios_version
    );
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(payload.as_bytes()));
    MachineFingerprint {
        hash,
        cpu_model: hw.cpu.model.clone(),
        gpu_models,
        board,
        bios_version: hw.motherboard.bios_version.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::{
        CpuInfo, GpuInfo, HardwareInfo, MotherboardInfo, RamInfo,
    };

    #[test]
    fn fingerprint_is_stable() {
        let hw = HardwareInfo {
            cpu: CpuInfo {
                vendor: "Intel".into(),
                model: "Core i7-12700K".into(),
                cores: 12,
                threads: 20,
                base_freq_mhz: 3600,
                max_freq_mhz: 5000,
                generation: Some(12),
            },
            gpu: vec![GpuInfo {
                vendor: "NVIDIA".into(),
                model: "RTX 3080".into(),
                vram_mb: 10240,
                max_core_clock_mhz: None,
                max_memory_clock_mhz: None,
            }],
            ram: RamInfo {
                total_mb: 32768,
                modules: vec![],
                xmp_enabled: false,
                configured_speed_mts: 2133,
                rated_speed_mts: None,
            },
            motherboard: MotherboardInfo {
                vendor: "ASUS".into(),
                model: "ROG STRIX Z690".into(),
                bios_version: "1401".into(),
            },
        };
        let a = compute_fingerprint(&hw);
        let b = compute_fingerprint(&hw);
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.hash.len(), 64);
    }
}
