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
    VerifyAppliedProfile,
    StartForgeAll,
    StopForgeAll,
    GetForgeAllProgress,
    StartBenchmark,
    StopBenchmark,
    GetBenchmarkProgress,
    StartPowerSweep,
    StopPowerSweep,
    GetPowerSweepProgress,
    ApplyPowerGodforge,
    ApplyPowerBrokkrs,
    ApplyPowerDeepCalm,
}

/// Telemetry confidence for a dwell metric, from how many valid samples backed it.
/// Serializes as "high"/"medium"/"low"/"unavailable". Descriptive only — used to
/// explain measurement strength in the UI/logs, never to fail a tuning decision
/// unless that decision actually depends on the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DwellQuality {
    High,
    Medium,
    Low,
    Unavailable,
}

/// One measured (locked voltage → sustained clock + power) point of the sweep.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PowerSweepPoint {
    /// LEGACY / display: the measured dwell voltage (a sparse NVAPI sensor max).
    /// Kept for backward compat + existing selection/logging. It is **measured
    /// telemetry, NOT a deterministic apply key** — for that use
    /// `vf_table_voltage_mv`. Equals `measured_voltage_mv` for points produced
    /// after the voltage split.
    pub voltage_mv: u32,
    pub clock_mhz: u32,
    /// Clock offset (MHz) that realizes this point — applied WITHOUT a hard
    /// voltage lock (curve flatten), so the card keeps its power management.
    pub offset_mhz: i32,
    /// Mean sustained power (W) under the max-power load.
    pub power_w: f32,
    /// Peak sampled power (W) — the spike headroom indicator.
    pub max_power_w: f32,
    /// Std-dev of power (W) — workload spikiness, for the Brokkr's headroom calc.
    pub power_std_w: f32,
    /// Fraction of samples (0–1) the card was power-capped (SW_POWER_CAP).
    pub power_capped_frac: f32,
    pub stable: bool,
    /// Efficiency proxy: sustained clock per watt.
    pub perf_per_watt: f64,
    /// Measured effective voltage under the dwell (telemetry only — same source as
    /// `voltage_mv`). Descriptive; never an apply/frontier key. `None` for points
    /// produced before the voltage split (see `decisions.md`).
    #[serde(default)]
    pub measured_voltage_mv: Option<u32>,
    /// Deterministic VF-table bin voltage this point snaps to — the apply/frontier
    /// key. `None` for legacy points produced before the split. (Added: voltage split.)
    #[serde(default)]
    pub vf_table_voltage_mv: Option<u32>,

    // ── Richer dwell stats (all additive/optional; `None` on legacy points) ──
    /// Lowest sustained clock (MHz) over the dwell — a clock that dips well below
    /// the mean is not truly sustained (matters for Godforge / F1b).
    #[serde(default)]
    pub min_clock_mhz: Option<u32>,
    /// 5th-percentile sustained clock (MHz) — the "bad-case" clock.
    #[serde(default)]
    pub p5_clock_mhz: Option<u32>,
    /// Ramp-filtered + sanity-checked measured-voltage distribution (telemetry only).
    #[serde(default)]
    pub avg_measured_voltage_mv: Option<u32>,
    #[serde(default)]
    pub min_measured_voltage_mv: Option<u32>,
    #[serde(default)]
    pub max_measured_voltage_mv: Option<u32>,
    /// How many valid (post-ramp, in-range) voltage samples backed the stats above.
    #[serde(default)]
    pub voltage_sample_count: Option<u32>,
    /// Confidence in the measured-voltage stats (sparse voltage sampling → Medium/Low).
    #[serde(default)]
    pub voltage_quality: Option<DwellQuality>,
    /// Post-ramp clock/power sample count and the dwell duration (ms).
    #[serde(default)]
    pub dwell_sample_count: Option<u32>,
    #[serde(default)]
    pub dwell_duration_ms: Option<u64>,
    /// GPU temperature at the start/end of the steady-state window and its mean (°C).
    #[serde(default)]
    pub start_temp_c: Option<f32>,
    #[serde(default)]
    pub end_temp_c: Option<f32>,
    #[serde(default)]
    pub avg_temp_c: Option<f32>,
    /// Overall dwell telemetry confidence (worst of clock/power/voltage quality).
    #[serde(default)]
    pub telemetry_quality: Option<DwellQuality>,
}

