//! Memory (VRAM) sweep — finds the GDDR6 **effective-bandwidth peak**, not the
//! max stable clock. Past the peak, on-die ECC corrects errors and *eats*
//! bandwidth, so more MHz = less real throughput. We raise the memory clock
//! offset, measuring achieved bandwidth + integrity each step, and stop at the
//! first artifact or when bandwidth stops improving (the ECC wall).
//!
//! Windows-only. Safe Loop armed per step; device-lost caught; always resets.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::SweepPhase;
use nidavellir_core::ipc::{MemSweepPoint, MemSweepProgress};
use nidavellir_core::safe_loop::SafeLoopStore;
use tracing::{info, warn};

fn idle() -> MemSweepProgress {
    MemSweepProgress {
        phase: SweepPhase::Idle,
        running: false,
        current_offset_mhz: 0,
        current_mem_mhz: 0,
        current_gbps: 0.0,
        baseline_gbps: 0.0,
        points: Vec::new(),
        peak_offset_mhz: 0,
        peak_gbps: 0.0,
    }
}

#[derive(Clone)]
pub struct MemSweepHandle {
    progress: Arc<Mutex<MemSweepProgress>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for MemSweepHandle {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(idle())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl MemSweepHandle {
    pub fn progress(&self) -> MemSweepProgress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_else(|_| idle())
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn start(&self, store: SafeLoopStore) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let progress = Arc::clone(&self.progress);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            #[cfg(windows)]
            run_mem_sweep(progress, stop, store);
            #[cfg(not(windows))]
            {
                let _ = (&progress, &stop, &store);
            }
            running.store(false, Ordering::SeqCst);
        });
        true
    }
}

#[cfg(windows)]
fn mem_clock_mhz() -> u32 {
    nidavellir_core::nvml_gpu::read_nvidia_gpus_nvml()
        .into_iter()
        .next()
        .and_then(|r| r.memory_clock_mhz)
        .unwrap_or(0)
}

#[cfg(windows)]
fn run_mem_sweep(
    progress: Arc<Mutex<MemSweepProgress>>,
    stop: Arc<AtomicBool>,
    store: SafeLoopStore,
) {
    use nidavellir_core::safe_loop::{BootFlag, TuningPoint};
    use nidavellir_gpu_nvapi as gpu;
    use nidavellir_gpu_stress::GpuCtx;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let set = |p: &Arc<Mutex<MemSweepProgress>>, v: MemSweepProgress| {
        if let Ok(mut g) = p.lock() {
            *g = v;
        }
    };

    info!("Memory sweep starting (bandwidth-peak search)");
    let ctx = match GpuCtx::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("mem sweep: GpuCtx init failed: {e}");
            set(&progress, idle());
            return;
        }
    };

    let _ = gpu::reset_all();

    let mut prog = idle();
    prog.running = true;
    prog.phase = SweepPhase::Baseline;
    set(&progress, prog.clone());

    let baseline = ctx.measure_bandwidth_gbps(2000);
    prog.baseline_gbps = baseline as f32;
    prog.peak_gbps = baseline as f32;
    prog.peak_offset_mhz = 0;
    set(&progress, prog.clone());

    let step = 50i32;
    let cap = 1500i32;
    let mut best = baseline;
    let mut no_improve = 0u32;
    let mut crashed = false;
    let mut offset = step;

    prog.phase = SweepPhase::VoltageBisection; // (reused: "sweeping")
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let intent = TuningPoint::from_axes([("gpu_mem_offset_mhz", offset as i64)]);
        let _ = store.arm_boot_flag(&BootFlag::new(intent, "gpu_mem_sweep"));

        if let Err(e) = gpu::set_mem_offset_mhz(offset) {
            warn!("mem sweep: set offset failed: {e}");
            break;
        }

        // Integrity at this clock (catch artifacts), then bandwidth.
        let integ = match catch_unwind(AssertUnwindSafe(|| ctx.run_vram_check(512 * 1024 * 1024, 1))) {
            Ok(r) => r.result,
            Err(_) => {
                crashed = true;
                nidavellir_core::gpu_sweep::StabilityResult::Crash
            }
        };
        let gbps = if crashed { 0.0 } else { ctx.measure_bandwidth_gbps(1500) };
        let mem_mhz = mem_clock_mhz();
        let stable = integ.is_stable() && !crashed && gbps > 0.0;

        if stable {
            let _ = store.clear_boot_flag();
        }

        prog.current_offset_mhz = offset;
        prog.current_mem_mhz = mem_mhz;
        prog.current_gbps = gbps as f32;
        prog.points.push(MemSweepPoint {
            offset_mhz: offset,
            mem_mhz,
            bandwidth_gbps: gbps as f32,
            stable,
        });
        info!("mem sweep: +{offset}MHz -> {mem_mhz}MHz, {gbps:.1} GB/s, stable={stable}");

        if !stable {
            set(&progress, prog.clone());
            break; // artifacts / crash → cliff
        }

        if gbps > best * 1.002 {
            best = gbps;
            prog.peak_gbps = gbps as f32;
            prog.peak_offset_mhz = offset;
            no_improve = 0;
        } else {
            no_improve += 1;
            if no_improve >= 2 {
                info!("mem sweep: bandwidth peaked (ECC wall) at +{} MHz", prog.peak_offset_mhz);
                set(&progress, prog.clone());
                break;
            }
        }
        set(&progress, prog.clone());

        offset += step;
        if offset > cap {
            break;
        }
    }

    let _ = gpu::reset_all();
    let _ = store.clear_boot_flag();
    prog.running = false;
    prog.current_offset_mhz = 0;
    prog.phase = if crashed { SweepPhase::Aborted } else { SweepPhase::Done };
    set(&progress, prog);
    info!("Memory sweep finished (crashed={crashed})");
}
