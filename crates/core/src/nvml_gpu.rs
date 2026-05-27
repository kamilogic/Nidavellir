//! NVIDIA GPU sensors via NVML (in-process; complements nvidia-smi).

use crate::sensor_meta::{SensorQuality, SensorSource};

#[derive(Debug, Clone)]
pub struct NvmlGpuReading {
    pub index: u32,
    pub name: String,
    pub utilization_pct: Option<f64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    pub core_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub source: SensorSource,
    pub quality: SensorQuality,
}

pub fn read_nvidia_gpus_nvml() -> Vec<NvmlGpuReading> {
    let nvml = match nvml_wrapper::Nvml::init() {
        Ok(n) => n,
        Err(_) => return vec![],
    };

    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    for i in 0..count {
        let Ok(device) = nvml.device_by_index(i) else {
            continue;
        };
        let name = device.name().unwrap_or_else(|_| format!("NVIDIA GPU {i}"));
        let util = device
            .utilization_rates()
            .ok()
            .map(|u| u.gpu as f64);
        let mem = device.memory_info().ok();
        let vram_used_mb = mem.as_ref().map(|m| m.used / (1024 * 1024));
        let vram_total_mb = mem.as_ref().map(|m| m.total / (1024 * 1024));
        let core_clock_mhz = device
            .clock_info(nvml_wrapper::enum_wrappers::device::Clock::Graphics)
            .ok();
        let memory_clock_mhz = device
            .clock_info(nvml_wrapper::enum_wrappers::device::Clock::Memory)
            .ok();
        let temperature_c = device
            .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32);
        let power_w = device.power_usage().ok().map(|mw| mw as f32 / 1000.0);

        out.push(NvmlGpuReading {
            index: i,
            name,
            utilization_pct: util,
            vram_used_mb,
            vram_total_mb,
            core_clock_mhz,
            memory_clock_mhz,
            temperature_c,
            power_w,
            source: SensorSource::Nvml,
            quality: SensorQuality::Live,
        });
    }
    out
}
