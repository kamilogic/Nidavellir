//! GPU undervolt/OC sweep engine (roadmap §5 + §12).
//!
//! The philosophy (per §12): find the *minimum stable voltage* per frequency by
//! walking the stability frontier from the safe side, detecting instability via
//! **compute validation** (known-answer divergence) *before* a crash — not by
//! crashing into the cliff like Afterburner. The result is a per-frequency
//! voltage map synthesized into the three named profiles (Godforge / Brokkr's
//! Best / Deep Calm), not a flat offset.
//!
//! This module is the **pure, fully-tested step machine**: it emits
//! [`SweepCommand`]s ("apply this point and test it") and consumes
//! [`StabilityResult`]s. All hardware I/O (applying via NVAPI, running the
//! Vulkan stressor, reading WHEA) lives behind the [`crate::gpu_control`] trait
//! in the service, so the engine itself is deterministic and testable.

use serde::{Deserialize, Serialize};

/// A point on the voltage/frequency curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VfPoint {
    pub freq_mhz: u32,
    pub voltage_mv: u32,
}

/// Outcome of evaluating a candidate point (roadmap §12: silent errors matter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilityResult {
    /// Compute validation matched the known answer and no WHEA/crash occurred.
    Stable,
    /// Driver/output produced wrong results without crashing — the real gap.
    SilentError,
    /// Hard fault (driver TDR / crash / WHEA). Caught by the Safe Loop on reboot.
    Crash,
}

impl StabilityResult {
    pub fn is_stable(self) -> bool {
        matches!(self, StabilityResult::Stable)
    }
}

/// Compute validation: compare a kernel's checksum against the known-good value.
/// Any divergence is a *silent error* — instability detected before the crash.
pub fn validate_compute(expected: u64, actual: u64) -> StabilityResult {
    if expected == actual {
        StabilityResult::Stable
    } else {
        StabilityResult::SilentError
    }
}

/// Apply the safety margin to a measured stability cliff (roadmap §12, Phase 3):
/// a fixed buffer plus a temperature coefficient, since the frontier moves with
/// temperature (hot silicon needs more voltage).
///
/// `cliff_mv` is the lowest voltage proven stable; `headroom_c` is how much
/// hotter the worst case may run than the (thermally-stabilized) test.
pub fn apply_safety_margin(
    cliff_mv: u32,
    fixed_margin_mv: u32,
    temp_coeff_mv_per_c: u32,
    headroom_c: u32,
) -> u32 {
    cliff_mv + fixed_margin_mv + temp_coeff_mv_per_c * headroom_c
}

/// Bisection search for the minimum stable voltage at a fixed frequency
/// (roadmap §12, Phase 3). Descends from a known-stable voltage; once an
/// unstable point is found it bisects the gap until it is within `min_step_mv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoltageBisection {
    pub freq_mhz: u32,
    /// Lowest voltage proven stable so far (starts at the stock voltage).
    pub stable_mv: u32,
    /// Highest voltage proven unstable so far, if any.
    pub unstable_mv: Option<u32>,
    /// Don't probe below this hard floor.
    pub floor_mv: u32,
    /// Coarse descent step before a cliff is bracketed.
    pub descent_step_mv: u32,
    /// Convergence threshold for the bisection gap.
    pub min_step_mv: u32,
    /// The point currently under test (None until `next_test` is called).
    pub current_mv: Option<u32>,
}

impl VoltageBisection {
    pub fn new(
        freq_mhz: u32,
        stock_mv: u32,
        floor_mv: u32,
        descent_step_mv: u32,
        min_step_mv: u32,
    ) -> Self {
        Self {
            freq_mhz,
            stable_mv: stock_mv,
            unstable_mv: None,
            floor_mv,
            descent_step_mv: descent_step_mv.max(1),
            min_step_mv: min_step_mv.max(1),
            current_mv: None,
        }
    }

    /// The next voltage to test, or `None` when converged.
    pub fn next_test(&mut self) -> Option<u32> {
        let candidate = match self.unstable_mv {
            // No cliff yet: descend coarsely from the lowest known-stable point.
            None => {
                if self.stable_mv <= self.floor_mv {
                    None // floor itself is stable — nothing lower to try
                } else {
                    Some(self.stable_mv.saturating_sub(self.descent_step_mv).max(self.floor_mv))
                }
            }
            // Cliff bracketed: bisect the [unstable, stable] gap.
            Some(unstable) => {
                if self.stable_mv.saturating_sub(unstable) <= self.min_step_mv {
                    None // converged
                } else {
                    Some(midpoint(unstable, self.stable_mv))
                }
            }
        };
        self.current_mv = candidate;
        candidate
    }

