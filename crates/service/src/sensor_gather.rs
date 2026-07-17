use nidavellir_core::detector::MotherboardInfo;
use nidavellir_core::sensor_input::SensorInput;
use nidavellir_core::sensor_meta::{SensorQuality, SensorSource};
use nidavellir_core::superio_profile;
use nidavellir_driver_pawnio::DriverManager;

pub fn gather_sensor_input(driver: &DriverManager, motherboard: &MotherboardInfo) -> SensorInput {
    let probe = driver.probe_superio();
    let resolved = superio_profile::resolve_superio(motherboard, probe.as_ref());
    let cpu_temp_c = driver.read_cpu_temperature_c();

    let mut input = SensorInput::from_driver_parts(
        motherboard.clone(),
        resolved,
        cpu_temp_c,
        cpu_temp_c.is_some(),
    );

    if input.cpu_vcore_mv.is_none() {
        if let Some(mv) = driver.read_vcore_intel_mv() {
            if (400..=2500).contains(&mv) {
                input.cpu_vcore_mv = Some(mv);
                input.cpu_vcore_source = Some(SensorSource::Msr);
                input.cpu_vcore_quality = SensorQuality::Nominal;
            }
        }
    }

    if let Some(mv) = nidavellir_gpu_nvapi::read_core_voltage_mv()
        .filter(|mv| (400..=1500).contains(mv))
    {
        input.gpu_voltage_mv = Some(mv);
        input.gpu_voltage_source = Some(SensorSource::Nvapi);
    }

    input
}
