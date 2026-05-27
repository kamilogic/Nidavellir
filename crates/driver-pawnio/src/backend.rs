use std::fmt;
use std::sync::Mutex;

use crate::pawnio_lib::{find_module_blob, PawnIoExecutor, PawnIoLib};
use crate::superio::probe_superio;
use nidavellir_core::superio_profile::SuperIoProbe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsrValue {
    pub eax: u32,
    pub edx: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverStatus {
    Loaded,
    NotLoaded,
    Failed(String),
}

impl fmt::Display for DriverStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl DriverStatus {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::NotLoaded => "not_installed",
            Self::Failed(_) => "error",
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Loaded => None,
            Self::NotLoaded => Some(
                "PawnIO not installed. Install from https://pawnio.eu/ and bundle IntelMSR.bin + LpcIO.bin."
                    .into(),
            ),
            Self::Failed(msg) => Some(msg.clone()),
        }
    }
}

pub trait DriverBackend: Send + Sync {
    fn status(&self) -> DriverStatus;
    fn read_msr(&self, index: u32) -> Result<MsrValue, String>;
    fn write_msr(&self, index: u32, value: MsrValue) -> Result<(), String>;
    fn probe_superio(&self) -> Option<SuperIoProbe>;
    fn read_cpu_temperature_c(&self) -> Option<f32>;
}

pub struct DriverManager {
    backend: Box<dyn DriverBackend>,
}

impl DriverManager {
    pub fn new() -> Self {
        Self {
            backend: Box::new(create_backend()),
        }
    }

    pub fn with_backend(backend: Box<dyn DriverBackend>) -> Self {
        Self { backend }
    }

    pub fn status(&self) -> DriverStatus {
        self.backend.status()
    }

    pub fn read_msr(&self, index: u32) -> Result<MsrValue, String> {
        self.backend.read_msr(index)
    }

    pub fn write_msr(&self, index: u32, value: MsrValue) -> Result<(), String> {
        self.backend.write_msr(index, value)
    }

    pub fn probe_superio(&self) -> Option<SuperIoProbe> {
        self.backend.probe_superio()
    }

    pub fn read_cpu_temperature_c(&self) -> Option<f32> {
        self.backend.read_cpu_temperature_c()
    }

    pub fn read_vcore_intel_mv(&self) -> Option<u32> {
        let msr = self.read_msr(crate::msr::IA32_PERF_STATUS).ok()?;
        let vid = nidavellir_core::msr::extract_perf_status_vid(nidavellir_core::msr::MsrValue {
            eax: msr.eax,
            edx: msr.edx,
        });
        nidavellir_core::msr::vid_to_vcore_mv_pre_haswell(vid)
    }
}

impl Default for DriverManager {
    fn default() -> Self {
        Self::new()
    }
}

fn create_backend() -> impl DriverBackend {
    #[cfg(windows)]
    {
        PawnIoBackend::new()
    }
    #[cfg(not(windows))]
    {
        StubBackend
    }
}

#[cfg(not(windows))]
struct StubBackend;

#[cfg(not(windows))]
impl DriverBackend for StubBackend {
    fn status(&self) -> DriverStatus {
        DriverStatus::NotLoaded
    }

    fn read_msr(&self, _index: u32) -> Result<MsrValue, String> {
        Err("PawnIO is only supported on Windows".into())
    }

    fn write_msr(&self, _index: u32, _value: MsrValue) -> Result<(), String> {
        Err("PawnIO is only supported on Windows".into())
    }

    fn probe_superio(&self) -> Option<SuperIoProbe> {
        None
    }

    fn read_cpu_temperature_c(&self) -> Option<f32> {
        None
    }
}

#[cfg(windows)]
struct IntelMsrSession {
    _lib: std::sync::Arc<PawnIoLib>,
    exec: PawnIoExecutor,
}

#[cfg(windows)]
impl IntelMsrSession {
    fn open() -> Result<Self, String> {
        let blob = find_module_blob("IntelMSR").ok_or_else(|| {
            "IntelMSR.bin not found — copy from PawnIO.Modules release into pawnio-modules/"
                .to_string()
        })?;
        let lib = PawnIoLib::load_default()?;
        let exec = lib.open_executor()?;
        exec.load_module(&blob)?;
        Ok(Self {
            _lib: lib,
            exec,
        })
    }

    fn read_msr(&self, index: u32) -> Result<MsrValue, String> {
        let out = self
            .exec
            .execute("ioctl_read_msr", &[index as u64], 1)?;
        let raw = *out.first().ok_or("ioctl_read_msr returned no data")?;
        Ok(msr_from_u64(raw))
    }

    fn write_msr(&self, index: u32, value: MsrValue) -> Result<(), String> {
        let packed = (value.edx as u64) << 32 | value.eax as u64;
        self.exec
            .execute("ioctl_write_msr", &[index as u64, packed], 0)?;
        Ok(())
    }
}

