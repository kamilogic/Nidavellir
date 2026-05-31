//! Read-only NVAPI dump — prints the live GPU state so we can confirm the
//! binding works on the real card before any write path is built.

#[cfg(windows)]
fn main() {
    if let Err(e) = nvapi::initialize() {
        eprintln!("NvAPI_Initialize failed: {e:?}");
        return;
    }
    let gpus = match nvapi::PhysicalGpu::enumerate() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("enumerate failed: {e:?}");
            return;
        }
    };
    println!("GPUs found: {}", gpus.len());

    for (i, gpu) in gpus.iter().enumerate() {
        println!("\n=== GPU {i} ===");
        dump("full_name", gpu.full_name());
        dump("current_pstate", gpu.current_pstate());
        dump("core_voltage", gpu.core_voltage());
        dump("power_limit", gpu.power_limit());
        dump("power_limit_info", gpu.power_limit_info());
        dump("pstates", gpu.pstates());
        dump("vfp_ranges", gpu.vfp_ranges());

        match gpu.vfp_mask() {
            Ok(mask) => {
                println!("vfp_mask = {mask:?}");
                dump("vfp_curve", gpu.vfp_curve(mask.mask));
            }
            Err(e) => println!("vfp_mask ERR: {e:?}"),
        }
    }
}

#[cfg(windows)]
fn dump<T: std::fmt::Debug, E: std::fmt::Debug>(label: &str, r: Result<T, E>) {
    match r {
        Ok(v) => println!("{label} = {v:?}"),
        Err(e) => println!("{label} ERR: {e:?}"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("NVAPI is Windows-only");
}
