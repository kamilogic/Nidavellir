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

    match request {
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
        IpcRequest::StartGpuSweep => {
            let store = guard.safe_store.clone();
            let started = guard.gpu_sweep.start(store);
            if started {
                IpcResponse::success(ResponseData::GpuSweep(guard.gpu_sweep.progress()))
            } else {
                IpcResponse::failure("GPU sweep already running")
            }
        }
        IpcRequest::StopGpuSweep => {
            guard.gpu_sweep.stop();
            IpcResponse::success(ResponseData::GpuSweep(guard.gpu_sweep.progress()))
        }
        IpcRequest::GetGpuSweepProgress => {
            IpcResponse::success(ResponseData::GpuSweep(guard.gpu_sweep.progress()))
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
