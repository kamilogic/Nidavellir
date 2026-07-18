use serde_json::Value;

#[cfg(windows)]
use std::io::{Read, Write};

const PIPE_NAME: &str = r"\\.\pipe\NidavellirCore";

pub fn call_service(method: &str) -> Result<Value, String> {
    call_service_with_params(method, None)
}

pub fn call_service_with_params(method: &str, params: Option<Value>) -> Result<Value, String> {
    let request = match params {
        Some(params) => serde_json::to_string(&serde_json::json!({
            "method": method,
            "params": params,
        }))
        .map_err(|e| format!("Invalid service request: {e}"))?,
        None => serde_json::to_string(&serde_json::json!({ "method": method }))
            .map_err(|e| format!("Invalid service request: {e}"))?,
    };
    let response = send_request(&request)?;
    let parsed: Value = serde_json::from_str(&response)
        .map_err(|e| format!("Invalid service response: {e}"))?;
    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown service error");
        return Err(msg.to_string());
    }
    Ok(parsed)
}

fn send_request(request: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::io::Write;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::Win32::System::Pipes::WaitNamedPipeW;
        use windows::Win32::Foundation::GENERIC_READ;
        use windows::Win32::Foundation::GENERIC_WRITE;

        let pipe_name: Vec<u16> = PIPE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let ok = WaitNamedPipeW(
                windows::core::PCWSTR(pipe_name.as_ptr()),
                5000,
            );
            if !ok.as_bool() {
                #[cfg(debug_assertions)]
                let msg = "Core Service not running - start with: cargo run -p nidavellir-service -- console";
                #[cfg(not(debug_assertions))]
                let msg = "Core Service not running - check Windows Services (NidavellirCore) or reinstall";
                return Err(msg.into());
            }
        }

        let handle = unsafe {
            CreateFileW(
                windows::core::PCWSTR(pipe_name.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                windows::Win32::Foundation::HANDLE::default(),
            )
        };
        if handle.is_err() {
            return Err(format!(
                "Failed to connect to Core Service: {}",
                handle.unwrap_err()
            ));
        }
        let handle = handle.unwrap();

        let mut file = PipeHandle { handle };
        file.write_all(request.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;
        file.write_all(b"\n")
            .map_err(|e| format!("Write failed: {e}"))?;

        let mut buf = String::new();
        file.read_line(&mut buf)
            .map_err(|e| format!("Read failed: {e}"))?;

        unsafe {
            let _ = CloseHandle(handle);
        }

        Ok(buf.trim().to_string())
    }

    #[cfg(not(windows))]
    {
        let _ = (request, Duration::from_secs(1));
        Err("Nidavellir Core Service IPC requires Windows".into())
    }
}

#[cfg(windows)]
struct PipeHandle {
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Write for PipeHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use windows::Win32::Storage::FileSystem::WriteFile;
        let mut written: u32 = 0;
        unsafe {
            WriteFile(self.handle, Some(buf), Some(&mut written), None)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        }
        Ok(written as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
impl Read for PipeHandle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows::Win32::Storage::FileSystem::ReadFile;
        let mut read: u32 = 0;
        unsafe {
            ReadFile(self.handle, Some(buf), Some(&mut read), None)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        }
        Ok(read as usize)
    }
}

#[cfg(windows)]
impl PipeHandle {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        let mut total = 0;
        let mut byte = [0u8; 1];
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let n = self.read(&mut byte)?;
            if n == 0 {
                break;
            }
            total += n;
            if byte[0] == b'\n' {
                break;
            }
            bytes.push(byte[0]);
        }
        *buf = String::from_utf8(bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(total)
    }
}
