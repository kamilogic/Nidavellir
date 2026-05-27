use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorSource {
    SuperIo,
    Msr,
    Wmi,
    Nvml,
    NvidiaSmi,
    #[default]
    Unknown,
}

impl SensorSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SuperIo => "superio",
            Self::Msr => "msr",
            Self::Wmi => "wmi",
            Self::Nvml => "nvml",
            Self::NvidiaSmi => "nvidia_smi",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorQuality {
    Live,
    Nominal,
    #[default]
    Unavailable,
}

impl SensorQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Nominal => "nominal",
            Self::Unavailable => "unavailable",
        }
    }
}
