use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct StressConfig {
    pub duration_secs: u64,
    pub cpu_threads: u32,
}

impl Default for StressConfig {
    fn default() -> Self {
        Self { duration_secs: 30, cpu_threads: 1 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StressMetrics {
    pub total_operations: u64,
    pub throughput_ops_per_sec: f64,
    pub threads_completed: u32,
}

pub struct StressTest {
    active: Arc<AtomicBool>,
}

impl Default for StressTest {
    fn default() -> Self {
        Self::new()
    }
}

impl StressTest {
    pub fn new() -> Self {
        Self { active: Arc::new(AtomicBool::new(false)) }
    }

    pub fn run(&self, config: &StressConfig) -> StressMetrics {
        self.active.store(true, Ordering::SeqCst);
        let active = self.active.clone();
        let total_ops = Arc::new(AtomicU64::new(0));
        let start = Instant::now();

        let handles: Vec<_> = (0..config.cpu_threads)
            .map(|tid| {
                let active = active.clone();
                let total_ops = Arc::clone(&total_ops);
                std::thread::spawn(move || {
                    let mut x = tid as f64 * 1.001;
                    let mut ops: u64 = 0;
                    while active.load(Ordering::SeqCst) {
                        for _ in 0..2048 {
                            x = (x * 3.141592653589793).sin();
                            x = x.mul_add(1.41421356237, 0.5772156649);
                            x = x.sqrt().mul_add(x, x.recip());
                        }
                        ops += 2048;
                        if ops % 81920 == 0 {
                            total_ops.fetch_add(81920, Ordering::Relaxed);
                            ops = 0;
                        }
                    }
                    if ops > 0 {
                        total_ops.fetch_add(ops, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        std::thread::sleep(std::time::Duration::from_secs(config.duration_secs));
        self.active.store(false, Ordering::SeqCst);
        let elapsed = start.elapsed().as_secs_f64();

        let final_ops = total_ops.load(Ordering::Relaxed);
        for h in handles {
            let _ = h.join();
        }

        StressMetrics {
            total_operations: final_ops,
            throughput_ops_per_sec: if elapsed > 0.0 { final_ops as f64 / elapsed } else { 0.0 },
            threads_completed: config.cpu_threads,
        }
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}
