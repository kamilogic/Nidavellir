mod gpu_apply;
mod gpu_benchmark;
mod gpu_forge_all;
mod gpu_mem_sweep;
mod gpu_power_sweep;
mod gpu_real;
mod gpu_sweep_real;
mod gpu_verify;
mod ipc_server;
mod safe_loop_runtime;
mod sensor_gather;
mod service_impl;

use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;
use windows_service::define_windows_service;

use gpu_benchmark::BenchmarkHandle;
use gpu_forge_all::ForgeAllHandle;
use gpu_mem_sweep::MemSweepHandle;
use gpu_power_sweep::PowerSweepHandle;
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
    pub forge_all: ForgeAllHandle,
    pub benchmark: BenchmarkHandle,
    pub power_sweep: PowerSweepHandle,
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
            "verify-applied" => return run_verify_only(),
            "build-frontier" => return run_build_frontier_cmd(&args),
            _ => {}
        }
    }

    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

/// Read-only diagnostic: classify the live VF curve against the applied profile and
/// print the result. Deliberately does NOT run startup recovery, the heartbeat,
/// `reapply_on_boot`, or the pipe server — so it performs **no apply, no reapply, and
/// no VF-curve write**. Safe to run while the GPU is at any state.
fn run_verify_only() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("verify-applied: read-only curve verification (no reapply, no VF write)");
    let status = gpu_verify::verify_applied_curve();
    // Structured result to stdout for headless QA (in addition to the apply_verify log).
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

/// `--confirm` present? Pure (unit-testable without hardware).
fn has_confirm_flag(args: &[OsString]) -> bool {
    args.iter().any(|a| a.to_string_lossy() == "--confirm")
}

/// Supervised console entry for the F1b multi-clock frontier (`build-frontier`). WITHOUT
/// `--confirm` it is a read-only DRY-RUN that only prints the plan (no hardware). WITH
/// `--confirm` it runs startup recovery (parachute) FIRST, then the real supervised hardware
/// frontier (transient VF ceilings + game-power dwells), always restoring stock. It never
/// applies or persists a profile.
fn run_build_frontier_cmd(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let confirm = has_confirm_flag(args);
    let store = SafeLoopStore::system();
    if confirm {
        tracing::warn!(
            "build-frontier: --confirm set — running startup recovery, then the SUPERVISED hardware frontier"
        );
        safe_loop_runtime::run_startup_recovery(&store);
    } else {
        tracing::info!("build-frontier: dry-run (pass --confirm to execute the supervised hardware run)");
    }
    gpu_power_sweep::run_build_frontier(&store, confirm);
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::has_confirm_flag;
    use std::ffi::OsString;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn confirm_flag_detected_only_when_present() {
        assert!(!has_confirm_flag(&os(&["build-frontier"])));
        assert!(has_confirm_flag(&os(&["build-frontier", "--confirm"])));
        assert!(has_confirm_flag(&os(&["build-frontier", "--confirm", "x"])));
        assert!(!has_confirm_flag(&os(&["build-frontier", "confirm"]))); // must be the flag form
    }
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
        forge_all: ForgeAllHandle::default(),
        benchmark: BenchmarkHandle::default(),
        // Seed from the persisted forge result so a restart restores forged
        // profiles/points instead of showing an unforged GPU.
        power_sweep: gpu_power_sweep::restore_handle(),
    }));
    ipc_server::run_pipe_server(state)?;
    Ok(())
}
