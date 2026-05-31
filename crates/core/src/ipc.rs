use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::CapabilityReport;
use crate::detector::HardwareInfo;
use crate::gpu_sweep::{GpuSweepProgress, StabilityResult, VfPoint};
use crate::safe_loop::{BlacklistRegion, CrashClass, SafeLoopState, TuningPoint};
use crate::sensors::SensorReadings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum IpcRequest {
    Ping,
    DetectHardware,
    ReadSensors,
    GetCapabilityReport,
    GetDriverStatus,
    GetSafeLoopStatus,
    StartGpuSweep,
    StopGpuSweep,
    GetGpuSweepProgress,
    GetGpuCurve,
    StartGpuValidation,
    GetGpuValidation,
}

/// The live V/F curve read from the GPU via NVAPI (the same data Afterburner's
/// curve editor shows). `plateau` is where a flat-curve undervolt has locked
/// the top clock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuCurveSnapshot {
    pub name: String,
    pub points: Vec<VfPoint>,
    pub plateau: Option<VfPoint>,
    /// True when read from real hardware (vs unavailable/simulated).
    pub real: bool,
}

/// Result of a real GPU compute-validation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuValidationStatus {
    pub running: bool,
    pub result: Option<StabilityResult>,
    pub mismatches: u32,
    pub elapsed_ms: u64,
    pub adapter: Option<String>,
}

/// Read-only snapshot of the Safe Loop for the UI's "Segurança" view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeLoopStatus {
    pub state: SafeLoopState,
    pub safe_mode: bool,
    pub consecutive_crashes: u32,
    pub crash_threshold: u32,
    pub boot_flag_armed: bool,
    pub last_validated: Option<TuningPoint>,
    pub blacklist: Vec<BlacklistRegion>,
    pub recent_crashes: Vec<CrashClass>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverStatusPayload {
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseData {
    Pong,
    Hardware(HardwareInfo),
    Sensors(SensorReadings),
    Capability(CapabilityReport),
    DriverStatus(DriverStatusPayload),
    SafeLoop(SafeLoopStatus),
    GpuSweep(GpuSweepProgress),
    GpuCurve(GpuCurveSnapshot),
    GpuValidation(GpuValidationStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
}

impl IpcResponse {
    pub fn success(data: ResponseData) -> Self {
        Self {
            ok: true,
            error: None,
            data: Some(data),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            data: None,
        }
    }
}

pub fn parse_request(line: &str) -> Result<IpcRequest, String> {
    serde_json::from_str(line).map_err(|e| format!("Invalid request JSON: {e}"))
}

pub fn serialize_response(response: &IpcResponse) -> Result<String, String> {
    serde_json::to_string(response).map_err(|e| format!("Failed to serialize response: {e}"))
}

pub fn response_to_value(response: &IpcResponse) -> Value {
    serde_json::to_value(response).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ping() {
        let req = parse_request(r#"{"method":"Ping"}"#).unwrap();
        assert!(matches!(req, IpcRequest::Ping));
        let resp = IpcResponse::success(ResponseData::Pong);
        let json = serialize_response(&resp).unwrap();
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"type\":\"Pong\""));
    }
}
