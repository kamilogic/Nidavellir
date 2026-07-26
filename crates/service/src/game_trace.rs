//! Read-only game-workload telemetry logger.
//!
//! Streams high-rate NVML samples (+ a slower NVAPI core-voltage sample) to a JSONL trace while the
//! operator plays — e.g. the Overwatch lobby that kills undervolts our synthetic gate passes. The
//! goal is to capture the *macroscopic fingerprint* of the real workload — power envelope and its
//! rate of change |dP/dt| (the software proxy for dI/dt aggression), clock/voltage BIN residency,
//! throttle reasons — so it can be diffed against our BoostEdge dwell and used to harden it.
//!
//! The actual failure mechanism (ns-scale transient voltage droop) is NOT software-observable; this
//! records its cause, never the droop itself. Performs NO hardware writes — pure read-only.
//!
//! Two entry points share one sampling loop ([`run_sampling`]): the `game-trace` CLI subcommand
//! ([`run`]) and the UI-controlled background task ([`GameTraceHandle`]).

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nidavellir_core::ipc::GameTraceStatus;
use nidavellir_core::nvml_gpu::{NvmlSample, NvmlSampler};

static CLI_STOP: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn ctrl_handler(_event: u32) -> windows::Win32::Foundation::BOOL {
    CLI_STOP.store(true, Ordering::SeqCst);
    // Handled: stop the loop cooperatively (no hardware to restore — this tool only reads).
    true.into()
}

struct Config {
    out: PathBuf,
    secs: u64,
    interval_ms: u64,
    volt_ms: u64,
}

impl Config {
    /// Defaults used by the UI-controlled background task (10 ms NVML, 200 ms voltage, unbounded).
    fn for_handle() -> Self {
        Self {
            out: default_out(),
            secs: 0,
            interval_ms: 10,
            volt_ms: 200,
        }
    }
}

fn epoch_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn default_out() -> PathBuf {
    PathBuf::from(format!(
        "C:\\ProgramData\\Nidavellir\\game-trace-{}.jsonl",
        epoch_ms()
    ))
}

