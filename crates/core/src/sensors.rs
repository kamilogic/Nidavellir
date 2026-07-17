use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::nvml_gpu::read_nvidia_gpus_nvml;
use crate::sensor_input::SensorInput;
use crate::sensor_meta::{SensorQuality, SensorSource};
use crate::superio_profile::MotherboardRail;

const WMIC_CACHE_TTL: Duration = Duration::from_secs(5);
// Dashboard/Forge poll live GPU cards once per second. A 30 s cache made clock,
// voltage, fan and power look frozen even while the workload changed.
const GPU_CACHE_TTL: Duration = Duration::from_secs(1);
const RAM_VOLT_CACHE_TTL: Duration = Duration::from_secs(30);
const CPU_VOLT_CACHE_TTL: Duration = Duration::from_secs(10);
const WHEA_CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorReadings {
    pub motherboard: MotherboardSensors,
    pub cpu: CpuSensors,
    pub memory: MemorySensors,
    pub gpu: Vec<GpuSensors>,
    pub whea: WheaInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotherboardSensors {
    pub vendor: String,
    pub model: String,
    pub superio_chip: Option<String>,
    pub profile_id: String,
    pub profile_source: String,
    pub rails: Vec<MotherboardRail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuSensors {
    pub utilization_pct: f64,
    pub clock_mhz: Option<u32>,
    pub voltage_mv: Option<u32>,
    pub voltage_source: Option<String>,
    pub voltage_quality: Option<String>,
    pub temperature_c: Option<f32>,
    pub temperature_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySensors {
    pub used_mb: u64,
    pub total_mb: u64,
    pub used_pct: f64,
    pub voltage_mv: Option<u32>,
    pub voltage_source: Option<String>,
    pub voltage_quality: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuSensors {
    pub name: String,
    pub utilization_pct: Option<f64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    pub core_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    pub max_core_clock_mhz: Option<u32>,
    pub max_memory_clock_mhz: Option<u32>,
    pub fan_speed_pct: Option<u32>,
    pub voltage_mv: Option<u32>,
    pub voltage_source: Option<String>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub temperature_source: Option<String>,
    pub power_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WheaEvent {
    pub timestamp: Option<String>,
    pub event_id: Option<u32>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WheaInfo {
    pub error_count: u32,
    pub last_error: Option<String>,
    pub events: Vec<WheaEvent>,
}

pub struct SensorEngine {
    sys: sysinfo::System,
    base_freq_mhz: u32,
    cached_clock: Option<(Instant, Option<u32>)>,
    cached_whea: Option<(Instant, WheaInfo)>,
    cached_gpu: Option<(Instant, Vec<GpuSensors>)>,
    cached_ram_v: Option<(Instant, Option<u32>)>,
    cached_cpu_v: Option<(Instant, Option<u32>)>,
}

impl Default for SensorEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SensorEngine {
    pub fn new() -> Self {
        Self {
            sys: sysinfo::System::new(),
            base_freq_mhz: read_cpu_base_freq(),
            cached_clock: None,
            cached_whea: None,
            cached_gpu: None,
            cached_ram_v: None,
            cached_cpu_v: None,
        }
    }

    pub fn read(&mut self, input: &SensorInput) -> SensorReadings {
        let motherboard = if let Some(ref s) = input.superio {
            MotherboardSensors {
                vendor: s.board_vendor.clone(),
                model: s.board_model.clone(),
                superio_chip: Some(s.chip_id_hex.clone()),
                profile_id: s.profile_id.clone(),
                profile_source: s.profile_source.clone(),
                rails: s.rails.clone(),
            }
        } else {
            MotherboardSensors {
                vendor: input.motherboard.vendor.clone(),
                model: input.motherboard.model.clone(),
                superio_chip: None,
                profile_id: "none".into(),
                profile_source: "unavailable".into(),
                rails: vec![],
            }
        };

        let mut gpu = self.read_gpu_cached();
        if let Some(primary) = gpu.first_mut() {
            primary.voltage_mv = input.gpu_voltage_mv;
            primary.voltage_source = input
                .gpu_voltage_source
                .map(|source| source.as_str().to_string());
        }

        SensorReadings {
            motherboard,
            cpu: self.read_cpu(input),
            memory: self.read_memory(input),
            gpu,
            whea: self.read_whea_cached(),
        }
    }
}

pub fn read_sensors(input: &SensorInput) -> SensorReadings {
    SensorEngine::new().read(input)
}

impl SensorEngine {
    fn read_cpu(&mut self, input: &SensorInput) -> CpuSensors {
        self.sys.refresh_cpu_usage();

        let (voltage_mv, voltage_source, voltage_quality) =
            if let Some(mv) = input.cpu_vcore_mv {
                (
                    Some(mv),
                    input.cpu_vcore_source.map(|s| s.as_str().to_string()),
                    Some(input.cpu_vcore_quality.as_str().to_string()),
                )
            } else if let Some(mv) = self.read_cpu_voltage_cached() {
                (
                    Some(mv),
                    Some(SensorSource::Wmi.as_str().to_string()),
                    Some(SensorQuality::Nominal.as_str().to_string()),
                )
            } else {
                (None, None, None)
            };

        CpuSensors {
            utilization_pct: self.sys.global_cpu_usage() as f64,
            clock_mhz: self.read_clock_cached(),
            voltage_mv,
            voltage_source,
            voltage_quality,
            temperature_c: input.cpu_temp_c,
            temperature_source: input
                .cpu_temp_source
                .map(|s| s.as_str().to_string()),
        }
    }

    fn read_memory(&mut self, input: &SensorInput) -> MemorySensors {
        self.sys.refresh_memory();
        let total = self.sys.total_memory() / (1024 * 1024);
        let used = self.sys.used_memory() / (1024 * 1024);
        let pct = if total > 0 {
            (used as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let (voltage_mv, voltage_source, voltage_quality) =
            if let Some(mv) = input.dram_mv {
                (
                    Some(mv),
                    input.dram_source.map(|s| s.as_str().to_string()),
                    Some(input.dram_quality.as_str().to_string()),
                )
            } else if let Some(mv) = self.read_ram_voltage_cached() {
                (
                    Some(mv),
                    Some(SensorSource::Wmi.as_str().to_string()),
                    Some(SensorQuality::Nominal.as_str().to_string()),
                )
            } else {
                (None, None, None)
            };

        MemorySensors {
            used_mb: used,
            total_mb: total,
            used_pct: pct,
            voltage_mv,
            voltage_source,
            voltage_quality,
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
}

impl SensorEngine {
    fn read_gpu_cached(&mut self) -> Vec<GpuSensors> {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_gpu {
            if now.duration_since(*ts) < GPU_CACHE_TTL {
                return val.clone();
            }
        }
        let val = read_gpu_sensors();
        self.cached_gpu = Some((now, val.clone()));
        val
    }

    fn read_ram_voltage_cached(&mut self) -> Option<u32> {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_ram_v {
            if now.duration_since(*ts) < RAM_VOLT_CACHE_TTL {
                return *val;
            }
        }
        let val = read_ram_voltage();
        self.cached_ram_v = Some((now, val));
        val
    }

    fn read_cpu_voltage_cached(&mut self) -> Option<u32> {
        let now = Instant::now();
        if let Some((ts, val)) = &self.cached_cpu_v {
            if now.duration_since(*ts) < CPU_VOLT_CACHE_TTL {
                return *val;
            }
        }
        let val = read_cpu_voltage_wmi();
        self.cached_cpu_v = Some((now, val));
        val
    }
}

fn read_cpu_base_freq() -> u32 {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
    hklm.open_subkey_with_flags(path, KEY_READ)
        .ok()
        .and_then(|k| k.get_value("~MHz").ok())
        .unwrap_or(0)
}

fn read_cpu_clock_wmi(base_freq_mhz: u32) -> Option<u32> {
    if base_freq_mhz > 0 {
        let ps_cmd = "Get-CimInstance -ClassName Win32_PerfFormattedData_Counters_ProcessorInformation \
                      -Filter \"Name='_Total'\" | Select-Object -ExpandProperty PercentProcessorPerformance";
        if let Ok(output) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Ok(pct) = text.trim().parse::<f64>() {
                    if pct > 0.0 {
                        return Some((base_freq_mhz as f64 * pct / 100.0) as u32);
                    }
                }
            }
        }
    }
    let ps_cmd =
        "Get-CimInstance Win32_Processor | Select-Object -ExpandProperty CurrentClockSpeed";
    if let Ok(output) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Ok(mhz) = text.trim().parse::<u32>() {
                if mhz > 0 {
                    return Some(mhz);
                }
            }
        }
    }
    None
}

fn read_ram_voltage() -> Option<u32> {
    let ps_cmd = "$m = Get-CimInstance Win32_PhysicalMemory | Select-Object -First 1; \
                  if ($m.ConfiguredVoltage -gt 0) { $m.ConfiguredVoltage }";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mv = text.trim().parse::<u32>().ok().filter(|&mv| mv > 0)?;
    (800..=2500).contains(&mv).then_some(mv)
}

fn read_cpu_voltage_wmi() -> Option<u32> {
    let ps_cmd =
        "Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty CurrentVoltage";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let raw = text.trim().parse::<u32>().ok().filter(|&v| v > 0)?;
    let mv = if raw & 0x80 != 0 {
        (raw & 0x7F) * 1000
    } else if raw < 100 {
        raw * 100
    } else {
        raw
    };
    (600..=2000).contains(&mv).then_some(mv)
}

fn read_gpu_sensors() -> Vec<GpuSensors> {
    let mut by_name: HashMap<String, GpuSensors> = HashMap::new();

    for nv in read_nvidia_gpus_nvml() {
        by_name.insert(
            nv.name.clone(),
            GpuSensors {
                name: nv.name,
                utilization_pct: nv.utilization_pct,
                vram_used_mb: nv.vram_used_mb,
                vram_total_mb: nv.vram_total_mb,
                core_clock_mhz: nv.core_clock_mhz,
                memory_clock_mhz: nv.memory_clock_mhz,
                max_core_clock_mhz: None,
                max_memory_clock_mhz: None,
                fan_speed_pct: nv.fan_speed_pct,
                voltage_mv: None,
                voltage_source: None,
                temperature_c: nv.temperature_c,
                power_w: nv.power_w,
                temperature_source: nv.temperature_c.map(|_| SensorSource::Nvml.as_str().to_string()),
                power_source: nv.power_w.map(|_| SensorSource::Nvml.as_str().to_string()),
            },
        );
    }

    let smi_list = read_gpu_sensors_nvidia_smi().unwrap_or_default();
    for smi in smi_list {
        by_name
            .entry(smi.name.clone())
            .and_modify(|g| merge_gpu_smi(g, &smi))
            .or_insert(smi);
    }

    if by_name.is_empty() {
        return read_gpu_sensors_wmi();
    }

    let mut out: Vec<_> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn merge_gpu_smi(dst: &mut GpuSensors, smi: &GpuSensors) {
    if dst.utilization_pct.is_none() {
        dst.utilization_pct = smi.utilization_pct;
    }
    if dst.vram_used_mb.is_none() {
        dst.vram_used_mb = smi.vram_used_mb;
    }
    if dst.vram_total_mb.is_none() {
        dst.vram_total_mb = smi.vram_total_mb;
    }
    if dst.core_clock_mhz.is_none() {
        dst.core_clock_mhz = smi.core_clock_mhz;
    }
    if dst.memory_clock_mhz.is_none() {
        dst.memory_clock_mhz = smi.memory_clock_mhz;
    }
    if dst.fan_speed_pct.is_none() {
        dst.fan_speed_pct = smi.fan_speed_pct;
    }
    dst.max_core_clock_mhz = smi.max_core_clock_mhz.or(dst.max_core_clock_mhz);
    dst.max_memory_clock_mhz = smi.max_memory_clock_mhz.or(dst.max_memory_clock_mhz);
}

fn read_gpu_sensors_nvidia_smi() -> Option<Vec<GpuSensors>> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,clocks.current.graphics,clocks.current.memory,clocks.max.graphics,clocks.max.memory,power.draw,fan.speed",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 8 {
            continue;
        }
        let name = parts[0].to_string();
        if name.is_empty() {
            continue;
        }
        let power_w = parts.get(8).and_then(|s| parse_power_w(s));
        let fan_speed_pct = parts.get(9).and_then(|s| parse_percentage(s));
        out.push(GpuSensors {
            name,
            utilization_pct: parts[1].parse::<f64>().ok(),
            vram_used_mb: parts[2].parse::<u64>().ok(),
            vram_total_mb: parts[3].parse::<u64>().ok(),
            core_clock_mhz: parts[4].parse::<u32>().ok(),
            memory_clock_mhz: parts[5].parse::<u32>().ok(),
            max_core_clock_mhz: parts[6].parse::<u32>().ok(),
            max_memory_clock_mhz: parts[7].parse::<u32>().ok(),
            fan_speed_pct,
            voltage_mv: None,
            voltage_source: None,
            temperature_c: None,
            power_w,
            temperature_source: None,
            power_source: power_w.map(|_| SensorSource::NvidiaSmi.as_str().to_string()),
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_power_w(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("N/A") || t.eq_ignore_ascii_case("[N/A]") {
        return None;
    }
    t.parse::<f32>().ok().filter(|&v| v >= 0.0)
}

fn parse_percentage(s: &str) -> Option<u32> {
    let value = s.trim().trim_end_matches('%').trim();
    if value.is_empty() || value.eq_ignore_ascii_case("N/A") || value.eq_ignore_ascii_case("[N/A]") {
        return None;
    }
    value.parse::<u32>().ok().filter(|value| *value <= 100)
}

fn read_gpu_sensors_wmi() -> Vec<GpuSensors> {
    let ps_cmd = "Get-CimInstance Win32_VideoController \
      | Where-Object { $_.Name -and $_.Name -notmatch 'Microsoft|Basic' } \
      | Select-Object Name, AdapterRAM \
      | ConvertTo-Json -Compress";
    let Ok(output) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_cmd])
        .output()
    else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let arr = match val {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![val],
        _ => return vec![],
    };
    let mut out = Vec::new();
    for item in arr {
        let name = item
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let vram_total_mb = item
            .get("AdapterRAM")
            .and_then(|v| v.as_u64())
            .filter(|&b| b > 0)
            .map(|b| b / (1024 * 1024));
        out.push(GpuSensors {
            name,
            utilization_pct: None,
            vram_used_mb: None,
            vram_total_mb,
            core_clock_mhz: None,
            memory_clock_mhz: None,
            max_core_clock_mhz: None,
            max_memory_clock_mhz: None,
            fan_speed_pct: None,
            voltage_mv: None,
            voltage_source: None,
            temperature_c: None,
            power_w: None,
            temperature_source: None,
            power_source: None,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn check_whea_errors() -> WheaInfo {
    let output = std::process::Command::new("wevtutil")
        .args([
            "qe",
            "Microsoft-Windows-Kernel-WHEA/Operational",
            "/c:20",
            "/rd:true",
            "/f:text",
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let event_count = text.matches("Event[").count() as u32;
            WheaInfo {
                error_count: event_count,
                last_error: None,
                events: vec![],
            }
        }
        _ => WheaInfo {
            error_count: 0,
            last_error: None,
            events: vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::parse_percentage;

    #[test]
    fn percentage_parser_preserves_unavailable_instead_of_fabricating_zero() {
        assert_eq!(parse_percentage("37"), Some(37));
        assert_eq!(parse_percentage("37 %"), Some(37));
        assert_eq!(parse_percentage("N/A"), None);
        assert_eq!(parse_percentage("[N/A]"), None);
        assert_eq!(parse_percentage("101"), None);
    }
}