    /// Feed back the result of testing [`Self::current_mv`].
    pub fn record(&mut self, result: StabilityResult) {
        let Some(tested) = self.current_mv else {
            return;
        };
        if result.is_stable() {
            self.stable_mv = self.stable_mv.min(tested);
        } else {
            self.unstable_mv = Some(self.unstable_mv.map_or(tested, |u| u.max(tested)));
        }
        self.current_mv = None;
    }

    /// Whether the search has converged on a minimum stable voltage.
    pub fn is_converged(&mut self) -> bool {
        // Peek without disturbing state: clone the decision logic.
        match self.unstable_mv {
            None => self.stable_mv <= self.floor_mv,
            Some(unstable) => self.stable_mv.saturating_sub(unstable) <= self.min_step_mv,
        }
    }

    /// The minimum stable voltage found (the cliff, before safety margin).
    pub fn min_stable_mv(&self) -> u32 {
        self.stable_mv
    }
}

fn midpoint(a: u32, b: u32) -> u32 {
    a + (b - a) / 2
}

/// One synthesized, named profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuProfile {
    pub name: String,
    pub description: String,
    pub point: VfPoint,
}

/// The three named outputs (roadmap §12, Phase 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuProfileSet {
    pub godforge: GpuProfile,
    pub brokkrs_best: GpuProfile,
    pub deep_calm: GpuProfile,
}

/// A validated (frequency, minimum-stable-voltage) datum from the sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeoffPoint {
    pub freq_mhz: u32,
    pub vmin_mv: u32,
}

/// Synthesize the three profiles from the validated tradeoff map (§12 Phase 5).
///
/// - **Godforge**: highest sustained frequency at its minimum voltage.
/// - **Brokkr's Best**: the perf/watt knee — best MHz-per-mV ratio.
/// - **Deep Calm**: lowest voltage among points that still hold ≥95% of the
///   stock sustained baseline frequency.
pub fn synthesize_profiles(baseline_freq_mhz: u32, points: &[TradeoffPoint]) -> Option<GpuProfileSet> {
    if points.is_empty() {
        return None;
    }

    let godforge = points
        .iter()
        .max_by_key(|p| (p.freq_mhz, std::cmp::Reverse(p.vmin_mv)))
        .copied()?;

    // perf/watt knee: maximize freq per mV (×1000 to keep integer precision).
    let brokkrs = points
        .iter()
        .max_by_key(|p| (p.freq_mhz as u64 * 1000) / p.vmin_mv.max(1) as u64)
        .copied()?;

    // Deep Calm: among points holding ≥95% baseline, the lowest voltage.
    let threshold = (baseline_freq_mhz as u64 * 95 / 100) as u32;
    let deep_calm = points
        .iter()
        .filter(|p| p.freq_mhz >= threshold)
        .min_by_key(|p| (p.vmin_mv, std::cmp::Reverse(p.freq_mhz)))
        .copied()
        .unwrap_or(godforge);

    Some(GpuProfileSet {
        godforge: GpuProfile {
            name: "Godforge".into(),
            description: "Highest sustained clock at the lowest stable voltage.".into(),
            point: VfPoint { freq_mhz: godforge.freq_mhz, voltage_mv: godforge.vmin_mv },
        },
        brokkrs_best: GpuProfile {
            name: "Brokkr's Best".into(),
            description: "Perf/watt knee — every MHz still worth the watt.".into(),
            point: VfPoint { freq_mhz: brokkrs.freq_mhz, voltage_mv: brokkrs.vmin_mv },
        },
        deep_calm: GpuProfile {
            name: "Deep Calm".into(),
            description: "Lowest watts while holding >=95% of base clock — cool and quiet.".into(),
            point: VfPoint { freq_mhz: deep_calm.freq_mhz, voltage_mv: deep_calm.vmin_mv },
        },
    })
}

/// Reported phase of the overall pipeline (roadmap §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepPhase {
    Idle,
    Baseline,
    VramDiagnostic,
    VoltageBisection,
    Synthesis,
    Done,
    Aborted,
}

/// Configuration for a sweep run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuSweepConfig {
    /// Target frequencies to characterize (descending sustained clocks).
    pub target_freqs_mhz: Vec<u32>,
    /// Stock voltage at the ceiling, the known-stable starting point.
    pub stock_mv: u32,
    /// Hard voltage floor — never probe below this.
    pub floor_mv: u32,
    pub descent_step_mv: u32,
    pub min_step_mv: u32,
    pub fixed_margin_mv: u32,
    pub temp_coeff_mv_per_c: u32,
    pub temp_headroom_c: u32,
}

impl Default for GpuSweepConfig {
    fn default() -> Self {
        Self {
            target_freqs_mhz: vec![1950, 1875, 1800, 1695],
            stock_mv: 1050,
            floor_mv: 700,
            descent_step_mv: 25,
            min_step_mv: 5,
            fixed_margin_mv: 15,
            temp_coeff_mv_per_c: 1,
            temp_headroom_c: 15,
        }
    }
}

