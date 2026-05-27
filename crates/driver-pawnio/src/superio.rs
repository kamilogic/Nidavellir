//! Super I/O probe via PawnIO `LpcIO` (chip ID + raw VIN bytes for profile mapping in core).

use nidavellir_core::superio_profile::{SuperIoProbe, SuperIoVendor, VIN_RAW_LEN};

use crate::pawnio_lib::{find_module_blob, PawnIoLib};

struct LpcIoSession {
    exec: crate::pawnio_lib::PawnIoExecutor,
}

impl LpcIoSession {
    fn open() -> Result<Self, String> {
        let blob = find_module_blob("LpcIO").ok_or_else(|| {
            "LpcIO.bin not found — place PawnIO.Modules release files in pawnio-modules/".to_string()
        })?;
        let lib = PawnIoLib::load_default()?;
        let exec = lib.open_executor()?;
        exec.load_module(&blob)?;
        Ok(Self { exec })
    }

    fn ioctl(&self, name: &str, input: &[u64], out_count: usize) -> Result<Vec<u64>, String> {
        self.exec.execute(name, input, out_count)
    }

    fn select_slot(&self, slot: u64) -> Result<(), String> {
        self.ioctl("ioctl_select_slot", &[slot], 0)?;
        Ok(())
    }

    fn pio_outb(&self, port: u16, value: u8) -> Result<(), String> {
        self.ioctl("ioctl_pio_outb", &[port as u64, value as u64], 0)?;
        Ok(())
    }

    fn pio_inb(&self, port: u16) -> Result<u8, String> {
        let out = self.ioctl("ioctl_pio_inb", &[port as u64], 1)?;
        Ok((out.first().copied().unwrap_or(0) & 0xFF) as u8)
    }

    fn superio_inb(&self, reg: u8) -> Result<u8, String> {
        let out = self.ioctl("ioctl_superio_inb", &[reg as u64], 1)?;
        Ok((out.first().copied().unwrap_or(0) & 0xFF) as u8)
    }

    fn superio_outb(&self, reg: u8, val: u8) -> Result<(), String> {
        self.ioctl("ioctl_superio_outb", &[reg as u64, val as u64], 0)?;
        Ok(())
    }

    fn find_bars(&self) -> Result<(), String> {
        self.ioctl("ioctl_find_bars", &[], 0)?;
        Ok(())
    }
}

/// Probe Super I/O chip and read raw VIN ADC bytes (no board-specific labels).
pub fn probe_superio() -> Option<SuperIoProbe> {
    match try_probe() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Super I/O probe failed: {e}");
            None
        }
    }
}

/// Exposed for diagnostics/tests.
pub fn try_probe() -> Result<Option<SuperIoProbe>, String> {
    with_isa_bus_lock(|| {
        let session = LpcIoSession::open()?;
        for slot in [0u64, 1] {
            if let Some(p) = try_slot(&session, slot)? {
                return Ok(Some(p));
            }
        }
        Ok(None)
    })
}

/// HWMonitor / LHM hold this while touching LPC Super I/O (see PawnIO `LpcIO` docs).
#[cfg(windows)]
fn with_isa_bus_lock<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenMutexW, ReleaseMutex, WaitForSingleObject, MUTEX_ALL_ACCESS, INFINITE,
    };

    let name: Vec<u16> = r"\BaseNamedObjects\Access_ISABUS.HTP.Method"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe { OpenMutexW(MUTEX_ALL_ACCESS, false, PCWSTR(name.as_ptr())) };

    let Some(h) = (match handle {
        Ok(h) if !h.is_invalid() => Some(h),
        _ => None,
    }) else {
        tracing::debug!("ISA bus mutex not available, probing without lock");
        return f();
    };

    unsafe {
        let wait = WaitForSingleObject(h, INFINITE);
        if wait != WAIT_OBJECT_0 {
            let _ = CloseHandle(h);
            return Err("Failed to wait on ISA bus mutex".into());
        }
    }

    let result = f();

    unsafe {
        let _ = ReleaseMutex(h);
        let _ = CloseHandle(h);
    }
    result
}

#[cfg(not(windows))]
fn with_isa_bus_lock<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    f()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalVendor {
    Ite,
    Nuvoton,
}

impl From<LocalVendor> for SuperIoVendor {
    fn from(v: LocalVendor) -> Self {
        match v {
            LocalVendor::Ite => SuperIoVendor::Ite,
            LocalVendor::Nuvoton => SuperIoVendor::Nuvoton,
        }
    }
}

