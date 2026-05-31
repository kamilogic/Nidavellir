//! Bridges the real GPU backends (NVAPI read, wgpu validation) into the service
//! IPC. Read-only / no voltage writes here — the cliff-finding sweep is a later,
//! explicitly-gated step. Validation runs on a background thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::gpu_sweep::VfPoint;
use nidavellir_core::ipc::{GpuCurveSnapshot, GpuValidationStatus};
use tracing::{info, warn};

/// Validation workload size (debug builds are slow; release is far faster).
const VAL_ELEMENTS: u32 = 500_000;
const VAL_ITERS: u32 = 8_000;

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
            GpuCurveSnapshot { name: format!("indisponível: {e}"), points: vec![], plateau: None, real: false }
        }
    }
}

#[cfg(not(windows))]
pub fn read_curve_snapshot() -> GpuCurveSnapshot {
    GpuCurveSnapshot { name: "NVAPI é só Windows".into(), points: vec![], plateau: None, real: false }
}

fn idle_validation() -> GpuValidationStatus {
    GpuValidationStatus { running: false, result: None, mismatches: 0, elapsed_ms: 0, adapter: None }
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

    /// Start a validation run if not already running. Returns false if busy.
    pub fn start(&self) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        if let Ok(mut s) = self.status.lock() {
            *s = GpuValidationStatus { running: true, result: None, mismatches: 0, elapsed_ms: 0, adapter: None };
        }
        let status = Arc::clone(&self.status);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            info!("GPU validation started ({VAL_ELEMENTS} lanes x {VAL_ITERS} iters)");
            let out = nidavellir_gpu_stress::validate_kat(VAL_ELEMENTS, VAL_ITERS);
            if let Ok(mut s) = status.lock() {
                *s = match out {
                    Ok(r) => GpuValidationStatus {
                        running: false,
                        result: Some(r.result),
                        mismatches: r.mismatches,
                        elapsed_ms: r.elapsed_ms as u64,
                        adapter: Some(r.adapter),
                    },
                    Err(e) => {
                        warn!("GPU validation error: {e}");
                        GpuValidationStatus { running: false, result: None, mismatches: 0, elapsed_ms: 0, adapter: Some(format!("erro: {e}")) }
                    }
                };
            }
            running.store(false, Ordering::SeqCst);
            info!("GPU validation finished");
        });
        true
    }
}