/// What the engine wants the runtime to do next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SweepCommand {
    /// Apply this point (arm Safe Loop, set V/F), dwell, run validation, report.
    ApplyAndTest { point: VfPoint },
    /// Sweep is finished; profiles are available.
    Finished,
}

/// Live, serializable progress snapshot for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuSweepProgress {
    pub phase: SweepPhase,
    pub current: Option<VfPoint>,
    pub freq_index: usize,
    pub total_freqs: usize,
    pub tradeoffs: Vec<TradeoffPoint>,
    pub profiles: Option<GpuProfileSet>,
    /// True when running against the simulated tuner (no real hardware writes).
    pub simulated: bool,
}

/// The pure sweep step machine: characterizes vmin per target frequency, then
/// synthesizes the profiles. Drive it with [`Self::next_command`] /
/// [`Self::record`]; it performs no I/O.
#[derive(Debug, Clone)]
pub struct GpuSweepEngine {
    config: GpuSweepConfig,
    baseline_freq_mhz: u32,
    phase: SweepPhase,
    freq_index: usize,
    bisection: Option<VoltageBisection>,
    tradeoffs: Vec<TradeoffPoint>,
    profiles: Option<GpuProfileSet>,
    simulated: bool,
}

impl GpuSweepEngine {
    pub fn new(config: GpuSweepConfig, baseline_freq_mhz: u32, simulated: bool) -> Self {
        Self {
            config,
            baseline_freq_mhz,
            phase: SweepPhase::VoltageBisection,
            freq_index: 0,
            bisection: None,
            tradeoffs: Vec::new(),
            profiles: None,
            simulated,
        }
    }

    fn start_freq(&mut self) {
        if let Some(&freq) = self.config.target_freqs_mhz.get(self.freq_index) {
            self.bisection = Some(VoltageBisection::new(
                freq,
                self.config.stock_mv,
                self.config.floor_mv,
                self.config.descent_step_mv,
                self.config.min_step_mv,
            ));
        }
    }

    /// Emit the next action, advancing internal phase as needed.
    pub fn next_command(&mut self) -> SweepCommand {
        if matches!(self.phase, SweepPhase::Done | SweepPhase::Aborted) {
            return SweepCommand::Finished;
        }
        if self.bisection.is_none() {
            if self.freq_index >= self.config.target_freqs_mhz.len() {
                return self.finish();
            }
            self.start_freq();
        }

        let bis = self.bisection.as_mut().expect("bisection set above");
        if let Some(mv) = bis.next_test() {
            SweepCommand::ApplyAndTest {
                point: VfPoint { freq_mhz: bis.freq_mhz, voltage_mv: mv },
            }
        } else {
            // Converged for this frequency: record the cliff + margin, advance.
            let cliff = bis.min_stable_mv();
            let safe = apply_safety_margin(
                cliff,
                self.config.fixed_margin_mv,
                self.config.temp_coeff_mv_per_c,
                self.config.temp_headroom_c,
            );
            self.tradeoffs.push(TradeoffPoint {
                freq_mhz: bis.freq_mhz,
                vmin_mv: safe,
            });
            self.bisection = None;
            self.freq_index += 1;
            self.next_command()
        }
    }

    fn finish(&mut self) -> SweepCommand {
        self.phase = SweepPhase::Synthesis;
        self.profiles = synthesize_profiles(self.baseline_freq_mhz, &self.tradeoffs);
        self.phase = SweepPhase::Done;
        SweepCommand::Finished
    }

    /// Feed back the stability result for the last [`SweepCommand::ApplyAndTest`].
    pub fn record(&mut self, result: StabilityResult) {
        if let Some(bis) = self.bisection.as_mut() {
            bis.record(result);
        }
    }

    /// Abort the sweep (e.g. on operator stop or a hard crash recovery).
    pub fn abort(&mut self) {
        self.phase = SweepPhase::Aborted;
        self.bisection = None;
    }

    pub fn progress(&self) -> GpuSweepProgress {
        GpuSweepProgress {
            phase: self.phase,
            current: self.bisection.as_ref().and_then(|b| {
                b.current_mv.map(|mv| VfPoint { freq_mhz: b.freq_mhz, voltage_mv: mv })
            }),
            freq_index: self.freq_index,
            total_freqs: self.config.target_freqs_mhz.len(),
            tradeoffs: self.tradeoffs.clone(),
            profiles: self.profiles.clone(),
            simulated: self.simulated,
        }
    }

    pub fn profiles(&self) -> Option<&GpuProfileSet> {
        self.profiles.as_ref()
    }

