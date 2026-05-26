use crate::driver::{self, MsrValue};
use crate::DriverState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningParams {
    pub cpu_voltage_offset_mv: i32,
    pub cpu_cache_offset_mv: i32,
    pub pl1_watts: u32,
    pub pl2_watts: u32,
    pub turbo_ratio_limit: u32,
    pub c_states_enabled: bool,
    pub gpu_core_offset_mhz: i32,
    pub gpu_mem_offset_mhz: i32,
    pub gpu_power_limit_pct: u8,
}

impl Default for TuningParams {
    fn default() -> Self {
        Self {
            cpu_voltage_offset_mv: 0,
            cpu_cache_offset_mv: 0,
            pl1_watts: 95,
            pl2_watts: 125,
            turbo_ratio_limit: 50,
            c_states_enabled: true,
            gpu_core_offset_mhz: 0,
            gpu_mem_offset_mhz: 0,
            gpu_power_limit_pct: 100,
        }
    }
}

pub fn apply_tuning(params: &TuningParams, driver: &DriverState) -> Result<(), String> {
    let dm = driver.0.lock().map_err(|e| e.to_string())?;
    if !dm.is_loaded() {
        return Err("Kernel driver not available".into());
    }

    if params.pl1_watts != 95 || params.pl2_watts != 125 {
        set_power_limits(&dm, params.pl1_watts, params.pl2_watts)?;
    }

    if params.turbo_ratio_limit != 50 {
        set_turbo_ratio(&dm, params.turbo_ratio_limit)?;
    }

    if !params.c_states_enabled {
        set_c_states(&dm, false)?;
    }

    Ok(())
}

pub fn reset_tuning(driver: &DriverState) -> Result<(), String> {
    let dm = driver.0.lock().map_err(|e| e.to_string())?;
    if !dm.is_loaded() {
        return Err("Kernel driver not available".into());
    }

    reset_power_limits(&dm)?;
    reset_turbo_ratio(&dm)?;
    set_c_states(&dm, true)?;
    Ok(())
}

fn set_power_limits(dm: &crate::driver::DriverManager, pl1: u32, pl2: u32) -> Result<(), String> {
    let current: MsrValue = dm.read_msr(driver::IA32_PACKAGE_POWER_LIMIT)?;

    let pl1_power = (pl1 as f64 / 0.125) as u32;
    let pl2_power = (pl2 as f64 / 0.125) as u32;
    let pl1_time: u32 = 28;
    let pl2_time: u32 = 28;
    let enable_bit: u32 = 1;

    let new_value: u64 = (pl1_power as u64)
        | ((pl1_time as u64) << 17)
        | ((enable_bit as u64) << 15)
        | ((pl2_power as u64) << 32)
        | ((pl2_time as u64) << 49)
        | ((enable_bit as u64) << 47);

    let eax = (new_value & 0xFFFF_FFFF) as u32;
    let edx = (new_value >> 32) as u32;

    if current.eax != eax || current.edx != edx {
        let _ = current;
        dm.write_msr(driver::MSR_PKG_CST_CONFIG_CONTROL, eax, edx)?;
    }

    // Actually write to power limit MSR
    dm.write_msr(driver::IA32_PACKAGE_POWER_LIMIT, eax, edx)
}

fn reset_power_limits(dm: &crate::driver::DriverManager) -> Result<(), String> {
    let pl1_power: u32 = (95.0 / 0.125) as u32;
    let pl2_power: u32 = (125.0 / 0.125) as u32;
    let pl1_time: u32 = 28;
    let pl2_time: u32 = 28;
    let enable_bit: u32 = 1;

    let new_value: u64 = (pl1_power as u64)
        | ((pl1_time as u64) << 17)
        | ((enable_bit as u64) << 15)
        | ((pl2_power as u64) << 32)
        | ((pl2_time as u64) << 49)
        | ((enable_bit as u64) << 47);

    let eax = (new_value & 0xFFFF_FFFF) as u32;
    let edx = (new_value >> 32) as u32;
    dm.write_msr(driver::IA32_PACKAGE_POWER_LIMIT, eax, edx)
}

fn set_turbo_ratio(dm: &crate::driver::DriverManager, ratio: u32) -> Result<(), String> {
    let v = (ratio as u64) * 0x01010101_01010101;
    let eax = (v & 0xFFFF_FFFF) as u32;
    let edx = (v >> 32) as u32;
    dm.write_msr(driver::MSR_TURBO_RATIO_LIMIT, eax, edx)
}

fn reset_turbo_ratio(dm: &crate::driver::DriverManager) -> Result<(), String> {
    set_turbo_ratio(dm, 50)
}

fn set_c_states(dm: &crate::driver::DriverManager, enabled: bool) -> Result<(), String> {
    let current = dm.read_msr(driver::MSR_PKG_CST_CONFIG_CONTROL)?;
    let eax = if enabled { current.eax & !1 } else { current.eax | 1 };
    dm.write_msr(driver::MSR_PKG_CST_CONFIG_CONTROL, eax, current.edx)
}
