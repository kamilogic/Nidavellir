use crate::sweep::{SweepParam, StepResult};
use crate::tuner::TuningParams;
use serde::{Deserialize, Serialize};

const PROFILES_PATH: &str = "C:\\ProgramData\\Nidavellir\\profiles.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProfileKind {
    Godforge,
    BrokkrsBest,
    DeepCalm,
}

impl ProfileKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Godforge => "Godforge — Maximum Performance",
            Self::BrokkrsBest => "Brokkr's Best — Efficient Performance",
            Self::DeepCalm => "Deep Calm — Low Power",
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Godforge => "Unlocks every drop of performance. Expect higher power draw and temperatures.",
            Self::BrokkrsBest => "Best performance-per-watt. Ideal for daily driving.",
            Self::DeepCalm => "Minimal power consumption. Quiet, cool, and stable.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub kind: ProfileKind,
    pub name: String,
    pub tuning: TuningParams,
    pub expected_performance_pct: f64,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSet {
    pub profiles: Vec<Profile>,
    pub source_param: String,
    pub generated_at: String,
}

pub fn generate_profiles(steps: &[StepResult], param: &SweepParam) -> ProfileSet {
    let baseline = find_baseline_throughput(steps, param);
    let stable: Vec<&StepResult> = steps.iter().filter(|s| s.stable).collect();
    let sorted: Vec<&StepResult> = if stable.is_empty() {
        let mut s: Vec<&StepResult> = steps.iter().collect();
        s.sort_by_key(|s| s.whea_errors);
        s
    } else {
        let mut s = stable;
        s.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        s
    };

    let godforge = select_godforge(&sorted, param, baseline);
    let brokkr = select_brokkr(&sorted, param, baseline);
    let deep_calm = select_deep_calm(&sorted, param, baseline);

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    ProfileSet {
        profiles: vec![godforge, brokkr, deep_calm],
        source_param: param.label().to_string(),
        generated_at: now,
    }
}

/// Pick the throughput at the parameter's "stock" value (closest to nominal/0)
/// as a baseline for percentage calculations.
fn find_baseline_throughput(steps: &[StepResult], param: &SweepParam) -> f64 {
    if steps.is_empty() {
        return 1.0;
    }
    let nominal = nominal_value(param);
    let closest = steps
        .iter()
        .min_by(|a, b| {
            (a.step.value - nominal)
                .abs()
                .partial_cmp(&(b.step.value - nominal).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|s| s.throughput)
        .unwrap_or(1.0);
    if closest > 0.0 {
        closest
    } else {
        1.0
    }
}

/// Stock/nominal value for each tunable, used as the baseline anchor.
fn nominal_value(param: &SweepParam) -> f64 {
    match param {
        SweepParam::CpuCoreVoltage | SweepParam::CpuCacheVoltage => 0.0,
        SweepParam::CpuPowerLimit => 95.0,
        SweepParam::CpuTurboRatio => 50.0,
    }
}

fn perf_pct(throughput: f64, baseline: f64) -> f64 {
    ((throughput / baseline) * 100.0).round()
}

fn select_godforge(sorted: &[&StepResult], param: &SweepParam, baseline: f64) -> Profile {
    let best = sorted.first().map(|s| s.step.value).unwrap_or(0.0);
    let perf = sorted.first().map(|s| s.throughput).unwrap_or(baseline);

    Profile {
        kind: ProfileKind::Godforge,
        name: ProfileKind::Godforge.label().to_string(),
        tuning: tuning_from_value(param, best),
        expected_performance_pct: perf_pct(perf, baseline),
        notes: format!(
            "Peak performance at {} = {:.1}. Best raw throughput.",
            param.label(),
            best
        ),
    }
}

fn select_brokkr(sorted: &[&StepResult], param: &SweepParam, baseline: f64) -> Profile {
    let best = sorted
        .iter()
        .copied()
        .filter(|s| s.stable && s.whea_errors == 0)
        .min_by(|a, b| {
            let eff_a = a.score / (a.step.value.abs().max(1.0));
            let eff_b = b.score / (b.step.value.abs().max(1.0));
            eff_b.partial_cmp(&eff_a).unwrap_or(std::cmp::Ordering::Equal)
        });

    match best {
        Some(s) => Profile {
            kind: ProfileKind::BrokkrsBest,
            name: ProfileKind::BrokkrsBest.label().to_string(),
            tuning: tuning_from_value(param, s.step.value),
            expected_performance_pct: perf_pct(s.throughput, baseline),
            notes: format!(
                "Best efficiency at {} = {:.1}. Score/value ratio is optimal.",
                param.label(),
                s.step.value
            ),
        },
        None => select_godforge(sorted, param, baseline),
    }
}

fn select_deep_calm(sorted: &[&StepResult], param: &SweepParam, baseline: f64) -> Profile {
    let best = sorted
        .iter()
        .copied()
        .filter(|s| s.stable && s.whea_errors == 0)
        .min_by(|a, b| {
            if a.step.value != b.step.value {
                a.step.value.partial_cmp(&b.step.value).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
            }
        });

    match best {
        Some(s) => Profile {
            kind: ProfileKind::DeepCalm,
            name: ProfileKind::DeepCalm.label().to_string(),
            tuning: tuning_from_value(param, s.step.value),
            expected_performance_pct: perf_pct(s.throughput, baseline),
            notes: format!(
                "Lowest power at {} = {:.1}. Minimal voltage while stable.",
                param.label(),
                s.step.value
            ),
        },
        None => select_godforge(sorted, param, baseline),
    }
}

fn tuning_from_value(param: &SweepParam, value: f64) -> TuningParams {
    let mut t = TuningParams::default();
    match param {
        SweepParam::CpuCoreVoltage => t.cpu_voltage_offset_mv = value as i32,
        SweepParam::CpuCacheVoltage => t.cpu_cache_offset_mv = value as i32,
        SweepParam::CpuPowerLimit => {
            t.pl1_watts = value as u32;
            t.pl2_watts = (value as u32).saturating_mul(125).saturating_div(100).max(value as u32);
        }
        SweepParam::CpuTurboRatio => t.turbo_ratio_limit = value as u32,
    }
    t
}

pub fn save_profile_set(set: &ProfileSet) -> Result<(), String> {
    let json = serde_json::to_string_pretty(set).map_err(|e| e.to_string())?;
    let path = std::path::Path::new(PROFILES_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_profile_set() -> Result<ProfileSet, String> {
    let path = std::path::Path::new(PROFILES_PATH);
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}
