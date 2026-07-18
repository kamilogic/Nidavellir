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
        use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED};
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
            if let Err(e) = ConnectNamedPipe(handle, None) {
                // ERROR_PIPE_CONNECTED (0x80070217): the client connected between
                // CreateNamedPipeW and ConnectNamedPipe — a documented SUCCESS case; the pipe is
                // usable. Treating it as fatal abandoned the instance WITHOUT closing the handle,
                // leaving the connected UI client waiting forever on a pipe nobody serves (the
                // frozen-UI symptom). Any other connect error must close the handle before
                // returning, or the instance leaks the same way.
                if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                    let _ = CloseHandle(handle);
                    return Err(format!("ConnectNamedPipe failed: {e}"));
                }
            }
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

    if gpu_write_requires_idle(&request) && gpu_operation_running(&guard) {
        return IpcResponse::failure(
            "Another GPU operation owns the service-wide tuning lease; stop it before starting or applying another operation",
        );
    }

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
        IpcRequest::AcknowledgeForgeIncident => {
            match crate::safe_loop_runtime::acknowledge_forge_incident(&guard.safe_store) {
                Ok(_) => IpcResponse::success(ResponseData::SafeLoop(
                    crate::safe_loop_runtime::status_snapshot(&guard.safe_store),
                )),
                Err(e) => IpcResponse::failure(e),
            }
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
            // Reset is the emergency recovery path after a TDR/interrupted forge. It must remain
            // available even if a worker is still marked running, so it is intentionally outside the
            // service-wide start/apply lease. Best-effort stop first; reset then clears Safe Loop.
            guard.real_sweep.stop();
            guard.mem_sweep.stop();
            guard.forge_all.stop();
            guard.benchmark.stop();
            guard.power_sweep.abort();
            let msg = match crate::gpu_apply::reset(&guard.safe_store) {
                Ok(()) => {
                    guard.power_sweep.recover_after_reset(
                        "Reset concluído; GPU em stock e Safe Loop desarmado. O checkpoint e a sequência da Forge foram preservados.",
                    );
                    "Reset to stock; Forge checkpoint preserved".to_string()
                }
                Err(e) => format!("Reset failed: {e}"),
            };
            IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
        }
        IpcRequest::ResetGpuTuningFull => {
            // Deep active-learning reset. Same emergency recovery as ResetGpuTuning (outside the
            // start/apply lease), but additionally wipes the Safe Loop working blacklist (by
            // replacing the record with the default), the F2 observation frontier, and legacy
            // knowledge. The durable condemnation ledger remains hardware-derived field evidence;
            // the UI arms the next Forge as Clean Run so that ledger does not steer the experiment.
            // Hardware → stock and the latch are handled by gpu_apply::reset first.
            guard.real_sweep.stop();
            guard.mem_sweep.stop();
            guard.forge_all.stop();
            guard.benchmark.stop();
            guard.power_sweep.abort();
            let msg = match crate::gpu_apply::reset(&guard.safe_store) {
                Ok(()) => {
                    let mut problems: Vec<String> = Vec::new();
                    // Replace the whole Safe Loop record with the default — this is what additionally
                    // drops the blacklist that the latch-only reset preserves.
                    if let Err(e) = guard
                        .safe_store
                        .save_record(&nidavellir_core::safe_loop::SafeLoopRecord::default())
                    {
                        problems.push(format!("safe loop record: {e}"));
                    }
                    problems.extend(crate::gpu_apply::clear_all_learning());
                    if let Err(e) = crate::gpu_power_sweep::clear_persisted_forge_state() {
                        problems.push(format!("forge checkpoint: {e}"));
                    }
                    // The sentinel's persisted history (baseline, status card, event log) is part of
                    // the learned state — wipe it too so a full reset leaves nothing inconsistent with
                    // the now-empty blacklist.
                    problems.extend(crate::tdr_sentinel::reset_sentinel_state());
                    guard.power_sweep.forget_after_full_reset(
                        "Reset completo concluído; GPU em stock, aprendizado ativo apagado e condenações reais duráveis preservadas.",
                    );
                    if problems.is_empty() {
                        "Full reset to stock; active learning cleared and durable real-world condemnations preserved"
                            .to_string()
                    } else {
                        format!("Full reset to stock; some state could not be cleared: {}", problems.join("; "))
                    }
                }
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
                IpcResponse::failure(power_sweep_start_failure(&guard.safe_store))
            }
        }
        IpcRequest::StartPowerSweepClean => {
            let store = guard.safe_store.clone();
            if guard.power_sweep.start_clean_run(store) {
                IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
            } else {
                IpcResponse::failure(power_sweep_start_failure(&guard.safe_store))
            }
        }
        IpcRequest::StartPowerSweepFast => {
            // Backward-compatible wire alias only. Fast no longer exists as a Forge behavior;
            // an older UI therefore receives the same bounded, fully qualified Standard run.
            let store = guard.safe_store.clone();
            if guard
                .power_sweep
                .start_with_mode(store, crate::gpu_power_sweep::PowerSweepMode::Standard)
            {
                IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
            } else {
                IpcResponse::failure(power_sweep_start_failure(&guard.safe_store))
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
                IpcResponse::failure(power_sweep_start_failure(&guard.safe_store))
            }
        }
        IpcRequest::StopPowerSweep => {
            guard.power_sweep.stop();
            IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
        }
        IpcRequest::ResumePowerSweep => {
            let store = guard.safe_store.clone();
            match guard.power_sweep.resume(store) {
                Ok(progress) => IpcResponse::success(ResponseData::PowerSweep(progress)),
                Err(e) => IpcResponse::failure(format!("Forge resume refused: {e}")),
            }
        }
        IpcRequest::GetPowerSweepProgress => {
            IpcResponse::success(ResponseData::PowerSweep(guard.power_sweep.progress()))
        }
        IpcRequest::ApplyPowerGodforge => {
            let prog = guard.power_sweep.progress();
            apply_forge_profile(&guard.safe_store, &prog, prog.godforge, "Godforge")
        }
        IpcRequest::ApplyPowerBrokkrs => {
            let prog = guard.power_sweep.progress();
            apply_forge_profile(&guard.safe_store, &prog, prog.brokkrs, "Brokkr's Best")
        }
        IpcRequest::ApplyPowerDeepCalm => {
            let prog = guard.power_sweep.progress();
            apply_forge_profile(&guard.safe_store, &prog, prog.deep_calm, "Deep Calm")
        }
        IpcRequest::ReportPowerGodforgeUnstable
        | IpcRequest::ReportPowerBrokkrsUnstable
        | IpcRequest::ReportPowerDeepCalmUnstable => {
            let key = match request {
                IpcRequest::ReportPowerGodforgeUnstable => "godforge",
                IpcRequest::ReportPowerBrokkrsUnstable => "brokkrs",
                _ => "deep_calm",
            };
            match guard
                .power_sweep
                .report_profile_unstable(&guard.safe_store, key)
            {
                Ok(progress) => IpcResponse::success(ResponseData::PowerSweep(progress)),
                Err(e) => IpcResponse::failure(e),
            }
        }
        IpcRequest::GetSentinelStatus => {
            let status = std::fs::read_to_string(
                nidavellir_core::safe_loop::default_data_dir().join("sentinel_status.json"),
            )
            .ok();
            IpcResponse::success(ResponseData::SentinelStatus { status })
        }
        IpcRequest::StartGameTrace => {
            if guard.game_trace.start() {
                IpcResponse::success(ResponseData::GameTrace(guard.game_trace.status()))
            } else {
                IpcResponse::failure("Game trace already running")
            }
        }
        IpcRequest::StopGameTrace => {
            guard.game_trace.stop();
            IpcResponse::success(ResponseData::GameTrace(guard.game_trace.status()))
        }
        IpcRequest::GetGameTraceStatus => {
            IpcResponse::success(ResponseData::GameTrace(guard.game_trace.status()))
        }
        IpcRequest::ExportForgeLog => {
            let prog = guard.power_sweep.progress();
            match crate::gpu_power_sweep::export_forge_log(&prog) {
                Ok(export) => IpcResponse::success(ResponseData::ForgeLogExport(export)),
                Err(e) => IpcResponse::failure(format!("Export de log falhou: {e}")),
            }
        }
    }
}

