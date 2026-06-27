use std::sync::{Arc, Mutex};

use nidavellir_core::ipc::{
    parse_request, serialize_response, DriverStatusPayload, IpcRequest, IpcResponse, ResponseData,
};
use tracing::{debug, warn};

use crate::AppState;
use crate::PIPE_NAME;
use nidavellir_driver_pawnio::DriverManager;

pub fn run_pipe_server(state: Arc<Mutex<AppState>>) -> Result<(), String> {
    loop {
        match serve_one_client(Arc::clone(&state)) {
            Ok(()) => debug!("Client disconnected"),
            Err(e) => {
                // Common on UI reload/close: broken pipe / pipe ended (0x8007006D).
                let is_broken_pipe =
                    e.contains("0x8007006D") || e.to_lowercase().contains("broken pipe");
                if is_broken_pipe {
                    debug!("Pipe client disconnected: {e}");
                } else {
                    warn!("Pipe client error: {e}");
                }
            }
        }
    }
}

fn serve_one_client(state: Arc<Mutex<AppState>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::io::{BufRead, BufReader};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };

        let pipe_name: Vec<u16> = PIPE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            CreateNamedPipeW(
                windows::core::PCWSTR(pipe_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            return Err("CreateNamedPipeW failed".into());
        }

        unsafe {
            ConnectNamedPipe(handle, None).map_err(|e| format!("ConnectNamedPipe failed: {e}"))?;
        }

        let mut reader = BufReader::new(PipeReader { handle });
        let mut line = String::new();
        while reader.read_line(&mut line).map_err(|e| e.to_string())? > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }
            let response = handle_request(trimmed, &state);
            let out = format!("{}\n", serialize_response(&response)?);
            write_pipe(handle, out.as_bytes())?;
            line.clear();
        }

        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err("Named pipe server requires Windows".into())
    }
}

#[cfg(windows)]
struct PipeReader {
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl std::io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows::Win32::Storage::FileSystem::ReadFile;

        let mut read: u32 = 0;
        let ok = unsafe { ReadFile(self.handle, Some(buf), Some(&mut read), None) };
        match ok {
            Ok(()) => {
                if read == 0 {
                    return Ok(0);
                }
                Ok(read as usize)
            }
            Err(e) => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                e.to_string(),
            )),
        }
    }
}

#[cfg(windows)]
fn write_pipe(handle: windows::Win32::Foundation::HANDLE, data: &[u8]) -> Result<(), String> {
    use windows::Win32::Storage::FileSystem::WriteFile;
    let mut written: u32 = 0;
    unsafe {
        WriteFile(handle, Some(data), Some(&mut written), None)
            .map_err(|e| format!("WriteFile failed: {e}"))?;
    }
    Ok(())
}

