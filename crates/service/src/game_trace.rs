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
    /// NVAPI core voltage (mV), sampled at `--volt-ms`; carried forward between fresh reads.
    volt_mv: Option<u32>,
    /// True only on the sample where voltage was freshly read (else the value is carried).
    volt_fresh: bool,
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

    let meta = serde_json::json!({
        "meta": true,
        "gpu": gpu,
        "power_limit_w": power_limit_w,
        "interval_ms": cfg.interval_ms,
        "volt_ms": cfg.volt_ms,
        "started_epoch_ms": epoch_ms(),
        "note": "read-only NVML+NVAPI game workload trace",
    });
    writeln!(w, "{meta}").map_err(|e| format!("write meta: {e}"))?;

    let out_str = cfg.out.display().to_string();
    tracing::info!(
        "game-trace: {gpu} — logging every {} ms (voltage every {} ms) to {out_str}",
        cfg.interval_ms,
        cfg.volt_ms
    );

    let mut volt_mv: Option<u32> = nidavellir_gpu_nvapi::read_core_voltage_mv();
    let mut last_volt = std::time::Instant::now();
    let mut rows: u64 = 0;
    let start = std::time::Instant::now();

    while !stop.load(Ordering::SeqCst) {
        if cfg.secs > 0 && start.elapsed().as_secs() >= cfg.secs {
            break;
        }
        let s: NvmlSample = sampler.sample();

        let volt_fresh = last_volt.elapsed().as_millis() as u64 >= cfg.volt_ms;
        if volt_fresh {
            last_volt = std::time::Instant::now();
            volt_mv = nidavellir_gpu_nvapi::read_core_voltage_mv().or(volt_mv);
        }

        writeln!(
            w,
            "{}",
            serde_json::to_string(&Row {
                s,
                volt_mv,
                volt_fresh
            })
            .map_err(|e| e.to_string())?
        )
        .map_err(|e| format!("write row: {e}"))?;
        rows += 1;
        if rows.is_multiple_of(200) {
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
