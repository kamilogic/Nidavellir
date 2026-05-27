use crate::detector::MotherboardInfo;
use crate::sensor_meta::{SensorQuality, SensorSource};
use crate::superio_profile::ResolvedSuperIo;

#[derive(Debug, Clone)]
pub struct SensorInput {
    pub motherboard: MotherboardInfo,
    pub cpu_vcore_mv: Option<u32>,
    pub cpu_vcore_source: Option<SensorSource>,
    pub cpu_vcore_quality: SensorQuality,
    pub cpu_temp_c: Option<f32>,
    pub cpu_temp_source: Option<SensorSource>,
    pub dram_mv: Option<u32>,
    pub dram_source: Option<SensorSource>,
    pub dram_quality: SensorQuality,
    pub superio: Option<ResolvedSuperIo>,
}

impl SensorInput {
    pub fn from_driver_parts(
        motherboard: MotherboardInfo,
        superio_resolved: Option<crate::superio_profile::ResolvedRails>,
        cpu_temp_c: Option<f32>,
        cpu_temp_from_msr: bool,
    ) -> Self {
        let mut input = Self {
            motherboard,
            cpu_vcore_mv: None,
            cpu_vcore_source: None,
            cpu_vcore_quality: SensorQuality::Unavailable,
            cpu_temp_c: None,
            cpu_temp_source: None,
            dram_mv: None,
            dram_source: None,
            dram_quality: SensorQuality::Unavailable,
            superio: None,
        };

        if let Some(r) = superio_resolved {
            input.superio = Some(r.meta);
            if let Some(mv) = r.cpu_vcore_mv {
                input.cpu_vcore_mv = Some(mv);
                input.cpu_vcore_source = Some(SensorSource::SuperIo);
                input.cpu_vcore_quality = SensorQuality::Live;
            }
            if let Some(mv) = r.dram_mv {
                input.dram_mv = Some(mv);
                input.dram_source = Some(SensorSource::SuperIo);
                input.dram_quality = SensorQuality::Live;
            }
        }

        if let Some(t) = cpu_temp_c {
            input.cpu_temp_c = Some(t);
            input.cpu_temp_source = if cpu_temp_from_msr {
                Some(SensorSource::Msr)
            } else {
                Some(SensorSource::Unknown)
            };
        }

        input
    }
}