fn power_sweep_start_failure(store: &nidavellir_core::safe_loop::SafeLoopStore) -> String {
    if store.load_record().pending_forge_incident.is_some() {
        "Forge recovery requires explicit operator acknowledgement before continuation".into()
    } else {
        "Power sweep already running".into()
    }
}

fn gpu_operation_running(state: &AppState) -> bool {
    state.gpu_validation.status().running
        || !matches!(
            state.real_sweep.progress().phase,
            nidavellir_core::gpu_sweep::SweepPhase::Idle
                | nidavellir_core::gpu_sweep::SweepPhase::Done
                | nidavellir_core::gpu_sweep::SweepPhase::Aborted
        )
        || state.mem_sweep.progress().running
        || state.forge_all.progress().running
        || state.benchmark.progress().running
        || state.power_sweep.progress().running
}

fn gpu_write_requires_idle(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::StartGpuValidation
            | IpcRequest::StartRealSweep
            | IpcRequest::StartRealSweepFast
            | IpcRequest::StartMemSweep
            | IpcRequest::ApplyGodforge
            | IpcRequest::ApplyBrokkrs
            | IpcRequest::ApplyDeepCalm
            | IpcRequest::ApplyMemPeak
            | IpcRequest::StartForgeAll
            | IpcRequest::StartBenchmark
            | IpcRequest::StartPowerSweep
            | IpcRequest::StartPowerSweepClean
            | IpcRequest::StartPowerSweepFast
            | IpcRequest::StartPowerSweepLong
            | IpcRequest::ResumePowerSweep
            | IpcRequest::ApplyPowerGodforge
            | IpcRequest::ApplyPowerBrokkrs
            | IpcRequest::ApplyPowerDeepCalm
    )
}

