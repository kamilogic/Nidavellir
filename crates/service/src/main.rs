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

/// Parse the first-run limiter flags (`--max-targets N`, `--max-probes N`,
/// `--max-probes-per-target N`, `--safe-start-cap MV`). Syntax-only: missing/non-numeric values
/// FAIL CLOSED (`Err`). Semantic checks (0 values, cap vs crash floor) happen in
/// `gpu_power_sweep::run_build_frontier`. Pure (unit-testable).
fn parse_frontier_limits(args: &[OsString]) -> Result<gpu_power_sweep::FrontierLimits, String> {
    let strs: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let mut limits = gpu_power_sweep::FrontierLimits::default();
    let mut i = 0;
    while i < strs.len() {
        match strs[i].as_str() {
            "--max-targets" => {
                let v = strs.get(i + 1).ok_or_else(|| "--max-targets needs a value".to_string())?;
                limits.max_targets =
                    Some(v.parse().map_err(|_| format!("--max-targets: invalid number '{v}'"))?);
                i += 2;
            }
            "--max-probes" => {
                let v = strs.get(i + 1).ok_or_else(|| "--max-probes needs a value".to_string())?;
                limits.max_probes =
                    Some(v.parse().map_err(|_| format!("--max-probes: invalid number '{v}'"))?);
                i += 2;
            }
            "--max-probes-per-target" => {
                let v = strs
                    .get(i + 1)
                    .ok_or_else(|| "--max-probes-per-target needs a value".to_string())?;
                limits.max_probes_per_target = Some(
                    v.parse().map_err(|_| format!("--max-probes-per-target: invalid number '{v}'"))?,
                );
                i += 2;
            }
            "--safe-start-cap" => {
                let v = strs.get(i + 1).ok_or_else(|| "--safe-start-cap needs a value".to_string())?;
                limits.safe_start_cap_mv =
                    Some(v.parse().map_err(|_| format!("--safe-start-cap: invalid number '{v}'"))?);
                i += 2;
            }
            // Opt-in warm-start voltage-bracket carry-forward (no value; default OFF).
            "--warm-start-brackets" => {
                limits.warm_start_brackets = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(limits)
}

/// Supervised console entry for the F1b multi-clock frontier (`build-frontier`). WITHOUT
/// `--confirm` it is a read-only DRY-RUN that only prints the plan (no hardware). WITH
/// `--confirm` it runs startup recovery (parachute) FIRST, then the real supervised hardware
/// frontier (transient VF ceilings + game-power dwells), always restoring stock. It never
/// applies or persists a profile.
fn run_build_frontier_cmd(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let confirm = has_confirm_flag(args);
    let limits = match parse_frontier_limits(args) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("build-frontier: invalid flags: {e}");
            println!("build-frontier: invalid flags: {e}");
            return Ok(()); // clean exit, no hardware
        }
    };
    let store = SafeLoopStore::system();
    if confirm {
        tracing::warn!(
            "build-frontier: --confirm set — running startup recovery, then the SUPERVISED hardware frontier"
        );
        safe_loop_runtime::run_startup_recovery(&store);
    } else {
        tracing::info!("build-frontier: dry-run (pass --confirm to execute the supervised hardware run)");
    }
    gpu_power_sweep::run_build_frontier(&store, confirm, limits);
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

    #[test]
    fn parse_limits_reads_all_flags() {
        let l = super::parse_frontier_limits(&os(&[
            "build-frontier", "--max-targets", "1", "--max-probes", "6", "--safe-start-cap", "1075",
        ]))
        .unwrap();
        assert_eq!(l.max_targets, Some(1));
        assert_eq!(l.max_probes, Some(6));
        assert_eq!(l.safe_start_cap_mv, Some(1075));
    }

    #[test]
    fn parse_limits_defaults_when_absent() {
        let l = super::parse_frontier_limits(&os(&["build-frontier"])).unwrap();
        assert_eq!(l, crate::gpu_power_sweep::FrontierLimits::default());
        assert!(!l.warm_start_brackets); // opt-in: default OFF
    }

    #[test]
    fn parse_warm_start_brackets_flag_opt_in() {
        // Absent → off; present → on. Existing flags unaffected.
        assert!(!super::parse_frontier_limits(&os(&["build-frontier"])).unwrap().warm_start_brackets);
        let l = super::parse_frontier_limits(&os(&[
            "build-frontier", "--warm-start-brackets", "--max-targets", "3",
        ]))
        .unwrap();
        assert!(l.warm_start_brackets);
        assert_eq!(l.max_targets, Some(3));
    }

    #[test]
    fn parse_limits_rejects_nonnumeric_and_missing_value() {
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--max-targets", "abc"])).is_err());
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--max-probes"])).is_err());
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--safe-start-cap", "x"])).is_err());
    }

    #[test]
    fn parse_max_probes_per_target_flag() {
        // Present → parsed; absent → None; missing/non-numeric value → error.
        let l = super::parse_frontier_limits(&os(&[
            "build-frontier", "--max-targets", "7", "--max-probes", "14", "--max-probes-per-target", "2",
        ]))
        .unwrap();
        assert_eq!(l.max_probes_per_target, Some(2));
        assert_eq!(l.max_targets, Some(7));
        assert_eq!(l.max_probes, Some(14));
        assert_eq!(
            super::parse_frontier_limits(&os(&["build-frontier"])).unwrap().max_probes_per_target,
            None
        );
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--max-probes-per-target"])).is_err());
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--max-probes-per-target", "x"])).is_err());
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
