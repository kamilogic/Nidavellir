use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::CapabilityReport;
use crate::detector::HardwareInfo;
use crate::gpu_sweep::{GpuSweepProgress, StabilityResult, SweepPhase, VfPoint};
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
    GetGpuCurve,
    StartGpuValidation,
    GetGpuValidation,
    StartRealSweep,
    StartRealSweepFast,
    StopRealSweep,
    GetRealSweepProgress,
    StartMemSweep,
    StopMemSweep,
    GetMemSweepProgress,
    ApplyGodforge,
    ApplyBrokkrs,
    ApplyDeepCalm,
    ApplyMemPeak,
    ResetGpuTuning,
    GetAppliedProfile,
}

/// The GPU profile currently applied/persisted, plus the last action's message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuApplyStatus {
    pub label: Option<String>,
    pub core: Option<VfPoint>,
    pub mem_offset_mhz: Option<i32>,
    pub message: String,
}

/// One step of the memory sweep: a clock offset with its measured effective
/// bandwidth and integrity verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemSweepPoint {
    pub offset_mhz: i32,
    pub mem_mhz: u32,
    pub bandwidth_gbps: f32,
    #[serde(default)]
    pub min_gbps: f32,
    pub stable: bool,
}

/// Memory sweep that finds the GDDR6 effective-bandwidth peak (not max clock).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemSweepProgress {
    pub phase: SweepPhase,
    pub running: bool,
    pub current_offset_mhz: i32,
    pub current_mem_mhz: u32,
    pub current_gbps: f32,
    pub baseline_gbps: f32,
    pub points: Vec<MemSweepPoint>,
    pub peak_offset_mhz: i32,
    pub peak_gbps: f32,
    #[serde(default)]
    pub validation_note: Option<String>,
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

/// One completed stage of the validation battery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuStageResult {
    pub name: String,
    pub result: StabilityResult,
    pub mismatches: u32,
    pub elapsed_ms: u64,
}

/// Live status of the real GPU compute-validation battery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuValidationStatus {
    pub running: bool,
    /// Name of the stage currently running (when `running`).
    pub current_stage: Option<String>,
    pub stage_index: u32,
    pub total_stages: u32,
    /// Stages completed so far.
    pub stages: Vec<GpuStageResult>,
    /// Overall verdict once finished (worst of all stages).
    pub result: Option<StabilityResult>,
    pub adapter: Option<String>,
    pub error: Option<String>,
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
    MemSweep(MemSweepProgress),
    GpuApply(GpuApplyStatus),
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