    pub fn phase(&self) -> SweepPhase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_validation_detects_divergence() {
        assert_eq!(validate_compute(42, 42), StabilityResult::Stable);
        assert_eq!(validate_compute(42, 7), StabilityResult::SilentError);
    }

    #[test]
    fn safety_margin_adds_fixed_and_thermal() {
        // cliff 900 + 15 fixed + 1mV/°C * 15°C = 930
        assert_eq!(apply_safety_margin(900, 15, 1, 15), 930);
    }

    #[test]
    fn bisection_converges_on_cliff() {
        // True minimum-stable voltage is 905 mV.
        let true_vmin = 905;
        let mut bis = VoltageBisection::new(1900, 1050, 700, 25, 5);
        let mut guard = 0;
        while let Some(mv) = bis.next_test() {
            let result = if mv >= true_vmin {
                StabilityResult::Stable
            } else {
                StabilityResult::SilentError
            };
            bis.record(result);
            guard += 1;
            assert!(guard < 100, "bisection failed to converge");
        }
        // Converged within min_step of the true cliff, and never below it.
        assert!(bis.min_stable_mv() >= true_vmin);
        assert!(bis.min_stable_mv() - true_vmin <= bis.min_step_mv + bis.descent_step_mv);
    }

    #[test]
    fn bisection_floor_stable_converges_without_unstable() {
        let mut bis = VoltageBisection::new(1800, 760, 750, 25, 5);
        // Everything down to the floor is stable.
        let mut guard = 0;
        while let Some(_mv) = bis.next_test() {
            bis.record(StabilityResult::Stable);
            guard += 1;
            assert!(guard < 50);
        }
        assert_eq!(bis.min_stable_mv(), 750);
    }

    #[test]
    fn synthesize_picks_distinct_profiles() {
        let points = vec![
            TradeoffPoint { freq_mhz: 1950, vmin_mv: 1000 },
            TradeoffPoint { freq_mhz: 1875, vmin_mv: 900 },
            TradeoffPoint { freq_mhz: 1800, vmin_mv: 820 },
            TradeoffPoint { freq_mhz: 1695, vmin_mv: 760 },
        ];
        let set = synthesize_profiles(1900, &points).unwrap();
        // Godforge = highest freq.
        assert_eq!(set.godforge.point.freq_mhz, 1950);
        // Deep Calm holds >=95% of 1900 (=1805) → only 1950/1875 qualify; lowest V = 1875@900.
        assert_eq!(set.deep_calm.point.freq_mhz, 1875);
        // Brokkr's = best MHz/mV → 1695/760 ≈ 2.23 beats 1950/1000 = 1.95.
        assert_eq!(set.brokkrs_best.point.freq_mhz, 1695);
    }

    #[test]
    fn synthesize_empty_is_none() {
        assert!(synthesize_profiles(1900, &[]).is_none());
    }

    #[test]
    fn engine_runs_full_sweep_against_a_frontier() {
        // Frontier: vmin rises with frequency (higher clock needs more volts).
        let frontier = |freq: u32| -> u32 {
            match freq {
                f if f >= 1950 => 980,
                f if f >= 1875 => 910,
                f if f >= 1800 => 850,
                _ => 790,
            }
        };
        let cfg = GpuSweepConfig::default();
        let mut engine = GpuSweepEngine::new(cfg, 1900, true);

        let mut guard = 0;
        loop {
            match engine.next_command() {
                SweepCommand::ApplyAndTest { point } => {
                    let result = if point.voltage_mv >= frontier(point.freq_mhz) {
                        StabilityResult::Stable
                    } else {
                        StabilityResult::SilentError
                    };
                    engine.record(result);
                }
                SweepCommand::Finished => break,
            }
            guard += 1;
            assert!(guard < 1000, "engine did not finish");
        }

        assert_eq!(engine.phase(), SweepPhase::Done);
        let progress = engine.progress();
        assert_eq!(progress.tradeoffs.len(), 4);
        // Each recorded vmin must be at/above the true frontier + margin headroom.
        for t in &progress.tradeoffs {
            assert!(t.vmin_mv >= frontier(t.freq_mhz));
        }
        assert!(engine.profiles().is_some());
        // Higher frequencies should require at least as much voltage.
        let v1950 = progress.tradeoffs.iter().find(|t| t.freq_mhz == 1950).unwrap().vmin_mv;
        let v1695 = progress.tradeoffs.iter().find(|t| t.freq_mhz == 1695).unwrap().vmin_mv;
        assert!(v1950 > v1695);
    }

    #[test]
    fn aborted_engine_finishes_immediately() {
        let mut engine = GpuSweepEngine::new(GpuSweepConfig::default(), 1900, true);
        engine.abort();
        assert_eq!(engine.next_command(), SweepCommand::Finished);
        assert_eq!(engine.phase(), SweepPhase::Aborted);
    }
}
