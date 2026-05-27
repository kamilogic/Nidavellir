use serde::{Deserialize, Serialize};

use crate::detector::HardwareInfo;
use crate::fingerprint::MachineFingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBucket {
    Automatic,
    NeedsAction,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub bucket: CapabilityBucket,
    pub estimated_gain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub fingerprint: MachineFingerprint,
    pub automatic: Vec<CapabilityItem>,
    pub needs_action: Vec<CapabilityItem>,
    pub blocked: Vec<CapabilityItem>,
    pub probe_pass2_status: ProbePass2Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbePass2Status {
    PendingDriver,
    Ready,
    Completed,
}

pub fn build_capability_report(hw: &HardwareInfo) -> CapabilityReport {
    let fingerprint = crate::fingerprint::compute_fingerprint(hw);
    let mut automatic = Vec::new();
    let mut needs_action = Vec::new();
    let mut blocked = Vec::new();

    if hw.gpu.iter().any(|g| g.vendor == "NVIDIA" || g.vendor == "AMD") {
        automatic.push(CapabilityItem {
            id: "gpu_tuning".into(),
            title: "GPU undervolt & power tuning".into(),
            description: "Adjustable via NVAPI/ADLX entirely from Windows — reversible, no reboot.".into(),
            bucket: CapabilityBucket::Automatic,
            estimated_gain: Some("Lower temps and noise at same or better sustained clock".into()),
        });
    }

    automatic.push(CapabilityItem {
        id: "cpu_power_limits".into(),
        title: "CPU power limits (PL1/PL2)".into(),
        description: "Package power limits via MSR on most Intel/AMD platforms.".into(),
        bucket: CapabilityBucket::Automatic,
        estimated_gain: Some("Sustained boost under power-limited workloads".into()),
    });

    automatic.push(CapabilityItem {
        id: "cpu_turbo_cstates".into(),
        title: "Turbo ratios & C-states".into(),
        description: "MSR-accessible on supported platforms when driver is loaded.".into(),
        bucket: CapabilityBucket::Automatic,
        estimated_gain: None,
    });

    if !hw.ram.xmp_enabled && hw.ram.configured_speed_mts <= 2400 {
        needs_action.push(CapabilityItem {
            id: "enable_xmp".into(),
            title: "Enable XMP / EXPO profile".into(),
            description: format!(
                "RAM is running at {} MT/s — likely below module rating. Enable XMP in BIOS.",
                hw.ram.configured_speed_mts
            ),
            bucket: CapabilityBucket::NeedsAction,
            estimated_gain: Some("Recover rated RAM frequency (often 2× JEDEC speed)".into()),
        });
    }

    if hw.cpu.generation.map_or(false, |g| g >= 11) && hw.cpu.vendor == "Intel" {
        blocked.push(CapabilityItem {
            id: "cpu_undervolt".into(),
            title: "CPU undervolt (OC Mailbox)".into(),
            description: "Intel 11th gen+ blocks runtime undervolt after Plundervolt mitigations. BIOS/UEFI path only.".into(),
            bucket: CapabilityBucket::Blocked,
            estimated_gain: None,
        });
    } else if hw.cpu.vendor == "AMD" {
        blocked.push(CapabilityItem {
            id: "cpu_undervolt_amd".into(),
            title: "CPU Curve Optimizer (runtime)".into(),
            description: "AMD Curve Optimizer is safest via BIOS PBO2. Runtime SMU access is risky and often locked.".into(),
            bucket: CapabilityBucket::Blocked,
            estimated_gain: None,
        });
    } else if hw.cpu.vendor == "Intel" {
        automatic.push(CapabilityItem {
            id: "cpu_undervolt".into(),
            title: "CPU undervolt (OC Mailbox)".into(),
            description: "May be available on pre-11th gen Intel when BIOS does not lock the mailbox.".into(),
            bucket: CapabilityBucket::Automatic,
            estimated_gain: Some("Lower CPU temps — pass 2 probe confirms unlock".into()),
        });
    }

    blocked.push(CapabilityItem {
        id: "ram_timings_runtime".into(),
        title: "RAM primary timings at runtime".into(),
        description: "Memory controller is trained at POST. Runtime timing changes are not stable — use BIOS/NVRAM.".into(),
        bucket: CapabilityBucket::Blocked,
        estimated_gain: None,
    });

    CapabilityReport {
        fingerprint,
        automatic,
        needs_action,
        blocked,
        probe_pass2_status: ProbePass2Status::PendingDriver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::{
        CpuInfo, GpuInfo, HardwareInfo, MotherboardInfo, RamInfo,
    };

    fn sample_hw(cpu: CpuInfo, ram: RamInfo) -> HardwareInfo {
        HardwareInfo {
            cpu,
            gpu: vec![GpuInfo {
                vendor: "NVIDIA".into(),
                model: "RTX 3060".into(),
                vram_mb: 12288,
                max_core_clock_mhz: None,
                max_memory_clock_mhz: None,
            }],
            ram,
            motherboard: MotherboardInfo {
                vendor: "Biostar".into(),
                model: "H610".into(),
                bios_version: "1.0".into(),
            },
        }
    }

    #[test]
    fn intel_12th_blocks_runtime_undervolt() {
        let hw = sample_hw(
            CpuInfo {
                vendor: "Intel".into(),
                model: "Core i5-12400".into(),
                cores: 6,
                threads: 12,
                base_freq_mhz: 2500,
                max_freq_mhz: 4400,
                generation: Some(12),
            },
            RamInfo {
                total_mb: 16384,
                modules: vec![],
                xmp_enabled: false,
                configured_speed_mts: 2133,
                rated_speed_mts: None,
            },
        );
        let report = build_capability_report(&hw);
        assert!(report.blocked.iter().any(|i| i.id == "cpu_undervolt"));
        assert!(report.needs_action.iter().any(|i| i.id == "enable_xmp"));
        assert!(report.automatic.iter().any(|i| i.id == "gpu_tuning"));
    }

    #[test]
    fn intel_10th_may_allow_undervolt() {
        let hw = sample_hw(
            CpuInfo {
                vendor: "Intel".into(),
                model: "Core i7-10700K".into(),
                cores: 8,
                threads: 16,
                base_freq_mhz: 3800,
                max_freq_mhz: 5100,
                generation: Some(10),
            },
            RamInfo {
                total_mb: 32768,
                modules: vec![],
                xmp_enabled: true,
                configured_speed_mts: 3200,
                rated_speed_mts: None,
            },
        );
        let report = build_capability_report(&hw);
        assert!(report.automatic.iter().any(|i| i.id == "cpu_undervolt"));
        assert!(report.needs_action.is_empty());
    }
}