/// Route a forge-profile apply to the correct writer (Phase 2). When the active forge produced an F2
/// anchored-undervolt result (`is_undervolt == true`), apply the F2 anchored undervolt; otherwise apply
/// the legacy F1 flatten-down ceiling. F2 RAISES a lower-voltage bin to hold the clock (dropping power);
/// F1 caps frequency down — applying the wrong one is unsafe, so the route keys on the structured flag,
/// never on text. Backward-compatible: a legacy/restored payload defaults `is_undervolt = false` → F1.
fn apply_forge_profile(
    store: &nidavellir_core::safe_loop::SafeLoopStore,
    prog: &nidavellir_core::ipc::PowerSweepProgress,
    pt: Option<nidavellir_core::ipc::PowerSweepPoint>,
    label: &str,
) -> IpcResponse {
    let record = store.load_record();
    if record.pending_forge_incident.is_some() {
        return IpcResponse::failure(
            "Forge recovery requires explicit operator acknowledgement before Apply",
        );
    }
    if prog.is_undervolt {
        if !prog.profiles_qualified {
            return IpcResponse::failure(
                "F2 profiles are provisional — run Standard or Long qualification before Apply",
            );
        }
        if let Some(point) = pt {
            let (target_mhz, anchor_mv) = undervolt_apply_params(&point);
            #[cfg(windows)]
            let ledger_condemned = nidavellir_core::condemnation::CondemnationLedger::new(
                store.base_dir(),
            )
            .condemned_pairs(&crate::gpu_power_sweep::current_gpu_key())
            .refuses(target_mhz, anchor_mv);
            #[cfg(not(windows))]
            let ledger_condemned = false;
            if crate::gpu_undervolt::field_pair_blacklisted(&record, target_mhz, anchor_mv)
                || ledger_condemned
            {
                return IpcResponse::failure(
                    "F2 profile is condemned by durable real-use evidence on this GPU — run Forge again",
                );
            }
            apply_undervolt_profile(store, Some(point), label)
        } else {
            apply_undervolt_profile(store, None, label)
        }
    } else {
        apply_power_profile(store, pt, label)
    }
}

