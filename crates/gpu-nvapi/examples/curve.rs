//! Print the structured V/F curve and the detected plateau (flat-curve UV).

fn main() {
    match nidavellir_gpu_nvapi::read_curve() {
        Ok(curve) => {
            println!("GPU: {}", curve.name);
            println!("pontos da curva: {}", curve.points.len());
            // Print a compact view: every 8th point plus the plateau region.
            for (i, p) in curve.points.iter().enumerate() {
                if i % 8 == 0 {
                    println!("  {:>4} mV -> {:>4} MHz", p.voltage_mv, p.freq_mhz);
                }
            }
            match curve.plateau() {
                Some(pl) => println!("PLATEAU (UV travado): {} MHz @ {} mV", pl.freq_mhz, pl.voltage_mv),
                None => println!("sem plateau"),
            }
        }
        Err(e) => eprintln!("erro: {e}"),
    }
}