#[cfg(windows)]
fn msr_from_u64(raw: u64) -> MsrValue {
    MsrValue {
        eax: raw as u32,
        edx: (raw >> 32) as u32,
    }
}

#[cfg(windows)]
struct PawnIoBackend {
    device_ok: bool,
    msr: Mutex<Option<Result<IntelMsrSession, String>>>,
}

#[cfg(windows)]
impl PawnIoBackend {
    fn new() -> Self {
        use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let device: Vec<u16> = r"\\.\PawnIO"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                windows::core::PCWSTR(device.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )
        };

        let device_ok = matches!(handle, Ok(h) if !h.is_invalid());
        if let Ok(h) = handle {
            if !h.is_invalid() {
                unsafe {
                    let _ = CloseHandle(h);
                }
            }
        }

        Self {
            device_ok,
            msr: Mutex::new(None),
        }
    }

    fn msr_session(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<Result<IntelMsrSession, String>>>, String> {
        self.msr.lock().map_err(|e| format!("MSR session lock: {e}"))
    }

    fn with_msr_session<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&IntelMsrSession) -> Result<T, String>,
    {
        if !self.device_ok {
            return Err("PawnIO driver not loaded".into());
        }
        let mut guard = self.msr_session()?;
        if guard.is_none() {
            *guard = Some(IntelMsrSession::open());
        }
        let session = guard
            .as_ref()
            .ok_or_else(|| "IntelMSR session missing".to_string())?
            .as_ref()
            .map_err(|e| e.clone())?;
        f(session)
    }
}

#[cfg(windows)]
impl DriverBackend for PawnIoBackend {
    fn status(&self) -> DriverStatus {
        if !self.device_ok {
            return DriverStatus::NotLoaded;
        }
        let mut guard = match self.msr_session() {
            Ok(g) => g,
            Err(e) => return DriverStatus::Failed(e),
        };
        if guard.is_none() {
            *guard = Some(IntelMsrSession::open());
        }
        match guard.as_ref() {
            Some(Ok(_)) => DriverStatus::Loaded,
            Some(Err(e)) => DriverStatus::Failed(e.clone()),
            None => DriverStatus::Failed("IntelMSR session failed".into()),
        }
    }

    fn read_msr(&self, index: u32) -> Result<MsrValue, String> {
        self.with_msr_session(|s| s.read_msr(index))
    }

    fn write_msr(&self, index: u32, value: MsrValue) -> Result<(), String> {
        self.with_msr_session(|s| s.write_msr(index, value))
    }

    fn probe_superio(&self) -> Option<SuperIoProbe> {
        if !self.device_ok {
            return None;
        }
        probe_superio()
    }

    fn read_cpu_temperature_c(&self) -> Option<f32> {
        use nidavellir_core::msr_temp::{
            core_temp_c_from_msrs, package_temp_c_from_msr, IA32_PACKAGE_THERM_STATUS,
            IA32_TEMPERATURE_TARGET, IA32_THERM_STATUS,
        };

        let core = self
            .read_msr(IA32_THERM_STATUS)
            .ok()
            .zip(self.read_msr(IA32_TEMPERATURE_TARGET).ok())
            .and_then(|(status, target)| {
                core_temp_c_from_msrs(
                    nidavellir_core::msr::MsrValue {
                        eax: status.eax,
                        edx: status.edx,
                    },
                    nidavellir_core::msr::MsrValue {
                        eax: target.eax,
                        edx: target.edx,
                    },
                )
            });

        if core.is_some() {
            return core;
        }

        self.read_msr(IA32_PACKAGE_THERM_STATUS)
            .ok()
            .and_then(|status| {
                package_temp_c_from_msr(nidavellir_core::msr::MsrValue {
                    eax: status.eax,
                    edx: status.edx,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msr_pack_unpack_roundtrip() {
        let v = MsrValue {
            eax: 0x1234_5678,
            edx: 0x9ABC_DEF0,
        };
        let raw = (v.edx as u64) << 32 | v.eax as u64;
        let back = MsrValue {
            eax: raw as u32,
            edx: (raw >> 32) as u32,
        };
        assert_eq!(back, v);
    }

    #[cfg(not(windows))]
    #[test]
    fn stub_backend_reports_not_loaded_on_non_windows() {
        let mgr = DriverManager::with_backend(Box::new(StubBackend));
        assert_eq!(mgr.status(), DriverStatus::NotLoaded);
    }

    #[cfg(windows)]
    #[test]
    fn integration_probe_superio_on_hardware() {
        use crate::pawnio_lib::{find_module_blob, module_search_paths};
        eprintln!("module paths: {:?}", module_search_paths());
        eprintln!(
            "LpcIO.bin: {:?}",
            find_module_blob("LpcIO").map(|b| b.len())
        );
        match crate::superio::try_probe() {
            Ok(p) => eprintln!("superio probe ok: {p:?}"),
            Err(e) => eprintln!("superio probe err: {e}"),
        }
    }
}
