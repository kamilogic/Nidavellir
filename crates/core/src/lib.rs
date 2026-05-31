pub mod capability;
pub mod detector;
pub mod fingerprint;
pub mod ipc;
pub mod msr;
pub mod msr_temp;
pub mod nvml_gpu;
pub mod safe_loop;
pub mod sensor_input;
pub mod sensor_meta;
pub mod sensors;
pub mod superio_profile;

pub use capability::{build_capability_report, CapabilityBucket, CapabilityItem, CapabilityReport};
pub use detector::{detect_hardware, HardwareInfo};
pub use fingerprint::{compute_fingerprint, MachineFingerprint};
pub use ipc::{IpcRequest, IpcResponse};
pub use safe_loop::{
    decide_recovery, BootFlag, RecoveryAction, SafeLoopRecord, SafeLoopState, SafeLoopStore,
    TuningPoint,
};
pub use sensor_input::SensorInput;
pub use sensors::{read_sensors, SensorReadings};
pub use superio_profile::{resolve_superio, SuperIoProbe, SuperIoVendor, VIN_RAW_LEN};