fn handle_request(line: &str, state: &Arc<Mutex<AppState>>) -> IpcResponse {
    let request = match parse_request(line) {
        Ok(r) => r,
        Err(e) => return IpcResponse::failure(e),
    };

    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(e) => return IpcResponse::failure(format!("State lock poisoned: {e}")),
    };

    match &request {
        IpcRequest::Ping => IpcResponse::success(ResponseData::Pong),
        IpcRequest::DetectHardware => {
            let mut hw = nidavellir_core::detect_hardware();
            refine_cpu_max_clock(&mut hw.cpu, &guard.driver);
            IpcResponse::success(ResponseData::Hardware(hw))
        }
        IpcRequest::ReadSensors => {
            let input = crate::sensor_gather::gather_sensor_input(
                &guard.driver,
                &guard.motherboard,
            );
            let sensors = guard.sensor_engine.read(&input);
            IpcResponse::success(ResponseData::Sensors(sensors))
        }
        IpcRequest::GetCapabilityReport => {
            let mut hw = nidavellir_core::detect_hardware();
            refine_cpu_max_clock(&mut hw.cpu, &guard.driver);
            let report = nidavellir_core::build_capability_report(&hw);
            IpcResponse::success(ResponseData::Capability(report))
        }
        IpcRequest::GetDriverStatus => {
            let status = guard.driver.status();
            IpcResponse::success(ResponseData::DriverStatus(DriverStatusPayload {
                status: status.code().to_string(),
                detail: status.detail(),
            }))
        }
        IpcRequest::GetSafeLoopStatus => {
            let status = crate::safe_loop_runtime::status_snapshot(&guard.safe_store);
            IpcResponse::success(ResponseData::SafeLoop(status))
        }
        IpcRequest::GetGpuCurve => {
            IpcResponse::success(ResponseData::GpuCurve(crate::gpu_real::read_curve_snapshot()))
        }
        IpcRequest::StartGpuValidation => {
            if guard.gpu_validation.start() {
                IpcResponse::success(ResponseData::GpuValidation(guard.gpu_validation.status()))
            } else {
                IpcResponse::failure("GPU validation already running")
            }
        }
        IpcRequest::GetGpuValidation => {
            IpcResponse::success(ResponseData::GpuValidation(guard.gpu_validation.status()))
        }
        IpcRequest::StartRealSweep => {
            let store = guard.safe_store.clone();
            if guard.real_sweep.start(store, crate::gpu_sweep_real::Quality::thorough()) {
                IpcResponse::success(ResponseData::GpuSweep(guard.real_sweep.progress()))
            } else {
                IpcResponse::failure("Real sweep already running")
            }
        }
        IpcRequest::StartRealSweepFast => {
            let store = guard.safe_store.clone();
            if guard.real_sweep.start(store, crate::gpu_sweep_real::Quality::fast()) {
                IpcResponse::success(ResponseData::GpuSweep(guard.real_sweep.progress()))
            } else {
                IpcResponse::failure("Real sweep already running")
            }
        }
        IpcRequest::StopRealSweep => {
            guard.real_sweep.stop();
            IpcResponse::success(ResponseData::GpuSweep(guard.real_sweep.progress()))
        }
        IpcRequest::GetRealSweepProgress => {
            IpcResponse::success(ResponseData::GpuSweep(guard.real_sweep.progress()))
        }
        IpcRequest::StartMemSweep => {
            let store = guard.safe_store.clone();
            if guard.mem_sweep.start(store) {
                IpcResponse::success(ResponseData::MemSweep(guard.mem_sweep.progress()))
            } else {
                IpcResponse::failure("Memory sweep already running")
            }
        }
        IpcRequest::StopMemSweep => {
            guard.mem_sweep.stop();
            IpcResponse::success(ResponseData::MemSweep(guard.mem_sweep.progress()))
        }
        IpcRequest::GetMemSweepProgress => {
            IpcResponse::success(ResponseData::MemSweep(guard.mem_sweep.progress()))
        }
        IpcRequest::ApplyGodforge | IpcRequest::ApplyBrokkrs | IpcRequest::ApplyDeepCalm => {
            let profiles = guard.real_sweep.progress().profiles;
            let chosen = profiles.as_ref().map(|p| match &request {
                IpcRequest::ApplyBrokkrs => (&p.brokkrs_best.name, p.brokkrs_best.point),
                IpcRequest::ApplyDeepCalm => (&p.deep_calm.name, p.deep_calm.point),
                _ => (&p.godforge.name, p.godforge.point),
            });
            match chosen {
                Some((name, point)) => {
                    let mut ap = crate::gpu_apply::load_applied().unwrap_or_default();
                    ap.label = name.clone();
                    ap.core = Some(point);
                    let msg = match crate::gpu_apply::apply_and_persist(
                        ap.label.clone(), ap.core, ap.mem_offset_mhz, &guard.safe_store,
                    ) {
                        Ok(()) => format!("Applied {} ({} MHz @ {} mV)", name, point.freq_mhz, point.voltage_mv),
                        Err(e) => format!("Apply failed: {e}"),
                    };
                    IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
                }
                None => IpcResponse::failure("Run the core sweep first"),
            }
        }
        IpcRequest::ApplyMemPeak => {
            let peak = guard.mem_sweep.progress().peak_offset_mhz;
            if peak <= 0 {
                IpcResponse::failure("Run the memory sweep first")
            } else {
                let mut ap = crate::gpu_apply::load_applied().unwrap_or_default();
                ap.mem_offset_mhz = Some(peak);
                if ap.label.is_empty() {
                    ap.label = "Custom".into();
                }
                let msg = match crate::gpu_apply::apply_and_persist(
                    ap.label.clone(), ap.core, ap.mem_offset_mhz, &guard.safe_store,
                ) {
                    Ok(()) => format!("Applied memory +{peak} MHz"),
                    Err(e) => format!("Apply failed: {e}"),
                };
                IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
            }
        }
        IpcRequest::ResetGpuTuning => {
            let msg = match crate::gpu_apply::reset(&guard.safe_store) {
                Ok(()) => "Reset to stock".to_string(),
                Err(e) => format!("Reset failed: {e}"),
            };
            IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
        }
        IpcRequest::GetAppliedProfile => {
            IpcResponse::success(ResponseData::GpuApply(applied_status(String::new())))
        }
        IpcRequest::VerifyAppliedProfile => {
            // Read-only: classifies the live modern VF curve vs the applied profile.
            // Never applies, reapplies, or mutates GPU state.
            IpcResponse::success(ResponseData::ApplyVerification(
                crate::gpu_verify::verify_applied_curve(),
            ))
        }
        IpcRequest::StartForgeAll => {
            let store = guard.safe_store.clone();
            if guard.forge_all.start(store) {
                IpcResponse::success(ResponseData::ForgeAll(guard.forge_all.progress()))
            } else {
                IpcResponse::failure("Forge-all already running")
            }
        }
        IpcRequest::StopForgeAll => {
            guard.forge_all.stop();
            IpcResponse::success(ResponseData::ForgeAll(guard.forge_all.progress()))
        }
        IpcRequest::GetForgeAllProgress => {
            IpcResponse::success(ResponseData::ForgeAll(guard.forge_all.progress()))
        }
        IpcRequest::StartBenchmark => {
            let store = guard.safe_store.clone();
            if guard.benchmark.start(store) {
                IpcResponse::success(ResponseData::Benchmark(guard.benchmark.progress()))
            } else {
                IpcResponse::failure("Benchmark already running")
            }
        }
        IpcRequest::StopBenchmark => {
            guard.benchmark.stop();
            IpcResponse::success(ResponseData::Benchmark(guard.benchmark.progress()))
        }
        IpcRequest::GetBenchmarkProgress => {
            IpcResponse::success(ResponseData::Benchmark(guard.benchmark.progress()))
        }
        IpcRequest::StartPowerSweep => {
            let store = guard.safe_store.clone();
            if guard.power_sweep.start(store) {
                IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
            } else {
                IpcResponse::failure("Power sweep already running")
            }
        }
        IpcRequest::StartPowerSweepFast => {
            let store = guard.safe_store.clone();
            if guard
                .power_sweep
                .start_with_mode(store, crate::gpu_power_sweep::PowerSweepMode::Fast)
            {
                IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
            } else {
                IpcResponse::failure("Power sweep already running")
            }
        }
        IpcRequest::StartPowerSweepLong => {
            let store = guard.safe_store.clone();
            if guard
                .power_sweep
                .start_with_mode(store, crate::gpu_power_sweep::PowerSweepMode::Long)
            {
                IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
            } else {
                IpcResponse::failure("Power sweep already running")
            }
        }
        IpcRequest::StopPowerSweep => {
            guard.power_sweep.stop();
            IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
        }
        IpcRequest::GetPowerSweepProgress => {
            IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
        }
        IpcRequest::ApplyPowerGodforge => {
            let prog = guard.power_sweep.progress();
            if let Some(r) = refuse_undervolt_apply(&prog) {
                r
            } else {
                apply_power_profile(&guard.safe_store, prog.godforge, "Godforge")
            }
        }
        IpcRequest::ApplyPowerBrokkrs => {
            let prog = guard.power_sweep.progress();
            if let Some(r) = refuse_undervolt_apply(&prog) {
                r
            } else {
                apply_power_profile(&guard.safe_store, prog.brokkrs, "Brokkr's Best")
            }
        }
        IpcRequest::ApplyPowerDeepCalm => {
            let prog = guard.power_sweep.progress();
            if let Some(r) = refuse_undervolt_apply(&prog) {
                r
            } else {
                apply_power_profile(&guard.safe_store, prog.deep_calm, "Deep Calm")
            }
        }
    }
}

