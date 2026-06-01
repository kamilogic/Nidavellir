//! Real undervolt sweep (writes voltage/clock to the GPU).
//!
//!   uvsweep locktest [mv]      lock at `mv` (default 875), brief load, read back, unlock
//!   uvsweep run [mv] [capMHz]  lock at `mv`, raise core offset until the battery
//!                              flags instability; report max stable clock at `mv`
//!
//! WARNING: `run` pushes to the stability cliff on purpose — expect display
//! flickers / a TDR near the limit. Always resets (unlock + offset 0) on exit.

#[cfg(windows)]
mod imp {
    use nidavellir_core::gpu_sweep::StabilityResult;
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    fn worst(a: StabilityResult, b: StabilityResult) -> StabilityResult {
        use StabilityResult::*;
        match (a, b) {
            (Crash, _) | (_, Crash) => Crash,
            (SilentError, _) | (_, SilentError) => SilentError,
            _ => Stable,
        }
    }

    fn smi_clock_mhz() -> Option<u32> {
        let out = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=clocks.gr", "--format=csv,noheader,nounits"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()
    }

    /// Run `body` while sampling the core clock; returns the peak clock seen.
    fn with_clock_peak<R>(body: impl FnOnce() -> R) -> (R, u32) {
        let peak = Arc::new(AtomicU32::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (p2, s2) = (peak.clone(), stop.clone());
        let sampler = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                if let Some(c) = smi_clock_mhz() {
                    p2.fetch_max(c, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
        });
        let r = body();
        stop.store(true, Ordering::SeqCst);
        let _ = sampler.join();
        (r, peak.load(Ordering::SeqCst))
    }

    /// One short stability pass (load + validation) under the current V/F state.
    fn validate_step(ctx: &GpuCtx) -> StabilityResult {
        let a = ctx.run_alu("alu", 1_000_000, 1_000_000, 2500).result;
        if !a.is_stable() {
            return a;
        }
        let m = ctx.run_memory("mem", 262_144, 2_048, 2500).result;
        worst(a, m)
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        let mode = args.get(1).cloned().unwrap_or_else(|| "locktest".into());
        let mv: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(875);
        let cap: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(450);

        let ctx = match GpuCtx::new() {
            Ok(c) => c,
            Err(e) => return eprintln!("GPU init failed: {e}"),
        };
        println!("adapter: {}", ctx.adapter_name);

        // Always start clean.
        let _ = gpu::reset_all();

        match mode.as_str() {
            "locktest" => {
                println!("== LOCK TEST @ {mv} mV ==");
                if let Err(e) = gpu::lock_core_voltage_mv(mv) {
                    let _ = gpu::reset_all();
                    return eprintln!("lock failed: {e}");
                }
                let (_r, peak) = with_clock_peak(|| validate_step(&ctx));
                let vread = gpu::read_core_voltage_mv();
                println!("sob carga: clock pico = {peak} MHz, voltagem lida = {vread:?} mV");
                let _ = gpu::reset_all();
                println!("reset OK (unlock + offset 0)");
            }
            "run" => {
                println!("== UNDERVOLT SWEEP @ {mv} mV (cap +{cap} MHz) ==");
                if let Err(e) = gpu::lock_core_voltage_mv(mv) {
                    let _ = gpu::reset_all();
                    return eprintln!("lock failed: {e}");
                }
                let mut best_stable_mhz = 0u32;
                let mut offset = 0i32;
                let step = 15i32;
                loop {
                    if let Err(e) = gpu::set_core_offset_mhz(offset) {
                        eprintln!("set offset failed: {e}");
                        break;
                    }
                    let (result, peak) = with_clock_peak(|| validate_step(&ctx));
                    println!("  offset +{offset:>3} MHz -> clock {peak} MHz : {result:?}");
                    match result {
                        StabilityResult::Stable => {
                            best_stable_mhz = best_stable_mhz.max(peak);
                            offset += step;
                            if offset > cap {
                                println!("  atingiu o teto de offset (+{cap})");
                                break;
                            }
                        }
                        _ => {
                            println!("  >> cliff: instabilidade em +{offset} MHz");
                            break;
                        }
                    }
                }
                let _ = gpu::reset_all();
                println!("reset OK (unlock + offset 0)");
                println!("\nRESULTADO: clock máx estável @ {mv} mV = {best_stable_mhz} MHz");
                println!("seu manual: 1800 MHz @ 875 mV");
            }
            _ => eprintln!("modo desconhecido: {mode}"),
        }
    }
}

#[cfg(windows)]
fn main() {
    imp::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("NVAPI/undervolt is Windows-only");
}
