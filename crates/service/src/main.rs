mod gpu_apply;
mod gpu_benchmark;
mod gpu_f2_sweep;
mod gpu_forge_all;
mod gpu_mem_sweep;
mod gpu_power_sweep;
mod gpu_real;
mod gpu_sweep_real;
mod gpu_undervolt;
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
            "undervolt-probe" => return run_undervolt_probe_cmd(&args),
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

/// `--help` / `-h` present? Pure (unit-testable without hardware).
fn has_help_flag(args: &[OsString]) -> bool {
    args.iter().any(|a| {
        let s = a.to_string_lossy();
        s == "--help" || s == "-h"
    })
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
            // Opt-in F1b bind-seeking v1 (no value; default OFF): stop a target at the first
            // verified+stable binding point instead of walking a fixed number of bins.
            "--bind-seeking" => {
                limits.bind_seeking = true;
                i += 1;
            }
            // Opt-in F1c power-bound knee-seeking (no value; default OFF): after a Phase-A power-bound
            // collapse, run a focused Phase-B deep descent to find the VF knee.
            "--power-bound-knee-seeking" => {
                limits.power_bound_knee_seeking = true;
                i += 1;
            }
            // Phase-B deep-descent budget (value; default None → built-in default when knee-seeking
            // is on). Only bounds the focused descent depth; the global --max-probes stays the cap.
            "--phase-b-probes" => {
                let v = strs.get(i + 1).ok_or_else(|| "--phase-b-probes needs a value".to_string())?;
                limits.phase_b_probes =
                    Some(v.parse().map_err(|_| format!("--phase-b-probes: invalid number '{v}'"))?);
                i += 2;
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

/// Supervised console entry for the F2 `undervolt-probe`. `--help`/`-h` prints usage and exits
/// (no hardware, no plan, no Safe Loop access). WITHOUT `--confirm` it is a read-only DRY-RUN that
/// prints the plan (no hardware). WITH `--confirm` it runs startup recovery FIRST, then a supervised
/// F2 run: anchored `--steps 1` is ONE single step; anchored `--steps 2..=3` is a bounded same-target
/// multi-step descent that stops at the first non-stable candidate (`--simple` stays single-step);
/// `--manual-prior` (opt-in dev/known-GPU shortcut, requires `--start-mv`, single-step) anchors at the
/// explicit voltage with a separate larger bounded offset cap. Confirmed mode can write bounded
/// positive VF offsets and TDR/reboot. It never persists, applies, or promotes a profile.
fn run_undervolt_probe_cmd(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    // Help short-circuits BEFORE anything else — no hardware read, no plan, no Safe Loop access.
    if has_help_flag(args) {
        println!("{}", gpu_undervolt::undervolt_usage());
        return Ok(());
    }
    let confirm = has_confirm_flag(args);
    let parsed = match gpu_undervolt::parse_undervolt_args(args) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("undervolt-probe: invalid flags: {e}");
            println!("undervolt-probe: invalid flags: {e}");
            return Ok(()); // clean exit, no hardware
        }
    };
    let store = SafeLoopStore::system();
    if confirm {
        tracing::warn!(
            "undervolt-probe: --confirm set — running startup recovery, then ONE supervised F2 single \
             step (may write a bounded positive VF offset; can TDR/reboot)"
        );
        safe_loop_runtime::run_startup_recovery(&store);
    } else {
        tracing::info!(
            "undervolt-probe: dry-run (pass `--steps 1 --confirm` to execute one supervised single step)"
        );
    }
    gpu_undervolt::run_undervolt_probe(&store, confirm, parsed);
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
    fn help_flag_detected_for_long_and_short() {
        assert!(!super::has_help_flag(&os(&["undervolt-probe"])));
        assert!(super::has_help_flag(&os(&["undervolt-probe", "--help"])));
        assert!(super::has_help_flag(&os(&["undervolt-probe", "-h"])));
        assert!(!super::has_help_flag(&os(&["undervolt-probe", "--target-mhz", "1800"])));
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
        assert!(!l.bind_seeking); // opt-in: default OFF
    }

    #[test]
    fn parse_bind_seeking_flag_opt_in() {
        // Absent → off; present → on. No value; other flags unaffected; warm-start stays off.
        assert!(!super::parse_frontier_limits(&os(&["build-frontier"])).unwrap().bind_seeking);
        let l = super::parse_frontier_limits(&os(&[
            "build-frontier", "--bind-seeking", "--max-targets", "7", "--max-probes-per-target", "3",
        ]))
        .unwrap();
        assert!(l.bind_seeking);
        assert!(!l.warm_start_brackets); // bind-seeking does NOT enable warm-start
        assert_eq!(l.max_targets, Some(7));
        assert_eq!(l.max_probes_per_target, Some(3));
    }

    #[test]
    fn parse_power_bound_knee_seeking_flags_opt_in() {
        // Both default OFF/None; the flag is valueless, --phase-b-probes takes a number.
        let def = super::parse_frontier_limits(&os(&["build-frontier"])).unwrap();
        assert!(!def.power_bound_knee_seeking);
        assert_eq!(def.phase_b_probes, None);
        let l = super::parse_frontier_limits(&os(&[
            "build-frontier", "--power-bound-knee-seeking", "--phase-b-probes", "12",
            "--max-probes-per-target", "3",
        ]))
        .unwrap();
        assert!(l.power_bound_knee_seeking);
        assert_eq!(l.phase_b_probes, Some(12));
        assert_eq!(l.max_probes_per_target, Some(3)); // Phase A cap unaffected
        assert!(!l.bind_seeking); // independent of bind-seeking
        // Missing / non-numeric value fails closed.
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--phase-b-probes"])).is_err());
        assert!(super::parse_frontier_limits(&os(&["build-frontier", "--phase-b-probes", "x"])).is_err());
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
