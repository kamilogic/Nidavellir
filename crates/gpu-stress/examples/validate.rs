//! Run the GPU compute-validation battery and print the verdict.

fn main() {
    let elements: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let iters: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(20_000);

    println!("Validando: {elements} lanes x {iters} iterações (KAT LCG)...");
    match nidavellir_gpu_stress::validate_kat(elements, iters) {
        Ok(r) => {
            println!("adapter:    {}", r.adapter);
            println!("resultado:  {:?}", r.result);
            println!("mismatches: {}", r.mismatches);
            println!("tempo:      {} ms", r.elapsed_ms);
        }
        Err(e) => eprintln!("erro: {e}"),
    }
}
