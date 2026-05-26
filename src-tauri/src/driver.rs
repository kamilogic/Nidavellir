#![allow(non_camel_case_types)]

type ols_bool = i32;
type ols_dword = u32;

#[derive(Debug, Clone, Copy)]
pub struct MsrValue {
    pub eax: u32,
    pub edx: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriverStatus {
    Loaded,
    NotLoaded,
    Failed(String),
}

pub struct DriverManager {
    lib: Option<libloading::Library>,
    status: DriverStatus,
}

// FFI function signatures
type InitializeOls = unsafe extern "system" fn() -> ols_bool;
type DeinitializeOls = unsafe extern "system" fn();
type ReadMsr = unsafe extern "system" fn(index: ols_dword, eax: *mut ols_dword, edx: *mut ols_dword) -> ols_bool;
type WriteMsr = unsafe extern "system" fn(index: ols_dword, eax: ols_dword, edx: ols_dword) -> ols_bool;
type ReadPciConfig = unsafe extern "system" fn(pci_addr: ols_dword, reg_size: ols_dword, reg_offset: ols_dword) -> ols_dword;
type ReadIoPortByte = unsafe extern "system" fn(port: u16, value: *mut u8) -> ols_bool;
type WriteIoPortByte = unsafe extern "system" fn(port: u16, value: u8) -> ols_bool;

impl DriverManager {
    pub fn new() -> Self {
        let mut dm = Self { lib: None, status: DriverStatus::NotLoaded };
        dm.load();
        dm
    }

    fn load(&mut self) {
        let dll_name = "WinRing0x64.dll";
        let lib = match unsafe { libloading::Library::new(dll_name) } {
            Ok(l) => l,
            Err(e) => {
                self.status = DriverStatus::Failed(format!("Failed to load {dll_name}: {e}"));
                return;
            }
        };

        let init: libloading::Symbol<InitializeOls> = match unsafe { lib.get(b"InitializeOls") } {
            Ok(s) => s,
            Err(e) => {
                self.status = DriverStatus::Failed(format!("Missing InitializeOls: {e}"));
                return;
            }
        };

        let result = unsafe { init() };
        if result == 0 {
            self.status = DriverStatus::Failed("InitializeOls returned FALSE (no admin? no driver?)".into());
            return;
        }

        self.lib = Some(lib);
        self.status = DriverStatus::Loaded;
    }

    pub fn status(&self) -> &DriverStatus {
        &self.status
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self.status, DriverStatus::Loaded)
    }

    pub fn read_msr(&self, index: u32) -> Result<MsrValue, String> {
        let lib = self.lib.as_ref().ok_or("Driver not loaded")?;
        let func: libloading::Symbol<ReadMsr> =
            unsafe { lib.get(b"ReadMsr") }.map_err(|e| e.to_string())?;
        let mut eax: u32 = 0;
        let mut edx: u32 = 0;
        let ok = unsafe { func(index, &mut eax, &mut edx) };
        if ok == 0 {
            Err(format!("ReadMsr(0x{index:X}) failed"))
        } else {
            Ok(MsrValue { eax, edx })
        }
    }

    pub fn write_msr(&self, index: u32, eax: u32, edx: u32) -> Result<(), String> {
        let lib = self.lib.as_ref().ok_or("Driver not loaded")?;
        let func: libloading::Symbol<WriteMsr> =
            unsafe { lib.get(b"WriteMsr") }.map_err(|e| e.to_string())?;
        let ok = unsafe { func(index, eax, edx) };
        if ok == 0 {
            Err(format!("WriteMsr(0x{index:X}) failed"))
        } else {
            Ok(())
        }
    }

    pub fn read_io_port_byte(&self, port: u16) -> Result<u8, String> {
        let lib = self.lib.as_ref().ok_or("Driver not loaded")?;
        let func: libloading::Symbol<ReadIoPortByte> =
            unsafe { lib.get(b"ReadIoPortByte") }.map_err(|e| e.to_string())?;
        let mut value: u8 = 0;
        let ok = unsafe { func(port, &mut value) };
        if ok == 0 { Err(format!("ReadIoPortByte(0x{port:04X}) failed")) } else { Ok(value) }
    }

    pub fn write_io_port_byte(&self, port: u16, value: u8) -> Result<(), String> {
        let lib = self.lib.as_ref().ok_or("Driver not loaded")?;
        let func: libloading::Symbol<WriteIoPortByte> =
            unsafe { lib.get(b"WriteIoPortByte") }.map_err(|e| e.to_string())?;
        let ok = unsafe { func(port, value) };
        if ok == 0 { Err(format!("WriteIoPortByte(0x{port:04X}, 0x{value:02X}) failed")) } else { Ok(()) }
    }

    pub fn read_pci_config(&self, bus: u8, device: u8, function: u8, offset: u8, size: u32) -> Result<u32, String> {
        let lib = self.lib.as_ref().ok_or("Driver not loaded")?;
        let func: libloading::Symbol<ReadPciConfig> =
            unsafe { lib.get(b"ReadPciConfig") }.map_err(|e| e.to_string())?;
        let addr = ((bus as u32) << 20) | ((device as u32) << 15) | ((function as u32) << 12);
        let value = unsafe { func(addr, size, offset as u32) };
        Ok(value)
    }
}

impl Drop for DriverManager {
    fn drop(&mut self) {
        if let Some(ref lib) = self.lib {
            if let Ok(func) = unsafe { lib.get::<DeinitializeOls>(b"DeinitializeOls") } {
                unsafe { func() };
            }
        }
    }
}

// Well-known MSR indices
pub const IA32_PERF_STATUS: u32 = 0x198;
pub const IA32_PERF_CTL: u32 = 0x199;
pub const IA32_CLOCK_MODULATION: u32 = 0x19A;
pub const IA32_THERM_INTERRUPT: u32 = 0x19B;
pub const IA32_THERM_STATUS: u32 = 0x19C;
pub const IA32_MISC_ENABLES: u32 = 0x1A0;
pub const IA32_ENERGY_PERF_BIAS: u32 = 0x1B0;
pub const IA32_PACKAGE_POWER_LIMIT: u32 = 0x610;
pub const IA32_PACKAGE_POWER_LIMIT_2: u32 = 0x611;
pub const MSR_VR_CURRENT_CONFIG: u32 = 0x601;
pub const MSR_PKG_CST_CONFIG_CONTROL: u32 = 0xE2;
pub const MSR_PLATFORM_INFO: u32 = 0xCE;
pub const MSR_TURBO_RATIO_LIMIT: u32 = 0x1AD;
pub const MSR_TURBO_RATIO_LIMIT1: u32 = 0x1AE;
pub const MSR_POWER_CTL: u32 = 0x1FC;
