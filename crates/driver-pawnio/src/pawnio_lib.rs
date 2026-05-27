//! Dynamic loader for `PawnIOLib.dll` (installed with PawnIO).

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::Library;
use windows::Win32::Foundation::HANDLE;

type Hresult = i32;
type PawnioOpen = unsafe extern "system" fn(*mut HANDLE) -> Hresult;
type PawnioClose = unsafe extern "system" fn(HANDLE) -> Hresult;
type PawnioLoad = unsafe extern "system" fn(HANDLE, *const u8, usize) -> Hresult;
type PawnioExecute = unsafe extern "system" fn(
    HANDLE,
    *const u8,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> Hresult;

pub struct PawnIoLib {
    _lib: Library,
    open: PawnioOpen,
    close: PawnioClose,
    load: PawnioLoad,
    execute: PawnioExecute,
}

impl PawnIoLib {
    pub fn load_default() -> Result<Arc<Self>, String> {
        for path in pawnio_lib_paths() {
            if path.is_file() {
                return Self::load_from(&path).map(Arc::new);
            }
        }
        Err("PawnIOLib.dll not found (install PawnIO from https://pawnio.eu/)".into())
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let lib = unsafe { Library::new(path) }
            .map_err(|e| format!("Failed to load {}: {e}", path.display()))?;

        unsafe {
            Ok(Self {
                open: *lib
                    .get(b"pawnio_open")
                    .map_err(|e| e.to_string())?,
                close: *lib
                    .get(b"pawnio_close")
                    .map_err(|e| e.to_string())?,
                load: *lib
                    .get(b"pawnio_load")
                    .map_err(|e| e.to_string())?,
                execute: *lib
                    .get(b"pawnio_execute")
                    .map_err(|e| e.to_string())?,
                _lib: lib,
            })
        }
    }

    pub fn open_executor(self: &Arc<Self>) -> Result<PawnIoExecutor, String> {
        let mut handle = HANDLE::default();
        let hr = unsafe { (self.open)(&mut handle) };
        if !hresult_ok(hr) {
            return Err(format!("pawnio_open failed: 0x{hr:08X}"));
        }
        Ok(PawnIoExecutor {
            lib: Arc::clone(self),
            handle,
        })
    }
}

pub struct PawnIoExecutor {
    lib: Arc<PawnIoLib>,
    handle: HANDLE,
}

// HANDLE is not `Send` in the windows crate; we only use executors behind `DriverManager`'s mutex.
unsafe impl Send for PawnIoExecutor {}
unsafe impl Sync for PawnIoExecutor {}

impl PawnIoExecutor {
    pub fn load_module(&self, blob: &[u8]) -> Result<(), String> {
        let hr =
            unsafe { (self.lib.load)(self.handle, blob.as_ptr(), blob.len()) };
        if !hresult_ok(hr) {
            return Err(format!("pawnio_load failed: 0x{hr:08X}"));
        }
        Ok(())
    }

    pub fn execute(
        &self,
        function: &str,
        input: &[u64],
        output_count: usize,
    ) -> Result<Vec<u64>, String> {
        let name = CString::new(function).map_err(|e| e.to_string())?;
        let mut output = vec![0u64; output_count];
        let mut returned = 0usize;
        let hr = unsafe {
            (self.lib.execute)(
                self.handle,
                name.as_ptr() as *const u8,
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut returned,
            )
        };
        if !hresult_ok(hr) {
            return Err(format!("pawnio_execute({function}) failed: 0x{hr:08X}"));
        }
        output.truncate(returned);
        Ok(output)
    }
}

impl Drop for PawnIoExecutor {
    fn drop(&mut self) {
        let hr = unsafe { (self.lib.close)(self.handle) };
        if !hresult_ok(hr) {
            tracing::warn!("pawnio_close failed: 0x{hr:08X}");
        }
    }
}

fn hresult_ok(hr: Hresult) -> bool {
    hr >= 0
}

fn pawnio_lib_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        paths.push(PathBuf::from(pf).join("PawnIO").join("PawnIOLib.dll"));
    }
    if let Ok(dir) = std::env::current_exe() {
        if let Some(parent) = dir.parent() {
            paths.push(parent.join("PawnIOLib.dll"));
        }
    }
    paths
}

/// Signed module search paths (`IntelMSR.bin`, `LpcIO.bin`, …).
pub fn module_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(dir) = std::env::var("NIDAVELLIR_PAWNIO_MODULES") {
        paths.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from);
        for _ in 0..8 {
            let Some(ref d) = dir else { break };
            paths.push(d.join("pawnio-modules"));
            paths.push(d.join("resources").join("pawnio-modules"));
            paths.push(
                d.join("apps")
                    .join("ui")
                    .join("src-tauri")
                    .join("resources")
                    .join("pawnio-modules"),
            );
            dir = d.parent().map(PathBuf::from);
        }
    }
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/ui/src-tauri/resources/pawnio-modules"),
    );
    paths
}

pub fn find_module_blob(name: &str) -> Option<Vec<u8>> {
    let file = format!("{name}.bin");
    for dir in module_search_paths() {
        let path = dir.join(&file);
        if path.is_file() {
            return std::fs::read(&path).ok();
        }
    }
    None
}
