//! Super I/O VIN mapping resolved from SMBIOS + probed chip + `data/superio_profiles.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::detector::MotherboardInfo;
use crate::sensor_meta::{SensorQuality, SensorSource};

const VOLT_LSB_MV: f32 = (3.3 / 256.0) * 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuperIoVendor {
    Ite,
    Nuvoton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RailRole {
    CpuVcore,
    Dram,
    TwelveVolt,
    FiveVolt,
    CpuSa,
    CpuVio,
    Vsb,
    V33,
    Avcc,
    Unknown,
}

impl RailRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CpuVcore => "cpu_vcore",
            Self::Dram => "dram",
            Self::TwelveVolt => "12v",
            Self::FiveVolt => "5v",
            Self::CpuSa => "cpu_sa",
            Self::CpuVio => "cpu_vio",
            Self::Vsb => "vsb",
            Self::V33 => "3v3",
            Self::Avcc => "avcc",
            Self::Unknown => "unknown",
        }
    }

    fn from_json(s: &str) -> Self {
        match s {
            "cpu_vcore" => Self::CpuVcore,
            "dram" => Self::Dram,
            "12v" => Self::TwelveVolt,
            "5v" => Self::FiveVolt,
            "cpu_sa" => Self::CpuSa,
            "cpu_vio" => Self::CpuVio,
            "vsb" => Self::Vsb,
            "3v3" => Self::V33,
            "avcc" => Self::Avcc,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct VinChannel {
    index: u8,
    label: String,
    role: RailRole,
    divider: f32,
    scale: u16,
    r1: Option<f32>,
    r2: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileOrigin {
    BoardMatch,
    ChipFamily,
    GenericIte,
}

/// Max VIN channels (NCT6798D exposes 15; ITE boards use the first 9).
pub const VIN_RAW_LEN: usize = 18;

#[derive(Debug, Clone)]
pub struct SuperIoProbe {
    pub chip_id: u16,
    pub vendor: SuperIoVendor,
    pub lpc_slot: u8,
    pub io_base: u16,
    pub vin_raw: [u8; VIN_RAW_LEN],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotherboardRail {
    pub label: String,
    pub role: String,
    pub channel: u8,
    pub voltage_mv: u32,
    pub source: String,
    pub quality: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSuperIo {
    pub chip_id_hex: String,
    pub profile_id: String,
    pub profile_source: String,
    pub board_vendor: String,
    pub board_model: String,
    pub rails: Vec<MotherboardRail>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRails {
    pub meta: ResolvedSuperIo,
    pub cpu_vcore_mv: Option<u32>,
    pub dram_mv: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ProfilesFile {
    board_rules: Vec<JsonBoardRule>,
    profiles: HashMap<String, JsonProfile>,
}

#[derive(Debug, Deserialize)]
struct JsonBoardRule {
    profile_id: String,
    vendor_contains: Vec<String>,
    product_contains: Vec<String>,
    #[serde(default)]
    chip_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct JsonProfile {
    vendor: String,
    #[serde(default)]
    chip_ids: Vec<String>,
    vin: Vec<JsonVin>,
}

#[derive(Debug, Deserialize)]
struct JsonVin {
    index: u8,
    label: String,
    role: String,
    #[serde(default)]
    divider: Option<f32>,
    #[serde(default)]
    scale: Option<u16>,
    /// LHM-style resistor divider (e.g. +12V uses r1=11, r2=1).
    #[serde(default)]
    r1: Option<f32>,
    #[serde(default)]
    r2: Option<f32>,
}

static PROFILES_DB: OnceLock<ProfilesFile> = OnceLock::new();

fn profiles_db() -> &'static ProfilesFile {
    PROFILES_DB.get_or_init(|| {
        const RAW: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/superio_profiles.json"
        ));
        serde_json::from_str(RAW).expect("data/superio_profiles.json must be valid")
    })
}

/// Map probed Super I/O + SMBIOS motherboard identity to labeled rails.
pub fn resolve_superio(
    motherboard: &MotherboardInfo,
    probe: Option<&SuperIoProbe>,
) -> Option<ResolvedRails> {
    let probe = probe?;
    if probe.chip_id == 0 || probe.chip_id == 0xFFFF {
        return None;
    }

    let (channels, profile_id, origin) =
        select_profile(&motherboard.vendor, &motherboard.model, probe);

    let mut rails = Vec::with_capacity(channels.len());
    for ch in &channels {
        let idx = ch.index as usize;
        if idx >= probe.vin_raw.len() {
            continue;
        }
        let raw = probe.vin_raw[idx];
        if raw == 0 || raw == 0xFF {
            continue;
        }
        let mv = voltage_mv(probe.vendor, raw, ch);
        if mv == 0 {
            continue;
        }
        let quality = if is_plausible_for_role(ch.role, mv) {
            SensorQuality::Live
        } else {
            SensorQuality::Unavailable
        };
        rails.push(MotherboardRail {
            label: ch.label.clone(),
            role: ch.role.as_str().to_string(),
            channel: ch.index,
            voltage_mv: mv,
            source: SensorSource::SuperIo.as_str().to_string(),
            quality: quality.as_str().to_string(),
        });
    }

    if rails.is_empty() {
        return None;
    }

    let cpu_vcore_mv = rails
        .iter()
        .find(|r| r.role == RailRole::CpuVcore.as_str() && r.quality == SensorQuality::Live.as_str())
        .map(|r| r.voltage_mv);
    let dram_mv = rails
        .iter()
        .find(|r| r.role == RailRole::Dram.as_str() && r.quality == SensorQuality::Live.as_str())
        .map(|r| r.voltage_mv);

    let profile_source = match origin {
        ProfileOrigin::BoardMatch => "board_match",
        ProfileOrigin::ChipFamily => "chip_family",
        ProfileOrigin::GenericIte => "generic_ite",
    };

    Some(ResolvedRails {
        meta: ResolvedSuperIo {
            chip_id_hex: format!("0x{:04X}", probe.chip_id),
            profile_id,
            profile_source: profile_source.to_string(),
            board_vendor: motherboard.vendor.clone(),
            board_model: motherboard.model.clone(),
            rails,
        },
        cpu_vcore_mv,
        dram_mv,
    })
}

fn voltage_mv(vendor: SuperIoVendor, raw: u8, ch: &VinChannel) -> u32 {
    let base_mv = match vendor {
        SuperIoVendor::Nuvoton => {
            let scale = if ch.scale > 0 { ch.scale } else { 800 };
            (raw as u32 * scale as u32) / 100
        }
        SuperIoVendor::Ite => (raw as f32 * VOLT_LSB_MV * ch.divider) as u32,
    };
    apply_resistor_divider(base_mv, ch.r1, ch.r2)
}

fn apply_resistor_divider(base_mv: u32, r1: Option<f32>, r2: Option<f32>) -> u32 {
    match (r1, r2) {
        (Some(a), Some(b)) if b > 0.0 => ((base_mv as f32) * (a + b) / b).round() as u32,
        _ => base_mv,
    }
}

fn is_plausible_for_role(role: RailRole, mv: u32) -> bool {
    match role {
        RailRole::CpuVcore | RailRole::CpuSa | RailRole::CpuVio => (400..=2200).contains(&mv),
        RailRole::Dram => (900..=2200).contains(&mv),
        RailRole::TwelveVolt => (10_000..=14_000).contains(&mv),
        RailRole::FiveVolt => (4_000..=6_000).contains(&mv),
        RailRole::V33 | RailRole::Avcc => (2_800..=3_600).contains(&mv),
        RailRole::Vsb => (2_800..=4_000).contains(&mv),
        RailRole::Unknown => (100..=15_000).contains(&mv),
    }
}

fn normalize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn parse_chip_id(hex: &str) -> Option<u16> {
    let s = hex.trim().strip_prefix("0x").unwrap_or(hex);
    u16::from_str_radix(s, 16).ok()
}

fn json_vin_to_channels(vin: &[JsonVin], vendor: &str) -> Vec<VinChannel> {
    vin.iter()
        .map(|v| VinChannel {
            index: v.index,
            label: v.label.clone(),
            role: RailRole::from_json(&v.role),
            divider: v.divider.unwrap_or(1.0),
            scale: v.scale.unwrap_or(if vendor == "nuvoton" { 800 } else { 0 }),
            r1: v.r1,
            r2: v.r2,
        })
        .collect()
}

fn select_profile(
    vendor: &str,
    product: &str,
    probe: &SuperIoProbe,
) -> (Vec<VinChannel>, String, ProfileOrigin) {
    let db = profiles_db();
    let v = normalize_id(vendor);
    let p = normalize_id(product);

    for rule in &db.board_rules {
        if !rule.vendor_contains.iter().any(|s| v.contains(&normalize_id(s))) {
            continue;
        }
        if !rule
            .product_contains
            .iter()
            .any(|s| p.contains(&normalize_id(s)))
        {
            continue;
        }
        if !rule.chip_ids.is_empty()
            && !rule
                .chip_ids
                .iter()
                .filter_map(|s| parse_chip_id(s))
                .any(|id| id == probe.chip_id)
        {
            continue;
        }
        if let Some(profile) = db.profiles.get(&rule.profile_id) {
            if profile_vendor_matches(probe, profile) {
                return (
                    json_vin_to_channels(&profile.vin, &profile.vendor),
                    rule.profile_id.clone(),
                    ProfileOrigin::BoardMatch,
                );
            }
        }
    }

    let mut profile_ids: Vec<_> = db.profiles.keys().collect();
    profile_ids.sort();
    for id in profile_ids {
        if is_board_specific_profile_id(id) {
            continue;
        }
        let profile = &db.profiles[id];
        if profile_vendor_matches(probe, profile)
            && profile
                .chip_ids
                .iter()
                .filter_map(|s| parse_chip_id(s))
                .any(|cid| cid == probe.chip_id)
        {
            return (
                json_vin_to_channels(&profile.vin, &profile.vendor),
                id.clone(),
                ProfileOrigin::ChipFamily,
            );
        }
    }

    if probe.vendor == SuperIoVendor::Nuvoton {
        if let Some(profile) = db.profiles.get("nct6798d_generic") {
            return (
                json_vin_to_channels(&profile.vin, &profile.vendor),
                "nct6798d_generic".into(),
                ProfileOrigin::ChipFamily,
            );
        }
    }

    (
        generic_ite_channels(),
        "generic_ite_9vin".into(),
        ProfileOrigin::GenericIte,
    )
}

fn is_board_specific_profile_id(id: &str) -> bool {
    id.contains("asus") || id.contains("msi_") || id.contains("gigabyte")
}

fn profile_vendor_matches(probe: &SuperIoProbe, profile: &JsonProfile) -> bool {
    match probe.vendor {
        SuperIoVendor::Ite => profile.vendor == "ite",
        SuperIoVendor::Nuvoton => profile.vendor == "nuvoton",
    }
}

fn generic_ite_channels() -> Vec<VinChannel> {
    (0u8..9)
        .map(|i| VinChannel {
            index: i,
            label: format!("VIN{i}"),
            role: RailRole::Unknown,
            divider: match i {
                2 => 10.0,
                5 => 3.0,
                _ => 1.0,
            },
            scale: 0,
            r1: None,
            r2: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_profiles_load() {
        let db = profiles_db();
        assert!(db.profiles.contains_key("nct6798d_generic"));
    }

    #[test]
    fn board_rule_matches_normalized_asus_z690_nuvoton() {
        let mb = MotherboardInfo {
            vendor: "ASUSTeK COMPUTER INC.".into(),
            model: "TUF GAMING Z690-PLUS D4".into(),
            bios_version: "1.0".into(),
        };
        let probe = SuperIoProbe {
            chip_id: 0xD428,
            vendor: SuperIoVendor::Nuvoton,
            lpc_slot: 0,
            io_base: 0x0A20,
            vin_raw: [100; VIN_RAW_LEN],
        };
        let r = resolve_superio(&mb, Some(&probe)).expect("resolved");
        assert_eq!(r.meta.profile_source, "board_match");
        assert_eq!(r.meta.profile_id, "asus_tuf_z690_plus_d4");
        assert!(r.cpu_vcore_mv.unwrap() > 0);
    }

    #[test]
    fn nuvoton_scaling_uses_scale_not_ite_lsb() {
        let mb = MotherboardInfo {
            vendor: "ASUSTeK COMPUTER INC.".into(),
            model: "TUF GAMING Z690-PLUS D4".into(),
            bios_version: "1.0".into(),
        };
        let probe = SuperIoProbe {
            chip_id: 0xD428,
            vendor: SuperIoVendor::Nuvoton,
            lpc_slot: 0,
            io_base: 0x0A20,
            vin_raw: {
                let mut v = [0u8; VIN_RAW_LEN];
                v[0] = 100;
                v
            },
        };
        let r = resolve_superio(&mb, Some(&probe)).expect("resolved");
        assert_eq!(r.cpu_vcore_mv, Some(800));
    }

    #[test]
    fn unknown_board_uses_chip_family_ite() {
        let mb = MotherboardInfo {
            vendor: "Unknown OEM".into(),
            model: "XYZ-9000".into(),
            bios_version: "?".into(),
        };
        let probe = SuperIoProbe {
            chip_id: 0x8688,
            vendor: SuperIoVendor::Ite,
            lpc_slot: 0,
            io_base: 0x0A20,
            vin_raw: [50; VIN_RAW_LEN],
        };
        let r = resolve_superio(&mb, Some(&probe)).expect("resolved");
        assert_eq!(r.meta.profile_source, "chip_family");
        assert_eq!(r.meta.profile_id, "ite8688_generic");
    }
}