/// Fail-closed apply GATE for F2 anchored-undervolt forge results. The Apply path writes an F1
/// flatten-down ceiling (`apply_core` → `apply_vf_ceiling`), which is the WRONG operation for an F2
/// undervolt point (F2 RAISES a lower-voltage bin; F1 caps frequency down). Until the dedicated F2 apply
/// is wired (Phase 2), REFUSE the apply when the active forge produced an F2 undervolt profile. Returns
/// `Some(refusal)` to refuse, or `None` to fall through to the legacy F1 apply (default — backward
/// compatible: a non-undervolt or legacy/restored payload has `is_undervolt = false`).
fn refuse_undervolt_apply(prog: &nidavellir_core::ipc::PowerSweepProgress) -> Option<IpcResponse> {
    if prog.is_undervolt {
        Some(IpcResponse::failure(
            "F2 undervolt apply not yet wired (Phase 2) — profile discovered but not applicable",
        ))
    } else {
        None
    }
}

/// Apply a power-sweep profile point (core voltage + clock, with the hard clock
/// cap) and persist it, keeping any existing memory offset.
fn apply_power_profile(
    store: &nidavellir_core::safe_loop::SafeLoopStore,
    pt: Option<nidavellir_core::ipc::PowerSweepPoint>,
    label: &str,
) -> IpcResponse {
    let Some(p) = pt else {
        return IpcResponse::failure("Run the power sweep first (no point for this profile)");
    };
    let mut ap = crate::gpu_apply::load_applied().unwrap_or_default();
    ap.core = Some(nidavellir_core::gpu_sweep::VfPoint {
        freq_mhz: p.clock_mhz,
        voltage_mv: p.voltage_mv,
    });
    ap.label = label.into();
    let msg = match crate::gpu_apply::apply_and_persist(ap.label.clone(), ap.core, ap.mem_offset_mhz, store) {
        Ok(()) => format!("Applied {label}: {} MHz @ {} mV", p.clock_mhz, p.voltage_mv),
        Err(e) => format!("Apply failed: {e}"),
    };
    IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
}

