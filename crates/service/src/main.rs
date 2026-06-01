mod gpu_apply;
mod gpu_mem_sweep;
mod gpu_real;
mod gpu_sweep_real;
mod ipc_server;
mod safe_loop_runtime;
mod sensor_gather;
mod service_impl;

use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;
use windows_service::define_windows_service;

use gpu_mem_sweep::MemSweepHandle;
use gpu_real::GpuValidationHandle;
use gpu_sweep_real::RealSweepHandle;
use nidavellir_core::safe_loop::SafeLoopStore;
use nidavellir_driver_pawnio::DriverManager;

pub const SERVICE_NAME: &str = "NidavellirCore";
pub const PIPE_NAME: &str = r"\\.\pipe\NidavellirCore";

pub struct AppState {
    pub driver: DriverManager,
    pub sensor_engine: nidavellir_core::sensors::SensorEngine,
    pub motherboard: nidavellir_core::detector::MotherboardInfo,
    pub safe_store: SafeLoopStore,
    pub gpu_validation: GpuValidationHandle,
    pub real_sweep: RealSweepHandle,
    pub mem_sweep: MemSweepHandle,
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = service_impl::run_service() {
        tracing::error!("Service failed: {e}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nidavellir=info".parse()?))
        .init();

    let args: Vec<OsString> = std::env::args_os().collect();
    if args.len() > 1 {
        match args[1].to_string_lossy().as_ref() {
            "run" | "console" => return run_standalone(),
            _ => {}
        }
    }

    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

fn run_standalone() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Starting Nidavellir Core Service in console mode");

    // Parachute first: recover from any prior crash before doing anything else.
    let safe_store = SafeLoopStore::system();
    safe_loop_runtime::run_startup_recovery(&safe_store);
    safe_loop_runtime::spawn_heartbeat(safe_store.clone());
    // Re-apply the persisted GPU profile (volatile offsets) unless a prior
    // crash/Safe Mode says not to.
    gpu_apply::reapply_on_boot(&safe_store);

    let hw = nidavellir_core::detect_hardware();
    let state = Arc::new(Mutex::new(AppState {
        driver: DriverManager::new(),
        sensor_engine: nidavellir_core::sensors::SensorEngine::new(),
        motherboard: hw.motherboard,
        safe_store,
        gpu_validation: GpuValidationHandle::default(),
        real_sweep: RealSweepHandle::default(),
        mem_sweep: MemSweepHandle::default(),
    }));
    ipc_server::run_pipe_server(state)?;
    Ok(())
}
