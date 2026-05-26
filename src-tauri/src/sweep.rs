use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::monitor::Monitor;
use crate::stress::{StressConfig, StressTest};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SweepParam {
    CpuCoreVoltage,
    CpuCacheVoltage,
    CpuPowerLimit,
    CpuTurboRatio,
}

impl SweepParam {
    pub fn label(&self) -> &str {
        match self {
            Self::CpuCoreVoltage => "CPU Core Voltage Offset",
            Self::CpuCacheVoltage => "CPU Cache Voltage Offset",
            Self::CpuPowerLimit => "CPU Power Limit (PL1)",
            Self::CpuTurboRatio => "CPU Turbo Ratio",
        }
    }

    pub fn default_range(&self) -> (f64, f64, f64) {
        match self {
            Self::CpuCoreVoltage | Self::CpuCacheVoltage => (-100.0, 50.0, 5.0),
            Self::CpuPowerLimit => (65.0, 150.0, 5.0),
            Self::CpuTurboRatio => (40.0, 55.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepConfig {
    pub param: SweepParam,
    pub range_start: f64,
    pub range_end: f64,
    pub step: f64,
    pub test_duration_secs: u64,
    pub cpu_threads: u32,
}

impl Default for SweepConfig {
    fn default() -> Self {
        let (start, end, step) = SweepParam::CpuCoreVoltage.default_range();
        Self {
            param: SweepParam::CpuCoreVoltage,
            range_start: start,
            range_end: end,
            step,
            test_duration_secs: 30,
            cpu_threads: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepStep {
    pub value: f64,
    pub label: String,
    pub index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub step: SweepStep,
    pub throughput: f64,
    pub cpu_utilization: f64,
    pub whea_errors: u32,
    pub stable: bool,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum SweepState {
    Idle,
    Running,
    Completed,
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepProgress {
    pub state: SweepState,
    pub current_step: usize,
    pub total_steps: usize,
    pub steps: Vec<StepResult>,
    pub best_score: f64,
    pub best_value: Option<f64>,
    pub param: Option<SweepParam>,
}

pub struct SweepEngine {
    active: Arc<AtomicBool>,
    progress: Arc<Mutex<SweepProgress>>,
    use_simulator: bool,
}

impl Default for SweepEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SweepEngine {
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
            progress: Arc::new(Mutex::new(SweepProgress {
                state: SweepState::Idle,
                current_step: 0,
                total_steps: 0,
                steps: Vec::new(),
                best_score: 0.0,
                best_value: None,
                param: None,
            })),
            use_simulator: true,
        }
    }

    pub fn start(&self, config: SweepConfig) -> Result<(), String> {
        {
            let p = self.progress.lock().map_err(|e| e.to_string())?;
            if p.state != SweepState::Idle {
                return Err("Sweep already in progress or not reset".into());
            }
        }

        self.active.store(true, Ordering::SeqCst);
        let active = Arc::clone(&self.active);
        let progress = Arc::clone(&self.progress);
        let use_sim = self.use_simulator;

        let mut values = Vec::new();
        let mut v = config.range_start;
        while v <= config.range_end {
            values.push(v);
            v += config.step;
        }
        if *values.last().unwrap() < config.range_end {
            values.push(config.range_end);
        }

        {
            let mut p = progress.lock().map_err(|e| e.to_string())?;
            p.state = SweepState::Running;
            p.total_steps = values.len();
            p.current_step = 0;
            p.steps.clear();
            p.best_score = 0.0;
            p.best_value = None;
            p.param = Some(config.param.clone());
        }

        let total = values.len();
        std::thread::spawn(move || {
            if use_sim {
                run_simulated_sweep(&config, &values, &active, &progress, total);
            } else {
                run_hardware_sweep(&config, &values, &active, &progress, total);
            }

            if let Ok(mut p) = progress.lock() {
                p.state = if active.load(Ordering::SeqCst) {
                    SweepState::Completed
                } else {
                    SweepState::Stopped
                };
            }
        });

        Ok(())
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn get_progress(&self) -> Result<SweepProgress, String> {
        self.progress
            .lock()
            .map(|p| p.clone())
            .map_err(|e| e.to_string())
    }

    pub fn reset(&self) {
        self.active.store(false, Ordering::SeqCst);
        if let Ok(mut p) = self.progress.lock() {
            *p = SweepProgress {
                state: SweepState::Idle,
                current_step: 0,
                total_steps: 0,
                steps: Vec::new(),
                best_score: 0.0,
                best_value: None,
                param: None,
            };
        }
    }

    pub fn set_simulator(&mut self, enabled: bool) {
        self.use_simulator = enabled;
    }
}

fn run_hardware_sweep(
    config: &SweepConfig,
    values: &[f64],
    active: &AtomicBool,
    progress: &Arc<Mutex<SweepProgress>>,
    _total: usize,
) {
    let stress = StressTest::new();
    let mut monitor = Monitor::new();
    let duration = config.test_duration_secs;
    let threads = config.cpu_threads;

    for (i, &value) in values.iter().enumerate() {
        if !active.load(Ordering::SeqCst) {
            break;
        }
        let metrics = stress.run(&StressConfig { duration_secs: duration, cpu_threads: threads });
        let sensors = monitor.read_sensors();
        let whea_penalty = 1.0 / (1.0 + sensors.whea.error_count as f64 * 10.0);
        let score = metrics.throughput_ops_per_sec * whea_penalty;
        let result = StepResult {
            step: SweepStep {
                value,
                label: format!("{} = {:.1}", config.param.label(), value),
                index: i,
            },
            throughput: metrics.throughput_ops_per_sec,
            cpu_utilization: sensors.cpu.utilization_pct,
            whea_errors: sensors.whea.error_count,
            stable: sensors.whea.error_count == 0,
            score,
        };
        if let Ok(mut p) = progress.lock() {
            p.current_step = i + 1;
            p.steps.push(result);
            if let Some(last) = p.steps.last() {
                if last.score > p.best_score {
                    p.best_score = last.score;
                    p.best_value = Some(value);
                }
            }
        }
    }
}

fn run_simulated_sweep(
    config: &SweepConfig,
    values: &[f64],
    active: &AtomicBool,
    progress: &Arc<Mutex<SweepProgress>>,
    _total: usize,
) {
    for (i, &value) in values.iter().enumerate() {
        if !active.load(Ordering::SeqCst) {
            break;
        }
        let result = simulate_step(&config.param, value, i);
        std::thread::sleep(std::time::Duration::from_millis(200));

        if let Ok(mut p) = progress.lock() {
            p.current_step = i + 1;
            p.steps.push(result);
            if let Some(last) = p.steps.last() {
                if last.score > p.best_score {
                    p.best_score = last.score;
                    p.best_value = Some(value);
                }
            }
        }
    }
}

fn simulate_step(param: &SweepParam, value: f64, index: usize) -> StepResult {
    let (throughput, whea, util) = match param {
        SweepParam::CpuCoreVoltage | SweepParam::CpuCacheVoltage => {
            let base = 100_000_000.0;
            let tf = if value < -60.0 {
                1.0 - (value + 60.0).abs() * 0.005
            } else if value < -30.0 {
                1.0 - (value + 30.0).abs() * 0.001
            } else if value < 0.0 {
                1.0
            } else if value < 30.0 {
                1.0 + value * 0.0015
            } else {
                1.045
            };
            let thro = base * tf;

            let whea = if value < -45.0 {
                let d = (value + 45.0).abs();
                (d * 0.08 + d * d * 0.001) as u32
            } else {
                0
            };

            let util = if value < -70.0 { 92.0 - (value + 70.0).abs() * 0.2 }
                       else if value < 0.0 { 95.0 }
                       else { 97.0 + value * 0.05 };

            (thro, whea, util.min(100.0))
        }

        SweepParam::CpuPowerLimit => {
            let base = 100_000_000.0;
            let nominal = 95.0;
            let tf = if value < nominal {
                0.4 + 0.6 * (value / nominal)
            } else {
                1.0
            };
            let thro = base * tf;
            let whea = 0;
            let util = if value < 70.0 { 85.0 } else if value < nominal { 90.0 } else { 98.0 };
            (thro, whea, util)
        }

        SweepParam::CpuTurboRatio => {
            let base = 100_000_000.0;
            let tf = if value < 44.0 {
                0.7 + 0.3 * (value - 40.0) / 4.0
            } else if value < 50.0 {
                1.0 + (value - 44.0) * 0.02
            } else {
                1.12
            };
            let thro = base * tf;
            let whea = if value > 52.0 { (value - 52.0) as u32 * 2 } else { 0 };
            let util = if value < 44.0 { 88.0 } else { 97.0 };
            (thro, whea, util)
        }
    };

    let whea_penalty = 1.0 / (1.0 + whea as f64 * 10.0);
    let score = throughput * whea_penalty;

    StepResult {
        step: SweepStep {
            value,
            label: format!("{} = {:.1}", param.label(), value),
            index,
        },
        throughput,
        cpu_utilization: util,
        whea_errors: whea,
        stable: whea == 0,
        score,
    }
}
