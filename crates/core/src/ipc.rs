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
    /// Deep reset: same emergency recovery as [`IpcRequest::ResetGpuTuning`] (stock GPU, Safe Loop
    /// latch released, run checkpoint cleared) but ALSO discards all learning — the Safe Loop
    /// blacklist, the F2 observation frontier, and legacy knowledge. Additive; the UI offers it as a
    /// separate, stronger-confirmation "forget everything" control.
    ResetGpuTuningFull,
    GetAppliedProfile,
    VerifyAppliedProfile,
    StartForgeAll,
    StopForgeAll,
    GetForgeAllProgress,
    StartBenchmark,
    StopBenchmark,
    GetBenchmarkProgress,
    StartPowerSweep,
    /// Fast-evidence variant of `StartPowerSweep`: the same complete F2 frontier with shorter dwell
    /// validation at every tested point. Additive — `StartPowerSweep` keeps its behavior.
    StartPowerSweepFast,
    /// High-confidence variant of `StartPowerSweep`: the same complete F2 frontier with longer
    /// dwells and independent repeated validations. Additive.
    StartPowerSweepLong,
    StopPowerSweep,
    GetPowerSweepProgress,
    ApplyPowerGodforge,
    ApplyPowerBrokkrs,
    ApplyPowerDeepCalm,
    /// Write a rich, human-readable log of the latest/current F2 forge run — run metadata, contract
    /// versions, the published profiles, the frontier summary, the live progress log, and every
    /// recorded dwell (clock/voltage/power/temp/outcome/pattern) — to a timestamped file under the
    /// data dir. Read-only: gathers persisted observations + the live progress; touches no hardware.
    ExportForgeLog,
}

