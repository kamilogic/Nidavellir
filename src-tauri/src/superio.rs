use serde::Serialize;
use crate::driver::DriverManager;

/// ITE IT87xx voltage ADC resolution: AVCC (3.3 V) / 256 levels ≈ 12.89 mV per LSB.
const VOLT_LSB: f32 = 3.3 / 256.0;

/// Resistor-divider multipliers for channels that monitor rails above 3.3 V.
/// Index matches VIN channel (VIN0-VIN8). 1.0 = no divider (direct measurement).
/// These are the typical ASUS Z690 / ITE IT8790E channel assignments:
///   VIN0  CPU Vcore          × 1.0
///   VIN1  DRAM voltage       × 1.0   ← what we care about
///   VIN2  +12 V rail         × 10.0  (100k/10k divider → ×11 minus 1 offset)
///   VIN3  CPU I/O (VCCIO)    × 1.0
///   VIN4  CPU SA (VCCSA)     × 1.0
///   VIN5  +5 V rail          × 3.0   (20k/10k divider → ×3)
///   VIN6  3VSB standby       × 1.0
///   VIN7  3.3 V rail         × 1.0
///   VIN8  AVCC (3.3 V ref)   × 1.0
const DIVIDER: [f32; 9] = [1.0, 1.0, 10.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0];

const CHANNEL_NAMES: [&str; 9] = [
    "CPU Vcore", "DRAM", "+12V", "CPU I/O", "CPU SA", "+5V", "3VSB", "+3.3V", "AVCC",
];

#[derive(Debug, Clone, Serialize)]
pub struct SuperIoVoltage {
    pub name: String,
    pub voltage_v: f32,
    pub channel: u8,
}

/// Try to read all VIN channels from an ITE IT87xx Super I/O chip.
/// Returns an empty vec if the chip is not found or the driver is unavailable.
pub fn read_ite_voltages(dm: &DriverManager) -> Vec<SuperIoVoltage> {
    for (addr, data) in [(0x2Eu16, 0x2Fu16), (0x4Eu16, 0x4Fu16)] {
        if let Some(v) = try_read_ite(dm, addr, data) {
            return v;
        }
    }
    vec![]
}

fn ite_cfg_read(dm: &DriverManager, addr: u16, data: u16, reg: u8) -> Option<u8> {
    dm.write_io_port_byte(addr, reg).ok()?;
    dm.read_io_port_byte(data).ok()
}

fn ite_cfg_write(dm: &DriverManager, addr: u16, data: u16, reg: u8, val: u8) -> Option<()> {
    dm.write_io_port_byte(addr, reg).ok()?;
    dm.write_io_port_byte(data, val).ok()
}

fn try_read_ite(dm: &DriverManager, addr: u16, data: u16) -> Option<Vec<SuperIoVoltage>> {
    // Enter ITE configuration mode (key: write 0x87 0x01 0x55 0x55 to addr port)
    dm.write_io_port_byte(addr, 0x87).ok()?;
    dm.write_io_port_byte(addr, 0x01).ok()?;
    dm.write_io_port_byte(addr, 0x55).ok()?;
    dm.write_io_port_byte(addr, 0x55).ok()?;

    let chip_high = ite_cfg_read(dm, addr, data, 0x20)?;
    let chip_low  = ite_cfg_read(dm, addr, data, 0x21)?;

    // All ITE chips have high byte 0x87
    if chip_high != 0x87 {
        let _ = dm.write_io_port_byte(addr, 0x02); // exit config
        return None;
    }

    // Select logical device 4 — hardware monitor
    ite_cfg_write(dm, addr, data, 0x07, 0x04)?;

    let base_high = ite_cfg_read(dm, addr, data, 0x60)?;
    let base_low  = ite_cfg_read(dm, addr, data, 0x61)?;
    let io_base   = ((base_high as u16) << 8) | base_low as u16;

    // Exit configuration mode
    let _ = dm.write_io_port_byte(addr, 0x02);

    if io_base < 0x100 || io_base == 0xFFFF {
        return None;
    }

    let chip_id = ((chip_high as u16) << 8) | chip_low as u16;
    eprintln!("[superio] ITE chip 0x{chip_id:04X} at 0x{addr:02X}, HWM base 0x{io_base:04X}");

    let mut out = Vec::new();
    for i in 0u8..9 {
        let raw = dm.read_io_port_byte(io_base + 0x20 + i as u16).ok()?;
        let voltage_v = raw as f32 * VOLT_LSB * DIVIDER[i as usize];
        out.push(SuperIoVoltage {
            name: CHANNEL_NAMES[i as usize].to_string(),
            voltage_v,
            channel: i,
        });
    }
    Some(out)
}