fn try_slot(session: &LpcIoSession, slot: u64) -> Result<Option<SuperIoProbe>, String> {
    session.select_slot(slot)?;
    let addr = if slot == 0 { 0x2e } else { 0x4e };

    // Nuvoton first — ITE config enter/exit can confuse NCT67xx if attempted first.
    for vendor in [LocalVendor::Nuvoton, LocalVendor::Ite] {
        enter_config(session, addr, vendor)?;
        let chip_high = session.superio_inb(0x20)?;
        let chip_low = session.superio_inb(0x21)?;
        let chip_id = ((chip_high as u16) << 8) | chip_low as u16;

        if !chip_id_is_plausible(chip_id, vendor) {
            let _ = exit_config(session, addr, vendor);
            tracing::debug!(
                "Super I/O slot {slot} {vendor:?}: chip 0x{chip_id:04X} not plausible"
            );
            continue;
        }

        if vendor == LocalVendor::Nuvoton {
            nuvoton_enable_io_mapping(session)?;
        }

        let ldn = hwm_logical_device(vendor);
        session.superio_outb(0x07, ldn)?;
        if session.superio_inb(0x30)? == 0 {
            session.superio_outb(0x30, 0x01)?;
        }
        let base_high = session.superio_inb(0x60)?;
        let base_low = session.superio_inb(0x61)?;

        let mut io_base = ((base_high as u16) << 8) | base_low as u16;
        if (io_base & 0x07) == 0x05 {
            io_base &= 0xFFF8;
        }
        if io_base < 0x100 || io_base == 0xFFFF {
            let _ = exit_config(session, addr, vendor);
            tracing::debug!("Super I/O 0x{chip_id:04X}: invalid HWM base 0x{io_base:04X}");
            continue;
        }

        // LpcIO only whitelists HWM BAR ports after find_bars, and find_bars requires
        // config mode (see PawnIO.Modules LpcIO.p).
        if let Err(e) = session.find_bars() {
            let _ = exit_config(session, addr, vendor);
            tracing::debug!(
                "Super I/O 0x{chip_id:04X}: ioctl_find_bars failed in config mode: {e}"
            );
            continue;
        }

        let _ = exit_config(session, addr, vendor);

        let vin_raw = read_vin_raw(session, io_base, vendor)?;

        tracing::debug!(
            "Super I/O 0x{chip_id:04X} ({vendor:?}) slot {slot}, HWM 0x{io_base:04X}, vin={vin_raw:?}"
        );

        return Ok(Some(SuperIoProbe {
            chip_id,
            vendor: vendor.into(),
            lpc_slot: slot as u8,
            io_base,
            vin_raw,
        }));
    }

    Ok(None)
}

/// NCT6775 HWM index/data ports are at io_base + 5 / + 6 (Linux `nct6775-platform`).
const NCT6775_HWM_INDEX: u16 = 5;
const NCT6775_HWM_DATA: u16 = 6;
const NCT6775_REG_BANK: u8 = 0x4E;
const NCT6791_REG_HM_IO_SPACE_LOCK_ENABLE: u8 = 0x28;

/// NCT6798D uses banked `0x480+` inputs (Linux `NCT6779_REG_IN`), not legacy `0x20+`.
const NCT6798_VIN_COUNT: usize = 15;
const NCT6798_VIN_BASE: u16 = 0x480;

fn read_vin_raw(
    session: &LpcIoSession,
    io_base: u16,
    vendor: LocalVendor,
) -> Result<[u8; VIN_RAW_LEN], String> {
    let mut vin_raw = [0u8; VIN_RAW_LEN];
    match vendor {
        LocalVendor::Ite => {
            for i in 0u8..9 {
                vin_raw[i as usize] = session.pio_inb(io_base + 0x20 + i as u16)?;
            }
        }
        LocalVendor::Nuvoton => {
            let mut bank: Option<u8> = None;
            for i in 0..NCT6798_VIN_COUNT {
                let reg = NCT6798_VIN_BASE + i as u16;
                vin_raw[i] = nuvoton_hwm_read_reg(session, io_base, reg, &mut bank)?;
            }
        }
    }
    Ok(vin_raw)
}

fn nuvoton_enable_io_mapping(session: &LpcIoSession) -> Result<(), String> {
    let val = session.superio_inb(NCT6791_REG_HM_IO_SPACE_LOCK_ENABLE)?;
    if val & 0x10 != 0 {
        session.superio_outb(NCT6791_REG_HM_IO_SPACE_LOCK_ENABLE, val & !0x10)?;
        tracing::debug!("Nuvoton: cleared HM IO space lock (0x28)");
    }
    Ok(())
}

fn nuvoton_hwm_read_reg(
    session: &LpcIoSession,
    io_base: u16,
    reg: u16,
    cached_bank: &mut Option<u8>,
) -> Result<u8, String> {
    let bank = (reg >> 8) as u8;
    let index = (reg & 0xFF) as u8;
    if *cached_bank != Some(bank) {
        session.pio_outb(io_base + NCT6775_HWM_INDEX, NCT6775_REG_BANK)?;
        session.pio_outb(io_base + NCT6775_HWM_DATA, bank)?;
        *cached_bank = Some(bank);
    }
    session.pio_outb(io_base + NCT6775_HWM_INDEX, index)?;
    session.pio_inb(io_base + NCT6775_HWM_DATA)
}

fn chip_id_is_plausible(chip_id: u16, vendor: LocalVendor) -> bool {
    if chip_id == 0 || chip_id == 0xFFFF {
        return false;
    }
    match vendor {
        LocalVendor::Ite => (chip_id >> 8) & 0xFF == 0x87,
        LocalVendor::Nuvoton => matches!(
            chip_id & 0xFFF0,
            0xC800 | 0xC910 | 0xD120 | 0xD350 | 0xD420 | 0xD428 | 0xD42B
        ),
    }
}

fn hwm_logical_device(vendor: LocalVendor) -> u8 {
    match vendor {
        LocalVendor::Ite => 0x04,
        LocalVendor::Nuvoton => 0x0B,
    }
}

fn enter_config(session: &LpcIoSession, addr: u16, vendor: LocalVendor) -> Result<(), String> {
    match vendor {
        LocalVendor::Ite => {
            session.pio_outb(addr, 0x87)?;
            session.pio_outb(addr, 0x01)?;
            session.pio_outb(addr, 0x55)?;
            session.pio_outb(addr, 0x55)?;
        }
        LocalVendor::Nuvoton => {
            session.pio_outb(addr, 0x87)?;
            session.pio_outb(addr, 0x87)?;
        }
    }
    Ok(())
}

fn exit_config(session: &LpcIoSession, addr: u16, vendor: LocalVendor) -> Result<(), String> {
    match vendor {
        LocalVendor::Ite => {
            session.pio_outb(addr, 0x02)?;
            session.pio_outb(addr, 0x02)?;
        }
        LocalVendor::Nuvoton => {
            session.pio_outb(addr, 0xAA)?;
        }
    }
    Ok(())
}
