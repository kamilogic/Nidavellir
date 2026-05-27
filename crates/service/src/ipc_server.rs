use std::sync::{Arc, Mutex};

use nidavellir_core::ipc::{
    parse_request, serialize_response, DriverStatusPayload, IpcRequest, IpcResponse, ResponseData,
};
use tracing::{debug, warn};

use crate::AppState;
use crate::PIPE_NAME;

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
            let hw = nidavellir_core::detect_hardware();
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
            let hw = nidavellir_core::detect_hardware();
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
    }
}