/// Build the apply-status payload from the persisted profile.
fn applied_status(message: String) -> nidavellir_core::ipc::GpuApplyStatus {
    let ap = crate::gpu_apply::load_applied().unwrap_or_default();
    nidavellir_core::ipc::GpuApplyStatus {
        label: if ap.label.is_empty() { None } else { Some(ap.label) },
        core: ap.core,
        mem_offset_mhz: ap.mem_offset_mhz,
        message,
    }
}

/// Replace the CPUID/WMI base-clock fallback with the real factory max turbo,
/// read from the silicon via MSR when the PawnIO driver is available.
///
/// Windows only exposes the base/rated clock (e.g. 3400 MHz on an i7-13700K).
/// The actual turbo ceiling lives in MSR_TURBO_RATIO_LIMIT (0x1AD); we fall
/// back to IA32_HWP_CAPABILITIES (0x771). Intel-only: AMD encodes ratios
/// differently (COF), so we leave its value untouched.
fn refine_cpu_max_clock(
    cpu: &mut nidavellir_core::detector::CpuInfo,
    driver: &DriverManager,
) {
    use nidavellir_core::msr;

    if cpu.vendor != "Intel" {
        return;
    }

    let to_core = |m: nidavellir_driver_pawnio::MsrValue| msr::MsrValue {
        eax: m.eax,
        edx: m.edx,
    };

    let ratio = driver
        .read_msr(msr::MSR_TURBO_RATIO_LIMIT)
        .ok()
        .and_then(|m| msr::max_turbo_ratio_from_turbo_limit(to_core(m)))
        .or_else(|| {
            driver
                .read_msr(msr::IA32_HWP_CAPABILITIES)
                .ok()
                .and_then(|m| msr::highest_perf_ratio_from_hwp(to_core(m)))
        });

    if let Some(ratio) = ratio {
        let mhz = msr::turbo_ratio_to_mhz(ratio);
        // Only override when it actually beats the base reading — a bogus MSR
        // read should never make the reported max worse.
        if mhz > cpu.base_freq_mhz {
            cpu.max_freq_mhz = mhz;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidavellir_core::ipc::PowerSweepProgress;

    #[test]
    fn apply_gate_refuses_f2_undervolt_profiles() {
        // An F2 undervolt forge result must REFUSE the F1 ceiling apply (Phase 2 wires the real F2
        // apply). The refusal carries the agreed message so the UI can explain it.
        let prog = PowerSweepProgress { is_undervolt: true, ..Default::default() };
        let r = refuse_undervolt_apply(&prog).expect("F2 undervolt apply must be refused");
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("F2 undervolt apply not yet wired (Phase 2) — profile discovered but not applicable")
        );
    }

    #[test]
    fn apply_gate_passes_through_legacy_f1_profiles() {
        // Backward-compatible: a non-undervolt (legacy F1 / restored) payload defaults `is_undervolt`
        // to false → the gate falls through (`None`) to the unchanged F1 apply path.
        let prog = PowerSweepProgress::default();
        assert!(!prog.is_undervolt, "default must keep the legacy F1 apply behavior");
        assert!(refuse_undervolt_apply(&prog).is_none());
    }
}
