//! Print measured memory bandwidth (stock) — sanity check.
fn main() {
    match nidavellir_gpu_stress::GpuCtx::new() {
        Ok(ctx) => {
            println!("adapter: {}", ctx.adapter_name);
            let gbps = ctx.measure_bandwidth_gbps(2500);
            println!("bandwidth: {gbps:.1} GB/s");
        }
        Err(e) => eprintln!("erro: {e}"),
    }
}