/// Parse `--out <path>`, `--secs <N>` (0 = until Ctrl+C), `--interval-ms <N>`, `--volt-ms <N>`.
/// Pure so it is unit-testable without hardware. Missing/non-numeric numeric values fail closed.
fn parse_config(args: &[OsString], default: PathBuf) -> Result<Config, String> {
    let strs: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut cfg = Config {
        out: default,
        secs: 0,
        interval_ms: 10,
        volt_ms: 200,
    };
    let mut i = 0;
    while i < strs.len() {
        let take = |i: usize| -> Result<&String, String> {
            strs.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", strs[i]))
        };
        let num = |i: usize| -> Result<u64, String> {
            take(i)?
                .parse::<u64>()
                .map_err(|_| format!("{} needs a number", strs[i]))
        };
        match strs[i].as_str() {
            "--out" => {
                cfg.out = PathBuf::from(take(i)?);
                i += 1;
            }
            "--secs" => {
                cfg.secs = num(i)?;
                i += 1;
            }
            "--interval-ms" => {
                cfg.interval_ms = num(i)?.max(1);
                i += 1;
            }
            "--volt-ms" => {
                cfg.volt_ms = num(i)?.max(1);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(cfg)
}

#[derive(serde::Serialize)]
struct Row {
    #[serde(flatten)]
    s: NvmlSample,
    /// Wall-clock correlation with Windows event logs and external game/crash timestamps.
    wall_epoch_ms: u128,
    /// Actual time since the preceding row. A large gap is a pre-hang/TDR precursor.
    sample_gap_ms: u64,
    /// False when the persistent NVML handle returned no usable GPU telemetry for this row.
    nvml_sample_valid: bool,
    /// NVAPI core voltage (mV), sampled at `--volt-ms`; carried forward between fresh reads.
    volt_mv: Option<u32>,
    /// True only when this row contains a successful new voltage read.
    volt_fresh: bool,
    /// True when a voltage read was attempted, including attempts that failed and carried old data.
    volt_attempted: bool,
    /// True while the live Sentinel owns its independent TextureRop context for this sample.
    sentinel_canary_active: bool,
    /// Monotonic canary-attempt id, retained between attempts for exact trace correlation.
    sentinel_canary_sequence: u64,
}

/// The shared sampling loop. Opens the JSONL trace, streams samples until `stop` or the optional
/// duration, and reports live status via `on_status`. Read-only; performs no hardware writes.
fn run_sampling(
    cfg: &Config,
    stop: &AtomicBool,
    mut on_status: impl FnMut(GameTraceStatus),
) -> Result<u64, String> {
    let (sampler, power_limit_w) = NvmlSampler::init(0)?;
    let gpu = sampler.gpu_name().unwrap_or_else(|| "unknown".into());

    if let Some(parent) = cfg.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file = std::fs::File::create(&cfg.out)
        .map_err(|e| format!("create {}: {e}", cfg.out.display()))?;
    let mut w = std::io::BufWriter::new(file);
    let tdr_before = crate::tdr_sentinel::query_latest_tdr_event();
    let live_vf_curve: Vec<_> = nidavellir_gpu_nvapi::read_vf_curve_modern()
        .into_iter()
        .map(|(index, voltage_mv, freq_mhz)| {
            serde_json::json!({
                "index": index,
                "voltage_mv": voltage_mv,
                "freq_mhz": freq_mhz,
            })
        })
        .collect();
    let initial_voltage_mv = nidavellir_gpu_nvapi::read_core_voltage_mv();
    let (_, canary_sequence_before) = crate::tdr_sentinel::canary_trace_marker();

    let meta = serde_json::json!({
        "meta": true,
        "trace_contract": "game-trace-v3",
        "gpu": gpu,
        "power_limit_w": power_limit_w,
        "interval_ms": cfg.interval_ms,
        "volt_ms": cfg.volt_ms,
        "started_epoch_ms": epoch_ms(),
        "initial_voltage_mv": initial_voltage_mv,
        "tdr_event_before": tdr_before.clone(),
        "sentinel_canary_sequence_before": canary_sequence_before,
        "live_vf_curve": live_vf_curve,
        "note": "read-only NVML+NVAPI game workload trace with live Sentinel canary correlation",
    });
    writeln!(w, "{meta}").map_err(|e| format!("write meta: {e}"))?;
    w.flush().map_err(|e| format!("flush meta: {e}"))?;

    let out_str = cfg.out.display().to_string();
    tracing::info!(
        "game-trace: {gpu} — logging every {} ms (voltage every {} ms) to {out_str}",
        cfg.interval_ms,
        cfg.volt_ms
    );

    let mut volt_mv = initial_voltage_mv;
    let mut last_volt = std::time::Instant::now();
    let mut rows: u64 = 0;
    let start = std::time::Instant::now();
    let mut last_row_at = start;
    let mut max_sample_gap_ms = 0u64;
    let mut missing_nvml_samples = 0u64;
    let mut missing_nvml_streak = 0u64;
    let mut max_missing_nvml_streak = 0u64;
    let mut voltage_fresh_samples = 0u64;
    let mut voltage_read_failures = 0u64;
    let mut voltage_min_mv: Option<u32> = None;
    let mut voltage_max_mv: Option<u32> = None;
    let mut power_max_w: Option<f32> = None;
    let mut core_min_mhz: Option<u32> = None;
    let mut core_max_mhz: Option<u32> = None;

    while !stop.load(Ordering::SeqCst) {
        if cfg.secs > 0 && start.elapsed().as_secs() >= cfg.secs {
            break;
        }
        let s: NvmlSample = sampler.sample();
        let row_at = std::time::Instant::now();
        let sample_gap_ms = if rows == 0 {
            0
        } else {
            row_at.duration_since(last_row_at).as_millis() as u64
        };
        last_row_at = row_at;
        max_sample_gap_ms = max_sample_gap_ms.max(sample_gap_ms);
        let nvml_sample_valid = s.power_w.is_some()
            || s.core_mhz.is_some()
            || s.mem_mhz.is_some()
            || s.util_pct.is_some()
            || s.temp_c.is_some();
        if nvml_sample_valid {
            missing_nvml_streak = 0;
        } else {
            missing_nvml_samples += 1;
            missing_nvml_streak += 1;
            max_missing_nvml_streak = max_missing_nvml_streak.max(missing_nvml_streak);
        }
        if let Some(power_w) = s.power_w {
            power_max_w = Some(power_max_w.map_or(power_w, |value| value.max(power_w)));
        }
        if let Some(core_mhz) = s.core_mhz {
            core_min_mhz = Some(core_min_mhz.map_or(core_mhz, |value| value.min(core_mhz)));
            core_max_mhz = Some(core_max_mhz.map_or(core_mhz, |value| value.max(core_mhz)));
        }

        let volt_attempted = last_volt.elapsed().as_millis() as u64 >= cfg.volt_ms;
        let mut volt_fresh = false;
        if volt_attempted {
            last_volt = std::time::Instant::now();
            if let Some(fresh_mv) = nidavellir_gpu_nvapi::read_core_voltage_mv() {
                volt_mv = Some(fresh_mv);
                volt_fresh = true;
                voltage_fresh_samples += 1;
                voltage_min_mv = Some(voltage_min_mv.map_or(fresh_mv, |value| value.min(fresh_mv)));
                voltage_max_mv = Some(voltage_max_mv.map_or(fresh_mv, |value| value.max(fresh_mv)));
            } else {
                voltage_read_failures += 1;
            }
        }
        let (sentinel_canary_active, sentinel_canary_sequence) =
            crate::tdr_sentinel::canary_trace_marker();

        writeln!(
            w,
            "{}",
            serde_json::to_string(&Row {
                s,
                wall_epoch_ms: epoch_ms(),
                sample_gap_ms,
                nvml_sample_valid,
                volt_mv,
                volt_fresh,
                volt_attempted,
                sentinel_canary_active,
                sentinel_canary_sequence,
            })
            .map_err(|e| e.to_string())?
        )
        .map_err(|e| format!("write row: {e}"))?;
        rows += 1;
        // Preserve the lead-up to a TDR/reset instead of leaving two seconds in the userspace buffer.
        if rows.is_multiple_of(50) {
            w.flush().ok();
        }

        on_status(GameTraceStatus {
            running: true,
            out_path: Some(out_str.clone()),
            samples: rows,
            elapsed_s: start.elapsed().as_secs(),
            last_power_w: s.power_w,
            last_core_mhz: s.core_mhz,
            last_volt_mv: volt_mv,
            note: None,
        });

        std::thread::sleep(std::time::Duration::from_millis(cfg.interval_ms));
    }

    let tdr_after = crate::tdr_sentinel::query_latest_tdr_event();
    let tdr_detected = tdr_after.is_some() && tdr_after != tdr_before;
    let (_, canary_sequence_after) = crate::tdr_sentinel::canary_trace_marker();
    let summary = serde_json::json!({
        "summary": true,
        "trace_contract": "game-trace-v3",
        "stopped_epoch_ms": epoch_ms(),
        "elapsed_ms": start.elapsed().as_millis() as u64,
        "samples": rows,
        "max_sample_gap_ms": max_sample_gap_ms,
        "missing_nvml_samples": missing_nvml_samples,
        "max_missing_nvml_streak": max_missing_nvml_streak,
        "voltage_fresh_samples": voltage_fresh_samples,
        "voltage_read_failures": voltage_read_failures,
        "voltage_min_mv": voltage_min_mv,
        "voltage_max_mv": voltage_max_mv,
        "power_max_w": power_max_w,
        "core_min_mhz": core_min_mhz,
        "core_max_mhz": core_max_mhz,
        "tdr_event_before": tdr_before,
        "tdr_event_after": tdr_after,
        "tdr_detected": tdr_detected,
        "sentinel_canary_sequence_before": canary_sequence_before,
        "sentinel_canary_sequence_after": canary_sequence_after,
    });
    writeln!(w, "{summary}").map_err(|e| format!("write summary: {e}"))?;
    w.flush().ok();
    tracing::info!(
        "game-trace: stopped — {rows} samples over {:.1} s to {out_str}",
        start.elapsed().as_secs_f64()
    );
    Ok(rows)
}

/// `game-trace` CLI subcommand entry point.
pub fn run(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = parse_config(args, default_out())
        .map_err(|e| -> Box<dyn std::error::Error> { format!("game-trace args: {e}").into() })?;

    unsafe {
        if windows::Win32::System::Console::SetConsoleCtrlHandler(Some(ctrl_handler), true).is_err()
        {
            tracing::warn!(
                "game-trace: Ctrl+C handler not installed — use --secs to bound the run"
            );
        }
    }
    if cfg.secs == 0 {
        tracing::info!("game-trace: Ctrl+C to stop.");
    }

    run_sampling(&cfg, &CLI_STOP, |_| {})?;
    Ok(())
}

/// UI-controlled background telemetry logger, living in the running service process (already
/// elevated). Mirrors the [`crate::gpu_benchmark::BenchmarkHandle`] start/stop/status shape.
#[derive(Clone)]
pub struct GameTraceHandle {
    status: Arc<Mutex<GameTraceStatus>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Default for GameTraceHandle {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(GameTraceStatus::default())),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn set_status(status: &Arc<Mutex<GameTraceStatus>>, s: GameTraceStatus) {
    if let Ok(mut g) = status.lock() {
        *g = s;
    }
}

impl GameTraceHandle {
    pub fn status(&self) -> GameTraceStatus {
        self.status.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Start the background trace. Returns false if one is already running.
    pub fn start(&self) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let status = Arc::clone(&self.status);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            let cfg = Config::for_handle();
            set_status(
                &status,
                GameTraceStatus {
                    running: true,
                    out_path: Some(cfg.out.display().to_string()),
                    note: Some("iniciando".into()),
                    ..Default::default()
                },
            );
            let st2 = Arc::clone(&status);
            let result = run_sampling(&cfg, &stop, move |s| set_status(&st2, s));
            if let Ok(mut g) = status.lock() {
                g.running = false;
                g.note = Some(match result {
                    Ok(n) => format!("parado — {n} amostras"),
                    Err(e) => format!("erro: {e}"),
                });
            }
            running.store(false, Ordering::SeqCst);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_config_defaults_and_overrides() {
        let def = PathBuf::from("D:\\default.jsonl");
        let c = parse_config(&os(&["game-trace"]), def.clone()).unwrap();
        assert_eq!(c.out, def);
        assert_eq!(c.secs, 0);
        assert_eq!(c.interval_ms, 10);
        assert_eq!(c.volt_ms, 200);

        let c = parse_config(
            &os(&[
                "game-trace",
                "--out",
                "X.jsonl",
                "--secs",
                "60",
                "--interval-ms",
                "5",
                "--volt-ms",
                "100",
            ]),
            def,
        )
        .unwrap();
        assert_eq!(c.out, PathBuf::from("X.jsonl"));
        assert_eq!(c.secs, 60);
        assert_eq!(c.interval_ms, 5);
        assert_eq!(c.volt_ms, 100);
    }

    #[test]
    fn parse_config_interval_floored_to_one_and_bad_values_fail_closed() {
        let def = PathBuf::from("d.jsonl");
        assert_eq!(
            parse_config(&os(&["x", "--interval-ms", "0"]), def.clone())
                .unwrap()
                .interval_ms,
            1
        );
        assert!(parse_config(&os(&["x", "--secs", "abc"]), def.clone()).is_err());
        assert!(parse_config(&os(&["x", "--out"]), def).is_err());
    }

    #[test]
    fn handle_starts_idle_and_reports_not_running() {
        let h = GameTraceHandle::default();
        let s = h.status();
        assert!(!s.running);
        assert_eq!(s.samples, 0);
    }
}