/// Power-target sweep: for a range of locked voltages, the max stable clock and
/// the sustained power it draws under a heavy load — used to find the perf/watt
/// knee (best performance just before diminishing returns) under the power cap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PowerSweepProgress {
    pub running: bool,
    pub phase: String,
    pub log: Vec<String>,
    pub points: Vec<PowerSweepPoint>,
    /// Enforced power cap (W).
    pub power_limit_w: f32,
    /// Target power (W) — defaults to the perf/watt knee's draw; configurable.
    pub target_w: f32,
    /// The recommended operating point (Deep Calm — the perf/watt knee).
    pub recommended: Option<PowerSweepPoint>,
    /// Max performance, up to full power for stability.
    pub godforge: Option<PowerSweepPoint>,
    /// Max performance the undervolt holds under the cap (stays off the cap).
    pub brokkrs: Option<PowerSweepPoint>,
    /// Best perf/watt with clock ≥ stock baseline.
    pub deep_calm: Option<PowerSweepPoint>,
    /// Stock baseline sustained clock (MHz) under the same load, for reference.
    pub stock_clock_mhz: u32,
    pub note: Option<String>,
}

/// One benchmark run's measured metrics (stock or tuned).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSnapshot {
    pub fps: f64,
    pub bandwidth_gbps: f64,
    pub avg_clock_mhz: u32,
    pub avg_power_w: f32,
    pub max_temp_c: f32,
    /// Performance per watt (FPS ÷ average watts).
    pub perf_per_watt: f64,
    /// Fraction of samples (0–1) where the card was power-capped (SW_POWER_CAP).
    pub power_capped_frac: f32,
}

/// Before/after benchmark progress + report (stock vs applied profile).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchmarkProgress {
    pub running: bool,
    pub phase: String,
    pub log: Vec<String>,
    pub stock: Option<BenchSnapshot>,
    pub tuned: Option<BenchSnapshot>,
    pub power_limit_w: f32,
    pub note: Option<String>,
}

/// Live status of the full auto pipeline (VRAM gate → core → memory → soak).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeAllProgress {
    pub running: bool,
    pub phase: String,
    pub log: Vec<String>,
    pub note: Option<String>,
}

/// The GPU profile currently applied/persisted, plus the last action's message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuApplyStatus {
    pub label: Option<String>,
    pub core: Option<VfPoint>,
    pub mem_offset_mhz: Option<i32>,
    pub message: String,
}

/// Read-only verification of whether the live modern VF curve matches the applied
/// Nidavellir profile (Patch A: curve-only — no telemetry/load classification yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveVerification {
    /// No applied profile recorded — nothing to verify.
    NotApplicable,
    /// A profile is recorded but the live curve was not / could not be checked
    /// against it (e.g. a memory-only profile with no core point).
    MetadataOnly,
    /// The live modern VF curve shows the expected flattening at/above the
    /// deterministic ceiling bin (within tolerance).
    VerifiedCurve,
    /// The live modern VF curve does NOT show the expected flattening.
    LiveMismatch,
    /// Verification could not be completed reliably (no modern API, empty readback,
    /// unmappable bin, …).
    VerificationFailed,
}

/// Structured result of a read-only applied-curve verification. Telemetry/load and
/// workload-context fields are intentionally absent (later patches B/C).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyVerificationStatus {
    pub status: CurveVerification,
    pub label: Option<String>,
    pub target_mhz: Option<u32>,
    /// Deterministic VF-table ceiling bin used for comparison — re-derived the same
    /// way apply derives its key. NOT the measured voltage.
    pub vf_table_voltage_mv: Option<u32>,
    /// Legacy/measured voltage from the applied profile, for diagnostics only.
    pub legacy_voltage_mv: Option<u32>,
    /// Expected-flattened plateau points and how many matched the target within tol.
    pub matched_points: Option<u32>,
    pub expected_points: Option<u32>,
    /// True only when `status == VerifiedCurve` (structured; UI must not parse message).
    pub live_curve_match: bool,
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
    /// True when the modern per-point V/F curve API (Afterburner-style elastic
    /// ceiling) is usable on this GPU + driver — drives UI messaging.
    #[serde(default)]
    pub vf_curve_supported: bool,
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
    ApplyVerification(ApplyVerificationStatus),
    ForgeAll(ForgeAllProgress),
    Benchmark(BenchmarkProgress),
    PowerSweep(PowerSweepProgress),
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
