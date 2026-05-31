//! Run the full GPU compute-validation battery and print each stage.

fn main() {
    let ctx = match nidavellir_gpu_stress::GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("erro: {e}");
            return;
        }
    };
    println!("adapter: {}", ctx.adapter_name);

    let stages = [
        ctx.run_alu("ALU (known-answer)", 1_000_000, 20_000, 1),
        ctx.run_memory("Memory-bound", 500_000, 4_096),
        ctx.run_alu("Rajada (transiente)", 1_000_000, 2_500, 8),
    ];
    for s in &stages {
        println!(
            "  {:<22} -> {:?}  (mismatches {}, {} ms)",
            s.name, s.result, s.mismatches, s.elapsed_ms
        );
    }
}
