use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MsrValue {
    pub eax: u32,
    pub edx: u32,
}

pub const IA32_PERF_STATUS: u32 = 0x198;

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
}
