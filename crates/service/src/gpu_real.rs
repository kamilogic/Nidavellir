//! Bridges the real GPU backends (NVAPI read, wgpu validation) into the service
//! IPC. Read-only / no voltage writes here — the cliff-finding sweep is a later,
//! explicitly-gated step. Validation runs on a background thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::{StabilityResult, VfPoint};
use nidavellir_core::ipc::{GpuCurveSnapshot, GpuStageResult, GpuValidationStatus};
use tracing::{info, warn};

/// Read the live V/F curve via NVAPI (real hardware).
#[cfg(windows)]
pub fn read_curve_snapshot() -> GpuCurveSnapshot {
    match nidavellir_gpu_nvapi::read_curve() {
        Ok(c) => {
            let points = c
                .points
                .iter()
                .map(|p| VfPoint { freq_mhz: p.freq_mhz, voltage_mv: p.voltage_mv })
                .collect();
            let plateau = c
                .plateau()
                .map(|p| VfPoint { freq_mhz: p.freq_mhz, voltage_mv: p.voltage_mv });
            GpuCurveSnapshot { name: c.name, points, plateau, real: true }
        }
        Err(e) => {
            warn!("NVAPI read_curve failed: {e}");
            GpuCurveSnapshot { name: format!("unavailable: {e}"), points: vec![], plateau: None, real: false }
        }
    }
}

#[cfg(not(windows))]
pub fn read_curve_snapshot() -> GpuCurveSnapshot {
    GpuCurveSnapshot { name: "NVAPI é só Windows".into(), points: vec![], plateau: None, real: false }
}

fn idle_validation() -> GpuValidationStatus {
    GpuValidationStatus {
        running: false,
        current_stage: None,
        stage_index: 0,
        total_stages: 0,
        stages: Vec::new(),
        result: None,
        adapter: None,
        error: None,
    }
}

/// Combine stage verdicts into an overall one (Crash worst, then SilentError).
fn worst(a: StabilityResult, b: StabilityResult) -> StabilityResult {
    use StabilityResult::*;
    match (a, b) {
        (Crash, _) | (_, Crash) => Crash,
        (SilentError, _) | (_, SilentError) => SilentError,
        _ => Stable,
    }
}

/// Background runner for the real GPU compute-validation battery.
#[derive(Clone)]
pub struct GpuValidationHandle {
    status: Arc<Mutex<GpuValidationStatus>>,
    running: Arc<AtomicBool>,
}

impl Default for GpuValidationHandle {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(idle_validation())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GpuValidationHandle {
    pub fn status(&self) -> GpuValidationStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_else(|_| idle_validation())
    }

    /// Start the validation battery if not already running. Returns false if busy.
    pub fn start(&self) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }

        // The battery: (label, runner). Each runner targets a failure mode.
        type Runner = Box<dyn Fn(&nidavellir_gpu_stress::GpuCtx) -> nidavellir_gpu_stress::StageReport + Send>;
        // Sustained loads that actually saturate the GPU; VRAM integrity first.
        let battery: Vec<(&'static str, Runner)> = vec![
            ("VRAM integrity", Box::new(|c| c.run_vram_check(4 * 1024 * 1024 * 1024, 2))),
            ("ALU (known-answer)", Box::new(|c| c.run_alu("ALU (known-answer)", 1_000_000, 1_000_000, 4000))),
            ("Memory (VRAM gather)", Box::new(|c| c.run_memory("Memory (VRAM gather)", 262_144, 4_096, 3500))),
            ("Mixed (ALU+mem)", Box::new(|c| c.run_alu("Mixed (ALU+mem)", 1_000_000, 1_000_000, 3000))),
        ];
        let total = battery.len() as u32;

        if let Ok(mut s) = self.status.lock() {
            *s = idle_validation();
            s.running = true;
            s.total_stages = total;
            s.current_stage = battery.first().map(|(n, _)| n.to_string());
        }

        let status = Arc::clone(&self.status);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            info!("GPU validation battery started ({total} estágios)");
            let ctx = match nidavellir_gpu_stress::GpuCtx::new() {
                Ok(c) => c,
                Err(e) => {
                    warn!("GPU validation: device init failed: {e}");
                    if let Ok(mut s) = status.lock() {
                        s.running = false;
                        s.error = Some(e);
                    }
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            };
            if let Ok(mut s) = status.lock() {
                s.adapter = Some(ctx.adapter_name.clone());
            }

            let mut overall = StabilityResult::Stable;
            for (idx, (name, run)) in battery.iter().enumerate() {
                if let Ok(mut s) = status.lock() {
                    s.stage_index = idx as u32;
                    s.current_stage = Some(name.to_string());
                }
                let report = run(&ctx);
                overall = worst(overall, report.result);
                if let Ok(mut s) = status.lock() {
                    s.stages.push(GpuStageResult {
                        name: report.name,
                        result: report.result,
                        mismatches: report.mismatches,
                        elapsed_ms: report.elapsed_ms,
                    });
                }
            }

            if let Ok(mut s) = status.lock() {
                s.running = false;
                s.current_stage = None;
                s.stage_index = total;
                s.result = Some(overall);
            }
            running.store(false, Ordering::SeqCst);
            info!("GPU validation battery finished: {overall:?}");
        });
        true
    }
}
