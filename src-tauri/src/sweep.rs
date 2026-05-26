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
}

pub struct SweepEngine {
    active: Arc<AtomicBool>,
    progress: Arc<Mutex<SweepProgress>>,
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
            })),
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
        }

        std::thread::spawn(move || {
            let stress = StressTest::new();
            let monitor = Monitor::new();
            let duration = config.test_duration_secs;
            let threads = config.cpu_threads;

            for (i, &value) in values.iter().enumerate() {
                if !active.load(Ordering::SeqCst) {
                    break;
                }

                apply_stub(&config.param, value);

                let metrics = stress.run(&StressConfig {
                    duration_secs: duration,
                    cpu_threads: threads,
                });

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
            };
        }
    }
}

fn apply_stub(param: &SweepParam, value: f64) {
    println!("[SWEEP] Would set {} to {:.1}", param.label(), value);
}
