//! Intel CPU temperature from MSRs (IA32_THERM_STATUS + IA32_TEMPERATURE_TARGET).

use crate::msr::MsrValue;

pub const IA32_THERM_STATUS: u32 = 0x19C;
pub const IA32_TEMPERATURE_TARGET: u32 = 0x1A2;
pub const IA32_PACKAGE_THERM_STATUS: u32 = 0x1B1;

/// Core temperature in °C from digital readout (best-effort).
pub fn core_temp_c_from_msrs(status: MsrValue, target: MsrValue) -> Option<f32> {
    let digital_readout = (status.eax >> 16) & 0x7F;
    let tj_max = (target.eax >> 16) & 0xFF;
    if tj_max == 0 {
        return None;
    }
    let temp = (tj_max as f32) - (digital_readout as f32);
    if temp > 0.0 && temp < 125.0 {
        Some(temp)
    } else {
        None
    }
}

/// Package temperature from MSR 0x1B1 (when available).
pub fn package_temp_c_from_msr(status: MsrValue) -> Option<f32> {
    let digital_readout = (status.eax >> 16) & 0x7F;
    let tj_max = (status.eax >> 8) & 0xFF;
    if tj_max == 0 {
        return None;
    }
    let temp = (tj_max as f32) - (digital_readout as f32);
    if temp > 0.0 && temp < 125.0 {
        Some(temp)
    } else {
        None
    }
}
