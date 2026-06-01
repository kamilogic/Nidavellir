//! Carefully-gated NVAPI **write** test (real hardware).
//!
//! Usage:
//!   apply noop            -> set core clock offset +0 MHz (proves the write call; changes nothing)
//!   apply offset <mhz>    -> apply a small core offset, hold 5s, then auto-reset to 0
//!   apply reset           -> force core offset back to 0 (safety)
//!
//! Always resets the offset to 0 before exiting.

#[cfg(windows)]
fn read_core_offset_khz(gpu: &nvapi::PhysicalGpu) -> Option<i32> {
    let pst = gpu.pstates().ok()?;
    let p0 = pst.pstates.iter().find(|p| p.id == nvapi::PState::P0)?;
    let g = p0
        .clocks
        .iter()
        .find(|c| c.domain() == nvapi::ClockDomain::Graphics)?;
    Some(g.frequency_delta().value.0)
}

#[cfg(windows)]
fn set_core_offset(gpu: &nvapi::PhysicalGpu, mhz: i32) -> Result<(), String> {
    let delta = nvapi::KilohertzDelta(mhz * 1000);
    gpu.set_pstates(std::iter::once((
        nvapi::PState::P0,
        nvapi::ClockDomain::Graphics,
        delta,
    )))
    .map_err(|e| format!("set_pstates failed: {e:?}"))
}

#[cfg(windows)]
fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "noop".into());
    let mhz: i32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(15)
        .clamp(-50, 50); // hard safety clamp for this test harness

    if let Err(e) = nvapi::initialize() {
        eprintln!("init failed: {e:?}");
        return;
    }
    let gpu = match nvapi::PhysicalGpu::enumerate() {
        Ok(g) => g.into_iter().next().unwrap(),
        Err(e) => {
            eprintln!("enumerate failed: {e:?}");
            return;
        }
    };

    println!("GPU: {}", gpu.full_name().unwrap_or_default());
    println!("offset antes: {:?} kHz", read_core_offset_khz(&gpu));

    match arg.as_str() {
        "noop" => {
            match set_core_offset(&gpu, 0) {
                Ok(()) => println!("set_pstates(+0 MHz) OK — escrita funciona, nada mudou"),
                Err(e) => println!("ESCRITA FALHOU: {e}"),
            }
            println!("offset depois: {:?} kHz", read_core_offset_khz(&gpu));
        }
        "offset" => {
            println!("aplicando +{mhz} MHz no core...");
            match set_core_offset(&gpu, mhz) {
                Ok(()) => {
                    println!("aplicado. offset agora: {:?} kHz", read_core_offset_khz(&gpu));
                    println!("segurando 5s (vigie estabilidade)...");
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                Err(e) => println!("ESCRITA FALHOU: {e}"),
            }
            // Always revert.
            match set_core_offset(&gpu, 0) {
                Ok(()) => println!("RESET para +0 OK. offset: {:?} kHz", read_core_offset_khz(&gpu)),
                Err(e) => println!("RESET FALHOU: {e}"),
            }
        }
        _ => {
            let _ = set_core_offset(&gpu, 0);
            println!("reset para +0. offset: {:?} kHz", read_core_offset_khz(&gpu));
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("NVAPI is Windows-only");
}
