use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MsrValue {
    pub eax: u32,
    pub edx: u32,
}

pub const IA32_PERF_STATUS: u32 = 0x198;
/// Intel MSR_TURBO_RATIO_LIMIT — per-core-count max turbo ratios (one byte each).
pub const MSR_TURBO_RATIO_LIMIT: u32 = 0x1AD;
/// Intel IA32_HWP_CAPABILITIES — bits 7:0 hold the guaranteed "Highest Performance" ratio.
pub const IA32_HWP_CAPABILITIES: u32 = 0x771;

/// Reference bus clock for modern Intel desktop parts: ratio * 100 MHz = core clock.
pub const INTEL_BCLK_MHZ: u32 = 100;

/// Extract the maximum single-core turbo ratio from MSR_TURBO_RATIO_LIMIT.
///
/// The MSR packs up to eight per-core-count ratios (bits 0-7 = 1 active core,
/// 8-15 = 2 cores, …). The 1-core entry is the highest; we take the max of the
/// non-zero bytes to stay robust across layouts.
pub fn max_turbo_ratio_from_turbo_limit(value: MsrValue) -> Option<u32> {
    let raw = ((value.edx as u64) << 32) | value.eax as u64;
    let max = (0..8)
        .map(|byte| ((raw >> (byte * 8)) & 0xFF) as u32)
        .filter(|&r| r > 0)
        .max()?;
    if max > 0 {
        Some(max)
    } else {
        None
    }
}

/// Extract the "Highest Performance" ratio (bits 7:0) from IA32_HWP_CAPABILITIES.
pub fn highest_perf_ratio_from_hwp(value: MsrValue) -> Option<u32> {
    let ratio = value.eax & 0xFF;
    if ratio > 0 {
        Some(ratio)
    } else {
        None
    }
}

/// Convert an Intel turbo ratio to a core clock in MHz (ratio * 100 MHz BCLK).
pub fn turbo_ratio_to_mhz(ratio: u32) -> u32 {
    ratio * INTEL_BCLK_MHZ
}

/// Pack OC mailbox voltage offset (mV) into IA32_OC_MAILBOX format (bits vary by platform).
pub fn pack_oc_mailbox_voltage_offset_mv(offset_mv: i32) -> u32 {
    let clamped = offset_mv.clamp(-500, 500);
    (clamped as u32) & 0xFFFF
}

/// Extract VID from IA32_PERF_STATUS (bits 0-12) for legacy Intel Vcore approximation.
pub fn extract_perf_status_vid(value: MsrValue) -> u32 {
    value.eax & 0x1FFF
}

/// Approximate Vcore in mV from VID (pre-Haswell formula; documented as approximate).
pub fn vid_to_vcore_mv_pre_haswell(vid: u32) -> Option<u32> {
    if vid == 0 {
        return None;
    }
    Some((vid as f64 * 5.0 + 245.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_clamps_voltage_offset() {
        assert_eq!(pack_oc_mailbox_voltage_offset_mv(-600), 65036);
        assert_eq!(pack_oc_mailbox_voltage_offset_mv(600), 500);
        assert_eq!(pack_oc_mailbox_voltage_offset_mv(-50), 65486);
    }

    #[test]
    fn extract_vid_masks_correct_bits() {
        let val = MsrValue {
            eax: 0x0000005F,
            edx: 0,
        };
        assert_eq!(extract_perf_status_vid(val), 0x05F);
    }

    #[test]
    fn vid_to_vcore_rejects_zero() {
        assert!(vid_to_vcore_mv_pre_haswell(0).is_none());
        assert_eq!(vid_to_vcore_mv_pre_haswell(100), Some(745));
    }

    #[test]
    fn turbo_limit_picks_highest_ratio() {
        // i7-13700K style: 1-core=54, 2-core=54, then lower bins.
        // bytes (low->high): 0x36 0x36 0x35 0x35 0x34 0x34 0x33 0x33
        let raw: u64 = 0x3333_3434_3535_3636;
        let val = MsrValue {
            eax: raw as u32,
            edx: (raw >> 32) as u32,
        };
        assert_eq!(max_turbo_ratio_from_turbo_limit(val), Some(0x36));
        assert_eq!(turbo_ratio_to_mhz(0x36), 5400);
    }

    #[test]
    fn turbo_limit_rejects_all_zero() {
        let val = MsrValue { eax: 0, edx: 0 };
        assert!(max_turbo_ratio_from_turbo_limit(val).is_none());
    }

    #[test]
    fn hwp_extracts_low_byte() {
        let val = MsrValue {
            eax: 0x00FF_0036,
            edx: 0,
        };
        assert_eq!(highest_perf_ratio_from_hwp(val), Some(0x36));
        assert!(highest_perf_ratio_from_hwp(MsrValue { eax: 0, edx: 0 }).is_none());
    }
}