/// Resolve the F2 apply axes from a forge point: the TARGET clock to hold and the anchor VF-table bin.
/// Prefers the deterministic forge fields (`target_clock_mhz`, `vf_table_voltage_mv`) and falls back to
/// the measured `clock_mhz` / `voltage_mv` for legacy points. Pure — unit-tested without hardware.
fn undervolt_apply_params(p: &nidavellir_core::ipc::PowerSweepPoint) -> (u32, u32) {
    let target = p.target_clock_mhz.unwrap_or(p.clock_mhz);
    let anchor = p.vf_table_voltage_mv.unwrap_or(p.voltage_mv);
    (target, anchor)
}

/// Apply an F2 anchored-undervolt forge point (`target MHz` held at the anchor VF bin) and persist it.
/// Writes via the fail-closed [`crate::gpu_apply::apply_and_persist_undervolt`] (arm Safe Loop → anchored
/// write → verify → persist `undervolt` descriptor → clear flag after the survival window; any non-verified
/// outcome resets to stock and returns an error). Keeps any existing memory offset.
fn apply_undervolt_profile(
    store: &nidavellir_core::safe_loop::SafeLoopStore,
    pt: Option<nidavellir_core::ipc::PowerSweepPoint>,
    label: &str,
) -> IpcResponse {
    let Some(p) = pt else {
        return IpcResponse::failure("Run the forge first (no point for this profile)");
    };
    if !p
        .power_p99_w
        .is_some_and(|power| power.is_finite() && power > 0.0)
    {
        return IpcResponse::failure(
            "F2 profile has no confirmed sustained-p99 power — run Forge again under discovery v4",
        );
    }
    if !p.apply_qualified
        || p.apply_qualification_version
            != Some(
                nidavellir_core::f2_observation::F2_QUALIFICATION_CONTRACT_VERSION,
            )
    {
        return IpcResponse::failure(
            "F2 profile was not reconciled and qualified under the current v6 contract — run Forge again",
        );
    }
    let (target_mhz, anchor_mv) = undervolt_apply_params(&p);
    let mem = crate::gpu_apply::load_applied().unwrap_or_default().mem_offset_mhz;
    let msg = match crate::gpu_apply::apply_and_persist_undervolt(
        label.into(),
        target_mhz,
        anchor_mv,
        mem,
        store,
    ) {
        Ok(()) => format!("Applied {label}: {target_mhz} MHz @ {anchor_mv} mV VF bin (undervolt)"),
        Err(e) => format!("Apply failed: {e}"),
    };
    IpcResponse::success(ResponseData::GpuApply(applied_status(msg)))
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

    use nidavellir_core::ipc::PowerSweepPoint;

    #[test]
    fn undervolt_apply_params_prefer_deterministic_forge_fields() {
        // F2 apply axes: TARGET clock + anchor VF bin come from the deterministic forge fields, NOT the
        // measured clock/voltage (which differ by boost behavior).
        let p = PowerSweepPoint {
            clock_mhz: 1815,
            voltage_mv: 910,
            target_clock_mhz: Some(1800),
            vf_table_voltage_mv: Some(875),
            ..Default::default()
        };
        assert_eq!(undervolt_apply_params(&p), (1800, 875));
    }

    #[test]
    fn undervolt_apply_params_fall_back_to_measured_for_legacy_points() {
        // A legacy point without the deterministic fields falls back to measured clock/voltage.
        let p = PowerSweepPoint { clock_mhz: 1800, voltage_mv: 906, ..Default::default() };
        assert_eq!(undervolt_apply_params(&p), (1800, 906));
    }

    #[test]
    fn apply_route_selects_f2_for_undervolt_and_f1_otherwise() {
        // The router keys on the structured `is_undervolt` flag: F2 forge → undervolt apply; a legacy
        // (default) payload → the unchanged F1 apply path. Both with no point yield a clear failure
        // (nothing applied), which is the safe observable here without touching hardware.
        let f2 = PowerSweepProgress {
            is_undervolt: true,
            profiles_qualified: true,
            ..Default::default()
        };
        assert!(f2.is_undervolt);
        let f1 = PowerSweepProgress::default();
        assert!(!f1.is_undervolt, "default must keep the legacy F1 apply behavior");

        let r_f2 = apply_forge_profile(&dummy_store(), &f2, None, "Godforge");
        assert!(!r_f2.ok, "no forge point → failure");
        let r_f1 = apply_forge_profile(&dummy_store(), &f1, None, "Godforge");
        assert!(!r_f1.ok, "no sweep point → failure");
    }

    #[test]
    fn provisional_f2_profile_cannot_be_applied() {
        let provisional = PowerSweepProgress {
            is_undervolt: true,
            profiles_qualified: false,
            ..Default::default()
        };
        let response = apply_forge_profile(&dummy_store(), &provisional, None, "Godforge");
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("provisional")
        );
    }

    #[test]
    fn qualified_f2_profile_without_p99_cannot_be_applied() {
        let qualified = PowerSweepProgress {
            is_undervolt: true,
            profiles_qualified: true,
            ..Default::default()
        };
        let legacy_point = PowerSweepPoint {
            target_clock_mhz: Some(1800),
            vf_table_voltage_mv: Some(900),
            power_p99_w: None,
            ..Default::default()
        };
        let response =
            apply_forge_profile(&dummy_store(), &qualified, Some(legacy_point), "Brokkr's Best");
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("sustained-p99")
        );
    }

    #[test]
    fn old_f2_profile_without_exact_apply_qualification_cannot_be_applied() {
        let qualified = PowerSweepProgress {
            is_undervolt: true,
            profiles_qualified: true,
            ..Default::default()
        };
        let old_point = PowerSweepPoint {
            target_clock_mhz: Some(1860),
            vf_table_voltage_mv: Some(893),
            power_p99_w: Some(180.0),
            apply_qualified: false,
            apply_qualification_version: None,
            ..Default::default()
        };
        let response =
            apply_forge_profile(&dummy_store(), &qualified, Some(old_point), "Brokkr's Best");
        assert!(!response.ok);
        assert!(response
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("current v6 contract"));
    }

    #[test]
    fn service_wide_gpu_lease_covers_starts_and_applies_but_not_recovery_reset() {
        for request in [
            IpcRequest::StartGpuValidation,
            IpcRequest::StartRealSweep,
            IpcRequest::StartMemSweep,
            IpcRequest::StartForgeAll,
            IpcRequest::StartBenchmark,
            IpcRequest::StartPowerSweep,
            IpcRequest::StartPowerSweepClean,
            IpcRequest::StartPowerSweepFast,
            IpcRequest::StartPowerSweepLong,
            IpcRequest::ResumePowerSweep,
            IpcRequest::ApplyPowerGodforge,
            IpcRequest::ApplyPowerBrokkrs,
            IpcRequest::ApplyPowerDeepCalm,
        ] {
            assert!(
                gpu_write_requires_idle(&request),
                "{request:?} must require the GPU lease"
            );
        }
        assert!(!gpu_write_requires_idle(&IpcRequest::ResetGpuTuning));
        assert!(!gpu_write_requires_idle(&IpcRequest::GetPowerSweepProgress));
        assert!(!gpu_write_requires_idle(&IpcRequest::StopPowerSweep));
        assert!(!gpu_write_requires_idle(&IpcRequest::ReadSensors));
    }

    fn dummy_store() -> nidavellir_core::safe_loop::SafeLoopStore {
        nidavellir_core::safe_loop::SafeLoopStore::new(std::env::temp_dir().join("nidavellir-test-ipc"))
    }
}
