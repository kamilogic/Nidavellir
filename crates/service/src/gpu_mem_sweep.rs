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
        validation_note: None,
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

        // Publish the running step so the UI shows what's executing now.
        prog.current_offset_mhz = offset;
        prog.current_gbps = 0.0;
        prog.validation_note = Some(format!("Testando +{offset} MHz · integridade (chase 8s)…"));
        set(&progress, prog.clone());

        // Pointer-chase integrity (sensitive to uncorrected errors), then bandwidth.
        // Longer dwell to give marginal errors time to surface at this clock.
        let integ = match catch_unwind(AssertUnwindSafe(|| ctx.run_mem_chase(8000))) {
            Ok(r) => r.result,
            Err(_) => {
                crashed = true;
                nidavellir_core::gpu_sweep::StabilityResult::Crash
            }
        };
        if !crashed {
            prog.validation_note = Some(format!("Testando +{offset} MHz · banda…"));
            set(&progress, prog.clone());
        }
        let gbps = if crashed { 0.0 } else { ctx.measure_bandwidth_gbps(3500) };
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

        // Knee detection: an *additive* improvement threshold. Once each step
        // stops adding real bandwidth (CRC retries eating the gain), we're at
        // the ECC wall — stop and recommend the knee, not the flat top.
        if gbps > best + 2.0 {
            best = gbps;
            prog.peak_gbps = gbps as f32;
            prog.peak_offset_mhz = offset;
            no_improve = 0;
        } else {
            no_improve += 1;
            if no_improve >= 3 {
                info!("mem sweep: bandwidth knee (CRC wall) — peak at +{} MHz", prog.peak_offset_mhz);
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

    // Phase E — arduous soak at the recommended (peak) offset to confirm it.
    if !crashed && !stop.load(Ordering::SeqCst) && prog.peak_offset_mhz > 0 {
        prog.validation_note = Some("Validação longa em andamento…".into());
        set(&progress, prog.clone());
        let off = prog.peak_offset_mhz;
        let _ = store.arm_boot_flag(&BootFlag::new(
            TuningPoint::from_axes([("gpu_mem_offset_mhz", off as i64)]),
            "gpu_mem_validate",
        ));
        let _ = gpu::set_mem_offset_mhz(off);
        let res = match catch_unwind(AssertUnwindSafe(|| ctx.run_mem_chase(90_000))) {
            Ok(r) => r.result,
            Err(_) => {
                crashed = true;
                nidavellir_core::gpu_sweep::StabilityResult::Crash
            }
        };
        prog.validation_note = Some(if res.is_stable() {
            let _ = store.clear_boot_flag();
            format!("Validado: +{off} MHz estável no soak de 90s — confirme em jogo")
        } else {
            "Falhou no soak longo — recue o offset de memória".into()
        });
        set(&progress, prog.clone());
    }

    let _ = gpu::reset_all();
    let _ = store.clear_boot_flag();
    prog.running = false;
    prog.current_offset_mhz = 0;
    prog.phase = if crashed { SweepPhase::Aborted } else { SweepPhase::Done };
    set(&progress, prog);
    info!("Memory sweep finished (crashed={crashed})");
}