/// Result of [`IpcRequest::ExportForgeLog`]: where the rich log was written and how much it covered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeLogExport {
    /// Absolute path of the written human-readable log file.
    pub path: String,
    /// Absolute path of the raw append-only observation JSONL (machine-readable companion).
    pub raw_observations_path: String,
    /// Size of the written log file in bytes.
    pub bytes: u64,
    /// Number of dwell observations included.
    pub observation_count: usize,
    /// One-line human summary (for a toast / status line).
    pub note: String,
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
    /// Sustained high-power percentile (W) used by F2 frontier decisions and profile calibration.
    /// `None` for legacy/F1 points and observations that predate discovery contract v3.
    #[serde(default)]
    pub power_p99_w: Option<f32>,
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
    /// Learned voltage-margin boundary before the application safety margin is added. For F2 points,
    /// `vf_table_voltage_mv` is the exact physical bin Apply will use; this field preserves the
    /// evidence boundary so the policy shift remains visible and auditable.
    #[serde(default)]
    pub boundary_voltage_mv: Option<u32>,
    /// Effective upward application margin after snapping to a real VF-table bin.
    #[serde(default)]
    pub apply_margin_mv: Option<u32>,
    /// Apply bin BEFORE any regime lift (boundary + standard margin). Regime requirements are
    /// computed from this pre-lift value so lifting one point can never cascade requirements
    /// through the frontier. `None` on legacy points (falls back to `vf_table_voltage_mv`).
    #[serde(default)]
    pub base_apply_mv: Option<u32>,

    // ── Richer dwell stats (all additive/optional; `None` on legacy points) ──
    /// Lowest sustained clock (MHz) over the dwell — a clock that dips well below
    /// the mean is not truly sustained (matters for Godforge / F1b).
    #[serde(default)]
    pub min_clock_mhz: Option<u32>,
    /// 5th-percentile sustained clock (MHz) — the "bad-case" clock.
    #[serde(default)]
    pub p5_clock_mhz: Option<u32>,
    /// 95th-percentile sustained clock (MHz) — the upper sustained boost regime observed during
    /// the same dwell. Kept separate from the configured target and the p5 stability floor.
    #[serde(default)]
    pub p95_clock_mhz: Option<u32>,
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
    /// GPU temperature at the start/end of the steady-state window plus mean/max (°C).
    #[serde(default)]
    pub start_temp_c: Option<f32>,
    #[serde(default)]
    pub end_temp_c: Option<f32>,
    #[serde(default)]
    pub avg_temp_c: Option<f32>,
    #[serde(default)]
    pub max_temp_c: Option<f32>,
    /// Whether NVML reported software or hardware thermal slowdown during the dwell.
    #[serde(default)]
    pub thermal_throttled: bool,
    /// Overall dwell telemetry confidence (worst of clock/power/voltage quality).
    #[serde(default)]
    pub telemetry_quality: Option<DwellQuality>,
    /// The TARGET clock (MHz) this point was probed at in the F1b multi-clock frontier
    /// (vs `clock_mhz` = the measured ACHIEVED clock, which may differ). `None` for
    /// single-clock / legacy points. Additive, backward-compatible — no schema bump.
    /// (Added: F1b Phase 2B.1.)
    #[serde(default)]
    pub target_clock_mhz: Option<u32>,
    /// Structured stability confidence for this exact point (0–1). The producer defines the
    /// confidence model; `None` keeps legacy payloads backward-compatible.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Successful confirmations accumulated at this exact target/anchor point.
    #[serde(default)]
    pub validation_count: Option<u32>,
    /// True only when FSGL3 A+B qualified this exact `(target_clock_mhz,
    /// vf_table_voltage_mv)` Apply pair after the application margin was added.
    #[serde(default)]
    pub apply_qualified: bool,
    /// Qualification contract that produced `apply_qualified`. Old/restored points default to
    /// `None` and remain ineligible for F2 Apply.
    #[serde(default)]
    pub apply_qualification_version: Option<u32>,
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
    /// True when the forged profiles came from the F2 ANCHORED UNDERVOLT path
    /// (a lower-voltage operating point), NOT the F1 flatten-down ceiling. The
    /// Apply IPC writes an F1 ceiling, which is the WRONG operation for an F2
    /// undervolt point, so it REFUSES to apply when this is set (Phase 2 wires
    /// the real F2 apply). Additive + backward-compatible: `default = false`
    /// preserves the legacy F1 apply behavior for every existing payload.
    #[serde(default)]
    pub is_undervolt: bool,
    /// Structured synthesis result. `true` means the measured frontier could not honestly
    /// differentiate the profiles because it remained power-bound.
    #[serde(default)]
    pub power_bound_collapse: bool,
    /// Structured live F2 progress. The total is an estimate because cross-clock pruning and the
    /// final 90%-of-Cmax floor become exact only as the frontier is learned.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub current_clock_mhz: Option<u32>,
    #[serde(default)]
    pub current_voltage_mv: Option<u32>,
    #[serde(default)]
    pub completed_steps: u32,
    #[serde(default)]
    pub total_steps_estimate: u32,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub estimated_remaining_ms: Option<u64>,
    /// Conservative estimated wall time from run start through completion. Unlike
    /// `estimated_remaining_ms`, this is an absolute total duration and may tighten as Cmax,
    /// frontier pruning, calibration gaps and final Apply pairs become known.
    #[serde(default)]
    pub estimated_total_upper_ms: Option<u64>,
    /// First reset-clean sustainable real clock found by the current F2 run.
    #[serde(default)]
    pub cmax_clock_mhz: Option<u32>,
    /// Lowest real clock included by the current run's inclusive 90%-of-Cmax domain.
    #[serde(default)]
    pub frontier_floor_clock_mhz: Option<u32>,
    /// Number of real clocks in the inclusive Cmax-to-floor domain.
    #[serde(default)]
    pub frontier_clock_count: Option<u32>,
    #[serde(default)]
    pub learned_points: u32,
    #[serde(default)]
    pub last_outcome: Option<String>,
    /// True once every completed candidate has been appended to durable F2 observation storage.
    #[serde(default)]
    pub learning_saved: bool,
    /// True only after the full Cmax→90% frontier completed and definitive profiles were synthesized.
    #[serde(default)]
    pub frontier_complete: bool,
    /// True only when the discovered frontier and selected profiles passed the mode's independent
    /// qualification requirements. Fast intentionally leaves this false and is preview-only.
    #[serde(default)]
    pub profiles_qualified: bool,
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

/// Load-state axis (Patch B): whether the applied profile's EXISTING synthetic-dwell
/// stats support a load-level verification claim. Orthogonal to `CurveVerification`.
/// Derived from stored dwell stats — does NOT run a new stress test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadVerification {
    /// No load classification attempted (curve not verified, or no dwell stats).
    NotEvaluated,
    /// Curve verified AND the applied point's dwell stats support it under synthetic load.
    VerifiedUnderLoad,
    /// Dwell stats exist but are too weak to claim VerifiedUnderLoad (low quality / no p5).
    TelemetryInsufficient,
    /// Dwell stats exist but contradict the profile (clock dipped, or not a stable point).
    LoadMismatch,
    /// Reserved for live real-workload context (e.g. unfocused/desktop 1062 mV). NOT
    /// produced by Patch B (which uses only synthetic-dwell stats).
    WorkloadStateMismatch,
    /// Load classification hit invalid/implausible data.
    LoadVerificationFailed,
}

