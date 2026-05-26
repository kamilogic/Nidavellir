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

pub fn apply_tuning(_params: &TuningParams) -> Result<(), String> {
    Err("Not implemented".into())
}

pub fn reset_tuning() -> Result<(), String> {
    Err("Not implemented".into())
}
