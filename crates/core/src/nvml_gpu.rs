//! NVIDIA GPU sensors via NVML (in-process; complements nvidia-smi).

use crate::sensor_meta::{SensorQuality, SensorSource};

#[derive(Debug, Clone)]
pub struct NvmlGpuReading {
    pub index: u32,
    pub name: String,
    /// Stable physical-device identity used to isolate learned tuning data between identical models.
    pub uuid: Option<String>,
    pub utilization_pct: Option<f64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    pub core_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    /// Enforced power limit (W) — the cap the card throttles against.
    pub power_limit_w: Option<f32>,
    /// True if the GPU is currently throttling because it hit the power cap
    /// (SW_POWER_CAP) — the key signal that an undervolt can reclaim headroom.
    pub power_capped: Option<bool>,
    /// True when NVML reports software or hardware thermal slowdown.
    pub thermal_throttled: Option<bool>,
    pub source: SensorSource,
    pub quality: SensorQuality,
}

/// Hard-cap the GPU core (graphics) clock at `max_mhz` via NVML locked clocks,
/// so the boost curve is **flat after the validated limit**: the GPU can never
/// clock above the point we proved stable, regardless of how much voltage is
/// available. `min` is left low so it still downclocks at idle. Driver-level and
/// independent of the V/F curve — the reliable way to flatten the top end.
pub fn lock_core_clock_max_mhz(max_mhz: u32) -> Result<(), String> {
    use nvml_wrapper::enums::device::GpuLockedClocksSetting;
    let nvml = nvml_wrapper::Nvml::init().map_err(|e| format!("NVML init: {e}"))?;
    let mut device = nvml.device_by_index(0).map_err(|e| format!("NVML device: {e}"))?;
    device
        .set_gpu_locked_clocks(GpuLockedClocksSetting::Numeric {
            min_clock_mhz: 210,
            max_clock_mhz: max_mhz,
        })
        .map_err(|e| format!("set_gpu_locked_clocks: {e}"))
}

/// PIN the core (graphics) clock to exactly `mhz` (min = max). Combined with a
/// voltage lock this forces a fixed V/F operating point — the real undervolt
/// test: hold the clock, drop the voltage, find the lowest that's stable.
pub fn pin_core_clock_mhz(mhz: u32) -> Result<(), String> {
    use nvml_wrapper::enums::device::GpuLockedClocksSetting;
    let nvml = nvml_wrapper::Nvml::init().map_err(|e| format!("NVML init: {e}"))?;
    let mut device = nvml.device_by_index(0).map_err(|e| format!("NVML device: {e}"))?;
    device
        .set_gpu_locked_clocks(GpuLockedClocksSetting::Numeric { min_clock_mhz: mhz, max_clock_mhz: mhz })
        .map_err(|e| format!("set_gpu_locked_clocks(pin): {e}"))
}

/// Release the core clock cap (back to the stock boost ceiling).
pub fn reset_core_clock_lock() -> Result<(), String> {
    let nvml = nvml_wrapper::Nvml::init().map_err(|e| format!("NVML init: {e}"))?;
    let mut device = nvml.device_by_index(0).map_err(|e| format!("NVML device: {e}"))?;
    device
        .reset_gpu_locked_clocks()
        .map_err(|e| format!("reset_gpu_locked_clocks: {e}"))
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
        let uuid = device.uuid().ok();
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
        let power_limit_w = device.enforced_power_limit().ok().map(|mw| mw as f32 / 1000.0);
        let throttle_reasons = device.current_throttle_reasons().ok();
        let power_capped = throttle_reasons.as_ref().map(|r| {
            r.contains(nvml_wrapper::bitmasks::device::ThrottleReasons::SW_POWER_CAP)
        });
        let thermal_throttled = throttle_reasons.as_ref().map(|r| {
            r.intersects(
                nvml_wrapper::bitmasks::device::ThrottleReasons::SW_THERMAL_SLOWDOWN
                    | nvml_wrapper::bitmasks::device::ThrottleReasons::HW_THERMAL_SLOWDOWN,
            )
        });

        out.push(NvmlGpuReading {
            index: i,
            name,
            uuid,
            utilization_pct: util,
            vram_used_mb,
            vram_total_mb,
            core_clock_mhz,
            memory_clock_mhz,
            temperature_c,
            power_w,
            power_limit_w,
            power_capped,
            thermal_throttled,
            source: SensorSource::Nvml,
            quality: SensorQuality::Live,
        });
    }
    out
}
