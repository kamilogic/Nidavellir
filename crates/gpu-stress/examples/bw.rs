//! Print measured memory bandwidth (stock) — sanity check.
fn main() {
    match nidavellir_gpu_stress::GpuCtx::new() {
        Ok(ctx) => {
            println!("adapter: {}", ctx.adapter_name);
            let gbps = ctx.measure_bandwidth_gbps(2500);
            println!("bandwidth: {gbps:.1} GB/s");
            let chase = ctx.run_mem_chase(2000);
            println!("chase: {} -> {:?} (mismatches {}, {} ms)", chase.name, chase.result, chase.mismatches, chase.elapsed_ms);
            let comb = ctx.run_combined(4000);
            println!("combined: {} -> {:?} (mismatches {}, {} ms)", comb.name, comb.result, comb.mismatches, comb.elapsed_ms);
        }
        Err(e) => eprintln!("erro: {e}"),
    }
}