/// Structured result of a read-only applied-profile verification. `status` is the
/// CURVE axis (curve-state); `load_state` is the LOAD axis (Patch B). Live real-game
/// workload-context classification is still future work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyVerificationStatus {
    /// Curve-state axis (unchanged from Patch A).
    pub status: CurveVerification,
    pub label: Option<String>,
    pub target_mhz: Option<u32>,
    /// Deterministic VF-table ceiling bin used for comparison — re-derived the same
    /// way apply derives its key. NOT the measured voltage.
    pub vf_table_voltage_mv: Option<u32>,
    /// Legacy/measured voltage from the applied profile, for diagnostics only.
    pub legacy_voltage_mv: Option<u32>,
    /// Expected-flattened plateau points and how many carry the flatten offset
    /// (offset readback is the primary criterion; GetStatus freq is diagnostic only).
    pub matched_points: Option<u32>,
    pub expected_points: Option<u32>,
    /// True only when `status == VerifiedCurve` (structured; UI must not parse message).
    pub live_curve_match: bool,
    pub message: String,

    // ── Load axis (Patch B; additive). `load_state == VerifiedUnderLoad` upgrades a
    //    VerifiedCurve to "verified under load". Absent load data never downgrades. ──
    #[serde(default)]
    pub load_state: Option<LoadVerification>,
    #[serde(default)]
    pub load_reason: Option<String>,
    /// `Some(true)` when load_state == VerifiedUnderLoad; structured for the UI.
    #[serde(default)]
    pub telemetry_match: Option<bool>,
    /// Dwell stats of the matched applied point (diagnostic context).
    #[serde(default)]
    pub p5_clock_mhz: Option<u32>,
    #[serde(default)]
    pub min_clock_mhz: Option<u32>,
    #[serde(default)]
    pub avg_measured_voltage_mv: Option<u32>,
    #[serde(default)]
    pub min_measured_voltage_mv: Option<u32>,
    #[serde(default)]
    pub max_measured_voltage_mv: Option<u32>,
    #[serde(default)]
    pub voltage_sample_count: Option<u32>,
    #[serde(default)]
    pub voltage_quality: Option<DwellQuality>,
    #[serde(default)]
    pub telemetry_quality: Option<DwellQuality>,

    // ── Read-only live diagnostic (Patch 11C; additive). ALL optional + serde-default,
    //    backward-compatible. NONE of these affect `status`/classification. The `live_*`
    //    snapshot is telemetry only (a single read, NOT load verification) and does NOT
    //    imply a hard voltage cap — measured voltage may sit ABOVE the VF/curve anchor. ──
    /// Index of the first (lowest-voltage) plateau bin carrying a non-zero flatten offset.
    #[serde(default)]
    pub first_modified_bin: Option<u32>,
    /// VF-table voltage (mV) of that first modified bin.
    #[serde(default)]
    pub first_modified_mv: Option<u32>,
    /// How many of the expected plateau bins carry a non-zero offset.
    #[serde(default)]
    pub modified_bin_count: Option<u32>,
    /// How many bins were expected to be flattened (points at/above the anchor).
    #[serde(default)]
    pub expected_bin_count: Option<u32>,
    /// GetStatus diagnostic: plateau points whose actual freq is within tolerance of target.
    #[serde(default)]
    pub getstatus_freq_match_count: Option<u32>,
    /// GetStatus observed plateau frequency spread (MHz) over the expected bins.
    #[serde(default)]
    pub getstatus_plateau_min_mhz: Option<u32>,
    #[serde(default)]
    pub getstatus_plateau_max_mhz: Option<u32>,
    /// Max GetStatus plateau freq above target (MHz); `Some(0)` when flat at/below target.
    #[serde(default)]
    pub max_target_overshoot_mhz: Option<i32>,
    /// Max GetStatus plateau freq below target (MHz); `Some(0)` when flat at/above target.
    #[serde(default)]
    pub max_target_undershoot_mhz: Option<i32>,
    /// Representative offset samples (kHz) — first-modified, anchor bin, highest-voltage bin.
    #[serde(default)]
    pub first_modified_offset_khz: Option<i32>,
    #[serde(default)]
    pub anchor_offset_khz: Option<i32>,
    #[serde(default)]
    pub highest_bin_offset_khz: Option<i32>,
    /// Single read-only live telemetry snapshot (telemetry only; may be unavailable → None,
    /// never a fake zero). Captured at verification time, NOT a sustained-load measurement.
    #[serde(default)]
    pub live_voltage_mv: Option<u32>,
    #[serde(default)]
    pub live_clock_mhz: Option<u32>,
    #[serde(default)]
    pub live_power_w: Option<f32>,
    #[serde(default)]
    pub live_utilization_pct: Option<f32>,
    #[serde(default)]
    pub live_temperature_c: Option<f32>,
    #[serde(default)]
    pub live_power_limit_w: Option<f32>,
    #[serde(default)]
    pub live_power_capped: Option<bool>,
    /// Compact human-readable diagnostic note (UI must not parse it for logic).
    #[serde(default)]
    pub diagnostic_message: Option<String>,
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
#[allow(clippy::large_enum_variant)] // IPC wire variants stay inline for backward-compatible serde shape.
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
    ForgeLogExport(ForgeLogExport),
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

    #[test]
    fn power_sweep_point_target_clock_roundtrips() {
        let p = PowerSweepPoint {
            target_clock_mhz: Some(1830),
            confidence: Some(0.92),
            validation_count: Some(4),
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PowerSweepPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back.target_clock_mhz, Some(1830));
        assert_eq!(back.confidence, Some(0.92));
        assert_eq!(back.validation_count, Some(4));
    }

    #[test]
    fn legacy_power_sweep_point_json_loads_target_clock_none() {
        // A payload produced before Phase 2B.1 has no `target_clock_mhz` key → defaults None.
        let legacy = r#"{
            "voltage_mv": 843, "clock_mhz": 1785, "offset_mhz": 150,
            "power_w": 180.0, "max_power_w": 185.0, "power_std_w": 2.0,
            "power_capped_frac": 0.2, "stable": true, "perf_per_watt": 9.9
        }"#;
        let p: PowerSweepPoint = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.target_clock_mhz, None);
        assert_eq!(p.confidence, None);
        assert_eq!(p.validation_count, None);
        assert_eq!(p.power_p99_w, None);
        assert_eq!(p.clock_mhz, 1785);
    }

    #[test]
    fn power_sweep_progress_is_undervolt_roundtrips() {
        let p = PowerSweepProgress {
            is_undervolt: true,
            profiles_qualified: true,
            estimated_total_upper_ms: Some(7_200_000),
            cmax_clock_mhz: Some(1935),
            frontier_floor_clock_mhz: Some(1755),
            frontier_clock_count: Some(13),
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PowerSweepProgress = serde_json::from_str(&json).unwrap();
        assert!(back.is_undervolt);
        assert!(back.profiles_qualified);
        assert!(!back.power_bound_collapse);
        assert_eq!(back.estimated_total_upper_ms, Some(7_200_000));
        assert_eq!(back.cmax_clock_mhz, Some(1935));
        assert_eq!(back.frontier_floor_clock_mhz, Some(1755));
        assert_eq!(back.frontier_clock_count, Some(13));
    }

    #[test]
    fn legacy_power_sweep_progress_json_defaults_is_undervolt_false() {
        // A payload produced before the F2 pivot has no `is_undervolt` key → defaults false, so the
        // Apply gate falls through to the unchanged legacy F1 apply behavior (backward-compatible).
        let legacy = r#"{
            "running": false, "phase": "done", "log": [], "points": [],
            "power_limit_w": 200.0, "target_w": 180.0,
            "recommended": null, "godforge": null, "brokkrs": null, "deep_calm": null,
            "stock_clock_mhz": 1800, "note": "ok"
        }"#;
        let p: PowerSweepProgress = serde_json::from_str(legacy).unwrap();
        assert!(!p.is_undervolt, "missing key must default to legacy F1 behavior");
        assert!(!p.profiles_qualified, "missing key must never unlock F2 Apply");
        assert!(!p.power_bound_collapse, "missing key must default to no structured collapse");
        assert_eq!(p.estimated_total_upper_ms, None);
        assert_eq!(p.cmax_clock_mhz, None);
        assert_eq!(p.frontier_floor_clock_mhz, None);
        assert_eq!(p.frontier_clock_count, None);
        assert_eq!(p.stock_clock_mhz, 1800);
    }
}
