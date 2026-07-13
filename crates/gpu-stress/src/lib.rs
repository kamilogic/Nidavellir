//! Real GPU compute-validation battery (wgpu / Vulkan).
//!
//! Detects instability by **wrong results** (silent errors), not just crashes
//! (roadmap §12). The battery covers distinct failure modes so it doesn't need
//! a specific game to surface them:
//!   - **ALU** — known-answer LCG, heavy integer mul/add → core stability.
//!   - **Memory** — many pseudo-random gathers from a table → memory subsystem.
//!   - **Burst** — the ALU load in on/off bursts → dI/dt power transients
//!     (what bursty game frametimes hit and a steady stress misses).
//!
//! Every stage has a bit-exact CPU reference; any divergence ⇒ `SilentError`,
//! a device-lost ⇒ `Crash`.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use nidavellir_core::gpu_sweep::StabilityResult;
use wgpu::util::DeviceExt;

const C1: u32 = 1664525;
const C2: u32 = 1013904223;
const HASH1: u32 = 2654435761;
const TABLE_INIT: u32 = 2246822519;
// FSGL3 hardware-calibration knobs: shorten the gap or increase TextureRop/MixedGame
// weights if known unstable bins survive; use a verify ring if per-frame work causes TDR pressure.
const DROOP_BURST: u64 = 6;
const DROOP_GAP_MS: u64 = 4;
// FrameCadence idle gaps between single heavy frames, cycled per frame. Sweeping the gap
// varies the load-release→re-load period so the droop transient crosses different VRM
// response frequencies instead of settling into one rhythm.
const FRAME_CADENCE_GAPS_MS: [u64; 4] = [2, 4, 6, 8];
// TextureStream banding: each frame renders in this many scissor bands, ONE SUBMIT EACH, so the
// driver can preempt between bands (desktop/audio stay responsive during the heaviest detector)
// and a stalling band is caught long before the ~2 s driver TDR watchdog.
const STREAM_BANDS: u32 = 16;
// A single band exceeding this wall time is a pre-hang: stop submitting and fail the dwell as
// Unstable instead of letting the driver watchdog reset the device.
const STREAM_PREHANG_BAND_MS: u64 = 500;
// Sustained frame time beyond reference × this factor (stock, captured with the golden) marks a
// bin as marginal: silicon slows down (internal retries) before it hangs.
const STREAM_DEGRADATION_FACTOR: u64 = 2;
// Same marginal-silicon principle for BoostEdge. The stock reference is captured with a per-frame
// CPU checksum readback, while the dwell only reads back ~4×/s — so healthy dwell frames run
// FASTER than the reference and the gate is inherently permissive; only genuine internal-retry
// slowdown crosses reference × this factor.
const BOOST_EDGE_DEGRADATION_FACTOR: u64 = 2;
const GOLDEN_MIN_FRAMES: u64 = 4;
// v16.3 BoostEdge lobby cadence: drain the pipeline EVERY frame (a CPU/engine-bound game loop
// finishes the frame, idles while the CPU builds the next one, then ramps again) and insert a
// varied sub-ms CPU-frame-build bubble before the next submit. Each frame becomes a discrete
// drain→idle→ramp current edge AT the anchor bin — the field-proven high-FPS killer regime that a
// saturated submission queue can never produce. Spin-waited (Windows sleep is ms-coarse). Values
// model CPU frame-build variance at hundreds of fps; not tied to any GPU.
const BOOST_EDGE_BUBBLE_US: &[u64] = &[0, 200, 500, 900, 300, 800];
// v15 boost-entry shock: TRUE idle gaps long enough for the driver to leave the high P-state
// (downclock hysteresis is seconds; 10-30 s is comfortably beyond), then an instant heavy slam.
// This is the game/benchmark-launch transition — idle P-state → full boost VF ramp + VRM load
// step — that continuous workloads never enter, and where the in-game TDR cascade (repeated ~2 s
// BusReset hangs, Event ID 153) was observed. The slam's wall time is the precursor detector: a
// stock heavy frame takes ~10-20 ms, so a post-idle burst beyond this threshold is a pre-hang —
// fail Unstable long BEFORE the ~2 s driver watchdog can fire (never reproduce the cascade itself).
const BOOST_ENTRY_GAPS_MS: [u64; 3] = [10_000, 20_000, 30_000];
const BOOST_ENTRY_STALL_MS: u64 = 500;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: u32,
    n: u32,
    _pad: [u32; 2],
}

/// Generic 4×u32 uniform block (16-byte aligned) reused by several kernels.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

/// Result of one battery stage.
#[derive(Debug, Clone)]
pub struct StageReport {
    pub name: String,
    pub result: StabilityResult,
    pub mismatches: u32,
    pub elapsed_ms: u64,
}

/// Result of a render-pipeline run: the stability verdict plus the rendered
/// frame count and FPS (the benchmark's performance metric).
#[derive(Debug, Clone)]
pub struct RenderResult {
    pub result: StabilityResult,
    pub frames: u64,
    pub fps: f64,
    /// Present only when the transient VF qualifier failed inside a named phase.
    pub failure_phase: Option<VfQualifierPhase>,
    pub phase_reports: Vec<VfPhaseReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderGoldens {
    pub power: u32,
    pub boost: u32,
    pub texrop: u32,
    /// Golden for the FrameCadence workload (1-instance PowerRender frame — a different image
    /// than the 8-instance `power` golden, so it needs its own stock checksum).
    pub cadence: u32,
    /// Golden for the GeometryDepth workload (instanced procedural triangles under depth test).
    pub geometry: u32,
    /// Golden for the TextureStream workload (scattered sampling of the large VRAM source).
    pub stream: u32,
    /// Stock average TextureStream frame time (ms), captured with the golden. The qualifier
    /// rejects a bin whose sustained frame time degrades far beyond this reference — marginal
    /// silicon slows down (internal retries) before it hangs.
    pub stream_frame_reference_ms: u32,
    /// Stock average BoostEdge frame time (µs, drain-per-frame lobby cadence), captured with the
    /// golden. Same marginal-silicon principle at the light-frame regime. The capture loop reads
    /// the checksum back on the CPU every frame (slightly slower than the dwell's drain-only
    /// frames), so the degradation gate errs permissive, never strict.
    pub boost_frame_reference_us: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfPhaseReport {
    pub phase: VfQualifierPhase,
    pub result: StabilityResult,
    pub frames: u64,
    pub checksum_count: u32,
    pub elapsed_ms: u64,
}

/// Deterministic failure-seeking phases used only by F2 qualification. Discovery keeps using the
/// unchanged steady power render so its clock/power statistics remain comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfQualifierPhase {
    PowerOpening,
    BoostEdge,
    HeavySpike,
    TextureRop,
    ComputeBurst,
    IdlePulse,
    MixedGame,
    PowerClosing,
    /// Game-frame-scale heavy burst / short idle gap cycling — targets VRM droop-release
    /// transients at real frame cadence (~5-25 ms period) that second-scale segments miss.
    FrameCadence,
    /// Multi-gigabyte VRAM-resident gather load — memory controller + DRAM on the shared rail,
    /// the streaming path games exercise that cache-resident render targets never touch.
    VramPressure,
    /// Instanced procedural triangles under a depth test — vertex fetch, raster and depth-ROP
    /// units the fullscreen-triangle workloads never exercise.
    GeometryDepth,
    /// Hang-prone heavy memory detector: per-pixel scattered sampling of the large VRAM source,
    /// rendered in preemptible scissor bands. Runs LAST in patterns (severity ladder).
    TextureStream,
    /// v15 boost-entry shock: true-idle (seconds, GPU leaves the high P-state) → instant heavy
    /// slam, cycling. Targets the launch-transition failure the continuous phases never enter.
    BoostEntry,
    /// v16.1 composite game load: heavy render + near-full VRAM-resident gather in the same submit.
    CompositeGameLoad,
}

impl VfQualifierPhase {
    /// Number of phase variants (codes are `0..COUNT`). Coverage bitmaps must use this size.
    pub const COUNT: usize = 14;

    pub const NONE_CODE: u8 = u8::MAX;

    pub fn label(self) -> &'static str {
        match self {
            VfQualifierPhase::PowerOpening => "power-opening",
            VfQualifierPhase::BoostEdge => "boost-edge",
            VfQualifierPhase::HeavySpike => "heavy-spike",
            VfQualifierPhase::TextureRop => "texture-rop",
            VfQualifierPhase::ComputeBurst => "compute-burst",
            VfQualifierPhase::IdlePulse => "idle-pulse",
            VfQualifierPhase::MixedGame => "mixed-game",
            VfQualifierPhase::PowerClosing => "power-closing",
            VfQualifierPhase::FrameCadence => "frame-cadence",
            VfQualifierPhase::VramPressure => "vram-pressure",
            VfQualifierPhase::GeometryDepth => "geometry-depth",
            VfQualifierPhase::TextureStream => "texture-stream",
            VfQualifierPhase::BoostEntry => "boost-entry",
            VfQualifierPhase::CompositeGameLoad => "composite-game-load",
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::PowerOpening),
            1 => Some(Self::BoostEdge),
            2 => Some(Self::HeavySpike),
            3 => Some(Self::TextureRop),
            4 => Some(Self::ComputeBurst),
            5 => Some(Self::IdlePulse),
            6 => Some(Self::MixedGame),
            7 => Some(Self::PowerClosing),
            8 => Some(Self::FrameCadence),
            9 => Some(Self::VramPressure),
            10 => Some(Self::GeometryDepth),
            11 => Some(Self::TextureStream),
            12 => Some(Self::BoostEntry),
            13 => Some(Self::CompositeGameLoad),
            _ => None,
        }
    }
}

/// Qualification workload sequence. FSGL1/2 remain available as legacy patterns; FSGL3 A/B are the
/// current default qualification patterns and intentionally differ from each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfQualifierPattern {
    Fsgl1,
    Fsgl2A,
    Fsgl2B,
    Fsgl3A,
    Fsgl3B,
    V8HighFps,
    V8Texture,
    V8Transitions,
    V8Memory,
    /// v14 candidate-only endurance soak: one CONTINUOUS mixed dwell (MixedGame + FrameCadence +
    /// the graceful TextureRop silent-error detector) that scales to fill a single ~15-min dwell.
    /// Run ONLY at the exact Apply pair, after the required 3-pattern set.
    Endurance,
    /// v15 candidate-only transition shock: true-idle → heavy-slam cycles (BoostEntry) with the
    /// graceful TextureRop detector between rounds. Targets the game/benchmark-LAUNCH transition
    /// (P-state exit + VF ramp + VRM load step) behind the observed in-game BusReset TDR cascade.
    /// Run ONLY at the exact Apply pair, before the Endurance soak (fail cheap first).
    TransitionShock,
}

impl VfQualifierPattern {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fsgl1 => "fsgl1",
            Self::Fsgl2A => "fsgl2-a",
            Self::Fsgl2B => "fsgl2-b",
            Self::Fsgl3A => "fsgl3-a",
            Self::Fsgl3B => "fsgl3-b",
            Self::V8HighFps => "v8-high-fps",
            Self::V8Texture => "v8-texture",
            Self::V8Transitions => "v8-transitions",
            Self::V8Memory => "v8-memory",
            Self::Endurance => "endurance",
            Self::TransitionShock => "transition-shock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfWorkload {
    PowerRender,
    BoostEdge,
    HeavySpike,
    TextureRop,
    ComputeBurst,
    IdlePulse,
    MixedGame,
    FrameCadence,
    VramPressure,
    GeometryDepth,
    TextureStream,
    /// v15: true-idle (10-30 s, GPU leaves the high P-state) → instant heavy golden-checked slam.
    BoostEntry,
    /// v16.1: heavy 8-instance render (texture hops, golden-checked) with a near-full VRAM-resident
    /// pool gathered IN THE SAME per-frame submit — composite real game load (compute + texture +
    /// memory controller on the shared rail simultaneously), the highest combined draw the soak has.
    CompositeGameLoad,
}

fn golden_for_workload(goldens: RenderGoldens, workload: VfWorkload) -> Option<u32> {
    match workload {
        // BoostEntry / CompositeGameLoad slam the same 8-instance heavy frame → same golden image
        // (the VRAM gather writes to a separate sink, never the rendered texture).
        VfWorkload::PowerRender
        | VfWorkload::HeavySpike
        | VfWorkload::IdlePulse
        | VfWorkload::BoostEntry
        | VfWorkload::CompositeGameLoad => {
            Some(goldens.power)
        }
        VfWorkload::BoostEdge => Some(goldens.boost),
        VfWorkload::TextureRop => Some(goldens.texrop),
        VfWorkload::FrameCadence => Some(goldens.cadence),
        VfWorkload::GeometryDepth => Some(goldens.geometry),
        VfWorkload::TextureStream => Some(goldens.stream),
        // VramPressure is compute-path: its gather sum is a known answer, self-verified.
        VfWorkload::ComputeBurst | VfWorkload::MixedGame | VfWorkload::VramPressure => None,
    }
}

fn observe_golden_checksum(
    reference: &mut Option<u32>,
    checksum: u32,
    frame_count: u64,
) -> Result<(), String> {
    match *reference {
        None => {
            *reference = Some(checksum);
            Ok(())
        }
        Some(expected) if expected == checksum => Ok(()),
        Some(expected) => Err(format!(
            "stock render golden diverged after {frame_count} frame(s): first={expected} current={checksum}"
        )),
    }
}

fn finish_golden_capture(reference: Option<u32>, frame_count: u64) -> Result<u32, String> {
    if frame_count < GOLDEN_MIN_FRAMES {
        return Err(format!(
            "stock render golden observed only {frame_count} frame(s), need at least {GOLDEN_MIN_FRAMES}"
        ));
    }
    reference.ok_or_else(|| "stock render golden produced no checksum".to_string())
}

fn render_integrity_result(crashed: bool, diverged: bool) -> StabilityResult {
    if crashed {
        StabilityResult::Crash
    } else if diverged {
        StabilityResult::SilentError
    } else {
        StabilityResult::Stable
    }
}

/// Marginal-silicon frame-time gate for the banded/light regimes (TextureStream, BoostEdge): true
/// when the sustained average frame time exceeds the stock reference by its factor — the silicon is
/// running internal retries and slowing down before it hangs. Pure so it can be regression-tested
/// without a GPU. `stream_frame_ms_total` is summed in ms (banded path), `boost_frame_us_total` in
/// µs; `reference_us` is always µs. Returns false when there is no reference, too few frames, or
/// (stream) a hard stall already fired.
fn frame_time_degraded(
    stream_banded: bool,
    boost_edge: bool,
    stalled: bool,
    frames: u64,
    stream_frame_ms_total: u64,
    boost_frame_us_total: u64,
    reference_us: Option<u64>,
) -> bool {
    if frames < GOLDEN_MIN_FRAMES {
        return false;
    }
    let Some(reference_us) = reference_us else {
        return false;
    };
    let reference_us = reference_us.max(1);
    if stream_banded && !stalled {
        (stream_frame_ms_total * 1000) / frames > reference_us * STREAM_DEGRADATION_FACTOR
    } else if boost_edge {
        boost_frame_us_total / frames > reference_us * BOOST_EDGE_DEGRADATION_FACTOR
    } else {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VfQualifierSegment {
    phase: VfQualifierPhase,
    workload: VfWorkload,
    duration_ms: u64,
}

/// One cycle crosses distinct graphics/compute profiles and deliberate idle→heavy transitions. The
/// weights scale to the requested target duration.
fn vf_qualifier_plan(target_ms: u64, pattern: VfQualifierPattern) -> Vec<VfQualifierSegment> {
    const FSGL1: [(VfQualifierPhase, VfWorkload, u64); 10] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 5),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 7),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 5),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 7),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 15),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 4),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 7),
    ];
    const FSGL2_A: [(VfQualifierPhase, VfWorkload, u64); 12] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 4),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 6),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 5),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 8),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 4),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 5),
    ];
    const FSGL2_B: [(VfQualifierPhase, VfWorkload, u64); 13] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 4),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 6),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 6),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 6),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 8),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 3),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 3),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 4),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 4),
    ];
    const FSGL3_A: [(VfQualifierPhase, VfWorkload, u64); 12] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 4),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 7),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 4),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 7),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 10),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 4),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 5),
    ];
    const FSGL3_B: [(VfQualifierPhase, VfWorkload, u64); 13] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 4),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 7),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 7),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 10),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 3),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 4),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 4),
    ];
    const V8_HIGH_FPS: [(VfQualifierPhase, VfWorkload, u64); 18] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 8),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 12),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 4),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 12),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 8),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 12),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 3),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 12),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 5),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 8),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 8),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 4),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::GeometryDepth, VfWorkload::GeometryDepth, 6),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 8),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 6),
    ];
    // Severity ladder: graceful silent-error detectors (L2 TextureRop, cadence) run FIRST; the
    // hang-prone memory detectors (VramPressure, TextureStream) run LAST — a bad bin usually
    // dies cheaply by wrong-pixel checksum long before anything TDR-prone executes.
    const V8_TEXTURE: [(VfQualifierPhase, VfWorkload, u64); 15] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 8),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 12),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 8),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 12),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 12),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 3),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 10),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 5),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 8),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 10),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 8),
        (VfQualifierPhase::TextureStream, VfWorkload::TextureStream, 8),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 7),
    ];
    const V8_TRANSITIONS: [(VfQualifierPhase, VfWorkload, u64); 20] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 8),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 5),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::GeometryDepth, VfWorkload::GeometryDepth, 6),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 5),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 6),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 5),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 4),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 8),
    ];
    // Memory-dominant pattern: sustained multi-GB VRAM traffic interleaved with cadence, texture
    // and geometry pressure — the streaming/memory-controller co-load real games apply to the
    // shared voltage rail. BoostEdge is retained so the phase-contrast diagnostic stays defined.
    const V8_MEMORY: [(VfQualifierPhase, VfWorkload, u64); 16] = [
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 6),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 12),
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 6),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 12),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 8),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 3),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 12),
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 5),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 5),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 10),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 6),
        (VfQualifierPhase::GeometryDepth, VfWorkload::GeometryDepth, 6),
        (VfQualifierPhase::VramPressure, VfWorkload::VramPressure, 10),
        (VfQualifierPhase::ComputeBurst, VfWorkload::ComputeBurst, 4),
        (VfQualifierPhase::TextureStream, VfWorkload::TextureStream, 8),
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 6),
    ];
    // v14 candidate-only endurance soak — WORST-REALISTIC (harsher than a real game, on purpose, so a
    // PASS means real games are safe with margin — but NOT a synthetic power-virus that would reject
    // game-stable points). ONE continuous dwell, never resets mid-run, so thermal saturation truly
    // accumulates. Four ingredients, each targeting a real undervolt failure mode:
    //   1. SUSTAINED max-power (HeavySpike held) → junction/VRM/current saturation a game's average
    //      load never reaches.
    //   2. CAP-SLAM (HeavySpike burst ↔ IdlePulse release, repeated) → oscillates the VRM; under the
    //      v13 clock ceiling the driver drops VOLTAGE at the cap, not clock — the exact transient that
    //      TDR'd Godforge 1920@918 after 20 min of Overwatch.
    //   3. FrameCadence → fine frame-scale droop transients (VRM response-period sweep).
    //   4. MixedGame → game-realistic varied load for coverage.
    // The graceful golden-checked TextureRop is interleaved after every heavy block so a stress-induced
    // silent error is caught by checksum promptly (ideally before a hard TDR). HeavySpike/IdlePulse/
    // PowerRender are all golden-checked too. No new shader. CALIBRATION KNOBS (tune on real HW):
    // HeavySpike amplitude + the burst/idle weight ratio below + FrameCadence's internal gap sweep.
    // v16 COMPOSITE: VramPressure (large VRAM-resident scattered gathers — memory controller + DRAM
    // on the shared core rail) is interleaved INTO the worst-realistic soak, under sustained heat and
    // right after the cap-slam blocks. This folds in the coverage the standalone 5-min Memory pass
    // gave (which never rejected a candidate as an isolated pass) but under continuous worst load —
    // stronger, so the required Transitions/Memory patterns can be dropped (contract v14).
    const ENDURANCE: [(VfQualifierPhase, VfWorkload, u64); 23] = [
        // Warm-up ramp into load.
        (VfQualifierPhase::PowerOpening, VfWorkload::PowerRender, 3),
        // Sustained max-power saturation block (the reset-between 5-min patterns never reach this heat).
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 14),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        // Cap-slam #1: heavy burst ↔ idle release, oscillating the VRM at high (already-hot) state.
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 2),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 2),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 2),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        // v16.1 composite: heavy render + near-full VRAM-resident gather SIMULTANEOUSLY (real game
        // load: compute + texture + memory controller on the shared rail at once). Golden-checked.
        (VfQualifierPhase::CompositeGameLoad, VfWorkload::CompositeGameLoad, 14),
        // v16.2 LOBBY REGIME (field-proven killer): sustained BoostEdge — hundreds of LIGHT frames
        // per second riding the TOP of the boost curve, i.e. continuous residency AT the anchor bin
        // + kHz-scale VRM ripple. This is the OW-lobby/high-FPS regime that killed 1815@843/862 and
        // 1890@900/918 in real use while every heavy pattern passed: heavy loads sit power-bound
        // BELOW the anchor; only light frames pin the anchor itself. Golden-checked (goldens.boost).
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 12),
        // Fine droop transients + game-realistic mixed load.
        (VfQualifierPhase::FrameCadence, VfWorkload::FrameCadence, 8),
        (VfQualifierPhase::MixedGame, VfWorkload::MixedGame, 10),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        // Second saturation + cap-slam pass — heat is at its peak now, the worst point for a marginal Vmin.
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 12),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        (VfQualifierPhase::IdlePulse, VfWorkload::IdlePulse, 2),
        (VfQualifierPhase::HeavySpike, VfWorkload::HeavySpike, 3),
        // Second composite pass at peak heat.
        (VfQualifierPhase::CompositeGameLoad, VfWorkload::CompositeGameLoad, 12),
        // Second lobby pass at peak heat — anchor-bin residency when the silicon is at its worst.
        (VfQualifierPhase::BoostEdge, VfWorkload::BoostEdge, 10),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 6),
        // Cool-down close.
        (VfQualifierPhase::PowerClosing, VfWorkload::PowerRender, 3),
    ];
    // v15 candidate-only transition shock: idle→slam cycles reproduce the game/benchmark-LAUNCH
    // transition (the driver leaves the high P-state during each 10-30 s true-idle gap, then the
    // slam forces the boost-entry VF ramp + VRM load step). The graceful golden-checked TextureRop
    // between rounds catches slam-induced silent corruption; the slam wall-time stall check inside
    // BoostEntry catches the pre-hang precursor (the ~2 s BusReset cascade class) before the
    // driver watchdog. Runs BEFORE the Endurance soak: ~8 min, fail cheap first.
    const TRANSITION_SHOCK: [(VfQualifierPhase, VfWorkload, u64); 5] = [
        (VfQualifierPhase::BoostEntry, VfWorkload::BoostEntry, 12),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 4),
        (VfQualifierPhase::BoostEntry, VfWorkload::BoostEntry, 12),
        (VfQualifierPhase::TextureRop, VfWorkload::TextureRop, 4),
        (VfQualifierPhase::BoostEntry, VfWorkload::BoostEntry, 12),
    ];

    let template: &[(VfQualifierPhase, VfWorkload, u64)] = match pattern {
        VfQualifierPattern::Fsgl1 => &FSGL1,
        VfQualifierPattern::Fsgl2A => &FSGL2_A,
        VfQualifierPattern::Fsgl2B => &FSGL2_B,
        VfQualifierPattern::Fsgl3A => &FSGL3_A,
        VfQualifierPattern::Fsgl3B => &FSGL3_B,
        VfQualifierPattern::V8HighFps => &V8_HIGH_FPS,
        VfQualifierPattern::V8Texture => &V8_TEXTURE,
        VfQualifierPattern::V8Transitions => &V8_TRANSITIONS,
        VfQualifierPattern::V8Memory => &V8_MEMORY,
        VfQualifierPattern::Endurance => &ENDURANCE,
        VfQualifierPattern::TransitionShock => &TRANSITION_SHOCK,
    };
    let total_weight = template.iter().map(|(_, _, weight)| *weight).sum::<u64>();
    let mut assigned = 0u64;
    template
        .iter()
        .enumerate()
        .map(|(index, &(phase, workload, weight))| {
            let duration_ms = if index + 1 == template.len() {
                target_ms.saturating_sub(assigned)
            } else {
                target_ms.saturating_mul(weight) / total_weight
            };
            assigned = assigned.saturating_add(duration_ms);
            VfQualifierSegment { phase, workload, duration_ms }
        })
        .collect()
}

/// Distinct phases a pattern's plan exercises — the qualification-coverage denominator.
/// Legacy FSGL patterns cover 8 phases; the v7 patterns add FrameCadence (9).
pub fn qualifier_expected_phases(pattern: VfQualifierPattern) -> u32 {
    let mut seen = [false; VfQualifierPhase::COUNT];
    for segment in vf_qualifier_plan(1_000, pattern) {
        seen[segment.phase.code() as usize] = true;
    }
    seen.iter().filter(|present| **present).count() as u32
}

const ALU_SHADER: &str = r#"
struct P { iters: u32, n: u32, p0: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var x = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        x = x * 1664525u + 1013904223u;
    }
    data[i] = x;
}
"#;

// Max-power virus: each lane drives BOTH datapaths at once — an integer vec4 LCG
// (verified via jump-ahead) AND a float vec4 FMA chain (the hottest op on NVIDIA;
// FP32 cores dominate the power budget). Pure integer ALU only reached ~80% of a
// 200 W cap on a 3060 Ti; mixing in independent FP FMAs pushes the draw up near
// the real cap so high-voltage points actually throttle and the V↔W map is real.
// The float result is written to a sink buffer so the compiler can't eliminate
// it; it isn't verified (only the integer chains are the known-answer).
const POWER_SHADER: &str = r#"
struct P { iters: u32, n: u32, p0: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read_write> fsink: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var v = data[i];
    let fi = f32(i);
    // Integer ALU + two independent FP FMA chains (FP32 cores). Compute-only and
    // register-resident: this gives the highest, steadiest sustained draw (a
    // strided VRAM RMW made it latency-bound and COLDER, 134W vs 188W). It can't
    // replicate a real game's transient power spikes — that's handled by margin.
    var f = vec4<f32>(fi * 1e-3 + 1.0, fi * 2e-3 + 1.5, fi * 3e-3 + 2.0, fi * 1.5e-3 + 0.5);
    var g = vec4<f32>(fi * 1.7e-3 + 1.1, fi * 0.9e-3 + 1.3, fi * 2.3e-3 + 1.9, fi * 1.2e-3 + 0.7);
    let m = vec4<f32>(1.0000001);
    let a = vec4<f32>(0.9999999);
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        v = v * 1664525u + vec4<u32>(1013904223u);
        f = fma(f, m, a);
        g = fma(g, a, m);
        f = fma(f, a, m);
        g = fma(g, m, a);
        f = fma(f, m, a);
        g = fma(g, a, m);
    }
    data[i] = v;
    fsink[i] = f + g;
}
"#;

// Each lane gathers from a LARGE (VRAM-resident) table, striding across the
// whole table so accesses miss cache and hit the memory controller / DRAM —
// the path that shares the core voltage rail.
const MEM_SHADER: &str = r#"
struct P { gathers: u32, lanes: u32, table_len: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read> table: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.lanes) { return; }
    var acc: u32 = 0u;
    for (var k: u32 = 0u; k < p.gathers; k = k + 1u) {
        let idx = (i * 2654435761u + k * 2246822519u) % p.table_len;
        acc = acc + table[idx];
    }
    data[i] = acc;
}
"#;

// Fills the large table on the GPU so we don't upload hundreds of MB. Uses a
// grid-stride loop so a bounded workgroup count covers a huge buffer (the X
// dispatch dimension is capped at 65535).
// v16.1 composite: cache-defeating scattered gather over a near-full VRAM-resident pool, run in the
// SAME per-frame submit as the heavy render. Pure memory-controller/DRAM bandwidth pressure on the
// shared core rail; the XOR into `sink` (never the render texture) only exists so the reads are not
// dead-code-eliminated — no correctness claim on the pool (the render golden is the detector).
const COMPOSITE_GATHER_SHADER: &str = r#"
struct GP { table_len: u32, gathers: u32, seed: u32, pad: u32 };
@group(0) @binding(0) var<storage, read> table: array<u32>;
@group(0) @binding(1) var<storage, read_write> sink: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> p: GP;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    var acc = p.seed ^ gid.x;
    var idx = (gid.x * 2654435761u) % p.table_len;
    for (var k = 0u; k < p.gathers; k = k + 1u) {
        acc = acc ^ table[idx];
        idx = (idx * 1664525u + 1013904223u) % p.table_len;
    }
    atomicXor(&sink[gid.x & 255u], acc);
}
"#;

const FILL_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> table: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        table[i] = i * 2246822519u;
        i = i + stride;
    }
}
"#;

// Memory-bandwidth kernel: read+write every element (8 bytes moved/elem) with
// minimal compute → DRAM-bandwidth bound. Grid-stride for huge buffers.
const BW_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> buf: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        buf[i] = buf[i] * 1664525u + 1013904223u;
        i = i + stride;
    }
}
"#;

// Fused core+memory kernel: each lane runs heavy LCG rounds (core ALU) AND, in
// the SAME loop, streams a large VRAM buffer with a read-modify-write (memory
// bandwidth) — so the shader cores and the memory controller are loaded
// *simultaneously per invocation*, like a real game shader. (Separate ALU and
// bandwidth dispatches in one pass run sequentially and don't co-load: the
// memory dispatch just adds bubbles that lower utilization.) `data[i]` is a
// pure LCG of its seed (verifiable via jump-ahead); the `buf` RMW is load-only.
const FUSED_SHADER: &str = r#"
struct P { iters: u32, n: u32, buf_n: u32, pad: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read_write> buf: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var x = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        x = x * 1664525u + 1013904223u;
        let idx = (i * 2654435761u + k * 2246822519u) % p.buf_n;
        buf[idx] = buf[idx] + x;
    }
    data[i] = x;
}
"#;

// Pointer-chasing chain: chain[i] = (i*CP + CQ) & mask — a permutation of a
// power-of-two-sized buffer. Following it does data-dependent random reads
// (memory-latency bound) where ANY uncorrected error sends the chase down a
// wrong address and cascades into a totally different result — far more
// sensitive to memory/addressing faults than a linear read/verify (which the
// GDDR6 link CRC tends to mask).
const CHAIN_CP: u32 = 2654435761;
const CHAIN_CQ: u32 = 1442695041;

const CHAIN_FILL_SHADER: &str = r#"
struct P { n: u32, mask: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> chain: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        chain[i] = (i * 2654435761u + 1442695041u) & p.mask;
        i = i + stride;
    }
}
"#;

const CHASE_SHADER: &str = r#"
struct P { steps: u32, mask: u32, lanes: u32, p2: u32 };
@group(0) @binding(0) var<storage, read> chain: array<u32>;
@group(0) @binding(1) var<storage, read_write> out: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let lane = gid.x;
    if (lane >= p.lanes) { return; }
    var idx = (lane * 2654435761u + 1442695041u) & p.mask;
    for (var k: u32 = 0u; k < p.steps; k = k + 1u) {
        idx = chain[idx] & p.mask;
    }
    out[lane] = idx;
}
"#;

// Real RENDER-pipeline stress (not compute): a heavy fragment shader over many
// overlapping instanced triangles (overdraw → raster + ROP + TMU + fragment ALU,
// the game path compute never touches). The output is a deterministic function
// of pixel position + instance, so a stable GPU renders the SAME frame every
// time; a diverging frame checksum is a silent error before a hard crash.
const RENDER_SHADER: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let a = f32(ii) * 0.137;
    let s = sin(a); let c = cos(a);
    let v = p[vi];
    let r = vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
    var o: VOut;
    o.pos = vec4<f32>(r * 0.999, 0.0, 1.0);
    o.uv = (v + vec2<f32>(1.0, 1.0)) * 0.5;
    o.seed = f32(ii);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // FurMark-class fragment: four independent FP32 FMA chains (ILP → saturates
    // the FP cores) PLUS heavy DEPENDENT texture sampling (TMU + L2/DRAM — the
    // game activity a flat-shaded render misses, which kept us at ~160W vs a
    // game's ~199W at the same V/clock), over heavy overdraw → raster + ROP + TMU
    // + FP all at once, like a worst-case game.
    var a = in.uv.x * 64.0 + in.seed;
    var b = in.uv.y * 64.0 - in.seed;
    var c = a * 1.3 + b * 0.7 + 1.0;
    var d = a - b * 1.1 + 2.0;
    var t = vec4<f32>(0.0);
    for (var k = 0; k < 96; k = k + 1) {
        a = fma(a, 1.0001, 0.013);
        b = fma(b, 0.9997, 0.017);
        c = fma(c, 1.0003, a);
        d = fma(d, 0.9994, b);
        // Dependent texture taps: UVs derived from the FP state so the sampler
        // can't be hoisted — pulls real TMU + cache/memory traffic per pixel.
        let uv0 = in.uv + vec2<f32>(fract(a * 0.01), fract(b * 0.01));
        let uv1 = in.uv * 4.0 + vec2<f32>(fract(c * 0.013), fract(d * 0.017));
        t = t + textureSampleLevel(tex, samp, uv0, 0.0) + textureSampleLevel(tex, samp, uv1, 0.0);
        c = c + sin(a) * 0.001 + t.x * 0.0001;
        d = d + cos(b) * 0.001 + t.y * 0.0001;
    }
    let v = fract(abs(a + b + c + d + t.x + t.z) * 0.00037);
    return vec4<f32>(v, fract(v * 7.0), fract(v * 13.0), 1.0);
}
"#;

// High-FPS / medium-power graphics path: same deterministic target, but a much cheaper fragment
// shader and one instance. It keeps boost high without pinning the board at its power limit.
const BOOST_EDGE_SHADER: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    o.uv = (p[vi] + vec2<f32>(1.0, 1.0)) * 0.5;
    o.seed = f32(ii);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var a = in.uv.x * 11.0 + 0.17;
    var b = in.uv.y * 13.0 + 0.29;
    for (var k = 0; k < 12; k = k + 1) {
        a = fma(a, 1.001, b * 0.003);
        b = fma(b, 0.999, a * 0.002);
    }
    let t = textureSampleLevel(tex, samp, fract(in.uv * 2.0 + vec2<f32>(a, b) * 0.001), 0.0);
    let v = fract(abs(a + b + t.x + t.y) * 0.031);
    return vec4<f32>(v, fract(v * 3.0), fract(v * 5.0), 1.0);
}
"#;

// TextureRop samples a LARGE VRAM-resident source (not the 1024² ≈ L2-resident one) with the tap
// chain SCATTERED per pixel, so neighbouring fragments hit far-apart texels and bilinear taps pay
// DRAM latency — TMU + memory controller together on the shared rail, the game texturing path.
// The dimension is FIXED (not probed at runtime): golden capture and qualifier run in separate
// GpuCtx instances, and any runtime size fallback could diverge between them and turn a benign
// allocation difference into a false SilentError. 8192² RGBA8 = 256 MB, far beyond any L2.
const TEXTURE_STREAM_DIM: u32 = 8192;

// Fills the large source texture on the GPU (uploading 256 MB from the CPU is not acceptable):
// one fullscreen triangle whose fragment shader hashes the pixel coordinate — deterministic
// content, same everywhere it is created.
const TEXTURE_STREAM_FILL_SHADER: &str = r#"
struct VOut { @builtin(position) pos: vec4<f32> };
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let x = u32(in.pos.x);
    let y = u32(in.pos.y);
    var h = (y * 8192u + x) * 2654435761u;
    h = h ^ (h >> 16u);
    h = h * 2246822519u;
    return vec4<f32>(
        f32(h & 0xffu) / 255.0,
        f32((h >> 8u) & 0xffu) / 255.0,
        f32((h >> 16u) & 0xffu) / 255.0,
        1.0,
    );
}
"#;

// Texture/ROP-biased path: dependent texture sampling and alpha blending dominate while the ALU
// chain stays deliberately lighter than PowerRender.
const TEXTURE_ROP_SHADER: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4<f32>(p[vi] * (0.997 - f32(ii) * 0.0005), 0.0, 1.0);
    o.uv = (p[vi] + vec2<f32>(1.0, 1.0)) * 0.5;
    o.seed = f32(ii) * 0.071;
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var uv = fract(in.uv * 3.0 + in.seed);
    var t = vec4<f32>(0.0);
    for (var k = 0; k < 48; k = k + 1) {
        uv = fract(uv * vec2<f32>(1.013, 0.991) + vec2<f32>(0.017, 0.029));
        t = t + textureSampleLevel(tex, samp, uv, 0.0);
        t = t + textureSampleLevel(tex, samp, fract(uv.yx * 1.7), 0.0);
        t = t + textureSampleLevel(tex, samp, fract(uv * 4.1 + t.xy * 0.0001), 0.0);
    }
    let v = fract((t.x + t.y + t.z) * 0.0031);
    return vec4<f32>(v, fract(v * 7.0), fract(v * 11.0), 0.55);
}
"#;

// TextureStream: the HANG-PRONE heavy memory detector, split out of TextureRop (which stays the
// L2-resident graceful silent-error detector — replacing it in v10 traded wrong-pixel failures
// for driver hangs/TDR on hardware). Samples the large VRAM-resident source with the tap chain
// start SCATTERED per pixel: neighbouring fragments hit far-apart texels, every bilinear tap
// pays DRAM latency — TMU + memory controller together under droop. Runs LAST in patterns
// (severity ladder) and is rendered in scissor BANDS with one submit each, so the driver can
// preempt between bands (desktop stays responsive) and a stalling band is caught well before
// the ~2 s TDR watchdog.
const TEXTURE_STREAM_SHADER: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    o.uv = (p[vi] + vec2<f32>(1.0, 1.0)) * 0.5;
    o.seed = f32(ii) * 0.113;
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var uv = vec2<f32>(
        fract(sin(dot(in.pos.xy, vec2<f32>(127.1, 311.7)) + in.seed) * 43758.5453),
        fract(sin(dot(in.pos.xy, vec2<f32>(269.5, 183.3)) - in.seed) * 43758.5453),
    );
    var t = vec4<f32>(0.0);
    for (var k = 0; k < 24; k = k + 1) {
        uv = fract(uv * vec2<f32>(1.013, 0.991) + vec2<f32>(0.017, 0.029) + t.xy * 0.0001);
        t = t + textureSampleLevel(tex, samp, uv, 0.0);
        t = t + textureSampleLevel(tex, samp, fract(uv.yx * 1.7 + vec2<f32>(0.31, 0.17)), 0.0);
    }
    let v = fract((t.x + t.y + t.z) * 0.0037);
    return vec4<f32>(v, fract(v * 7.0), fract(v * 13.0), 1.0);
}
"#;

// Geometry/raster/depth path: many small procedurally-placed triangles (no vertex buffer — position
// hashed from vertex/instance index) rendered under a depth test. Loads vertex fetch/transform,
// triangle setup, raster and depth-ROP — units every fullscreen-triangle workload skips. Each
// triangle gets a UNIQUE depth (hashed, no ties) so the depth test resolves deterministically
// regardless of rasterization order and the frame checksum is stable on a healthy GPU.
const GEOMETRY_DEPTH_TRIS: u32 = 49_152;
const GEOMETRY_DEPTH_INSTANCES: u32 = 8;
const GEOMETRY_DEPTH_SHADER: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
fn hash(x: u32) -> u32 {
    var h = x * 2654435761u;
    h = h ^ (h >> 16u);
    h = h * 2246822519u;
    return h ^ (h >> 13u);
}
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let tri = vi / 3u;
    let corner = vi % 3u;
    let id = tri * 8u + ii;
    let h0 = hash(id);
    let h1 = hash(id ^ 0x9e3779b9u);
    // Center in clip space, small triangle, unique depth per (tri, instance).
    let cx = (f32(h0 & 0xffffu) / 32768.0) - 1.0;
    let cy = (f32(h0 >> 16u) / 32768.0) - 1.0;
    let size = 0.006 + f32(h1 & 0xffu) / 8192.0;
    var off = vec2<f32>(0.0, size);
    if (corner == 1u) { off = vec2<f32>(-size, -size); }
    if (corner == 2u) { off = vec2<f32>(size, -size); }
    let z = (f32(id) + 0.5) / f32(49152u * 8u);
    var o: VOut;
    o.pos = vec4<f32>(cx + off.x, cy + off.y, z, 1.0);
    o.uv = vec2<f32>(cx, cy) * 0.5 + vec2<f32>(0.5, 0.5);
    o.seed = f32(h1 & 0x3ffu);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var a = in.seed * 0.013 + in.uv.x;
    var b = in.seed * 0.007 + in.uv.y;
    for (var k = 0; k < 8; k = k + 1) {
        a = fma(a, 1.003, b * 0.011);
        b = fma(b, 0.997, a * 0.009);
    }
    let t = textureSampleLevel(tex, samp, fract(in.uv * 3.0 + vec2<f32>(a, b) * 0.001), 0.0);
    let v = fract(abs(a + b + t.x + t.y) * 0.043);
    return vec4<f32>(v, fract(v * 5.0), fract(v * 9.0), 1.0);
}
"#;

// Sum every texel (as u32) into one atomic — a whole-frame checksum read back as
// 4 bytes, so frame-to-frame determinism is cheap to verify.
const REDUCE_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read> buf: array<u32>;
@group(0) @binding(1) var<storage, read_write> res: atomic<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = g.x; var acc = 0u;
    loop { if (i >= p.n) { break; } acc = acc + buf[i]; i = i + stride; }
    atomicAdd(&res, acc);
}
"#;

// Positional checksum for FSGL3. Each invocation hashes a deterministic grid-stride
// lane, then atomically folds lanes by addition so scheduling order cannot change it.
const REDUCE3_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read> buf: array<u32>;
@group(0) @binding(1) var<storage, read_write> res: atomic<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = g.x;
    var h = 2166136261u;
    loop {
        if (i >= p.n) { break; }
        h = (h ^ (buf[i] ^ (i * 2654435761u))) * 16777619u;
        i = i + stride;
    }
    atomicAdd(&res, h);
}
"#;

const CHECKSUM_COMPARE_SHADER: &str = r#"
struct P { golden: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> sum: atomic<u32>;
@group(0) @binding(1) var<storage, read_write> mismatches: atomic<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(1)
fn main() {
    if (atomicLoad(&sum) != p.golden) {
        atomicAdd(&mismatches, 1u);
    }
}
"#;

// VRAM integrity: write a deterministic pattern, then verify it bit-for-bit,
// counting mismatches on the GPU (only the count is read back).
const VRAM_PATTERN_FN: &str = r#"
fn pattern(i: u32, mode: u32) -> u32 {
    if (mode == 0u) { return i * 2654435761u; }       // address-in-cell
    if (mode == 1u) { return 0xffffffffu; }           // all ones
    if (mode == 2u) { return 0u; }                    // all zeros
    if (mode == 3u) { if ((i & 1u) == 0u) { return 0xaaaaaaaau; } return 0x55555555u; } // checkerboard
    return 1u << (i % 32u);                            // walking bit
}
"#;

#[cfg(test)]
fn lcg(seed: u32, iters: u32) -> u32 {
    let mut x = seed;
    for _ in 0..iters {
        x = x.wrapping_mul(C1).wrapping_add(C2);
    }
    x
}

/// f(x) = C1*x + C2 (mod 2^32). Returns (A, C) for f^n via fast exponentiation,
/// so `f^n(seed) = A*seed + C` — lets us verify any number of GPU rounds in
/// O(log n) on the CPU instead of replaying the loop.
fn lcg_pow(n: u64) -> (u32, u32) {
    // Compose: apply `h` then `g` → g(h(x)) = (g.A*h.A)*x + (g.A*h.C + g.C).
    fn compose(h: (u32, u32), g: (u32, u32)) -> (u32, u32) {
        (g.0.wrapping_mul(h.0), g.0.wrapping_mul(h.1).wrapping_add(g.1))
    }
    let mut result = (1u32, 0u32); // identity
    let mut base = (C1, C2); // f^1
    let mut e = n;
    while e > 0 {
        if e & 1 == 1 {
            result = compose(result, base);
        }
        base = compose(base, base);
        e >>= 1;
    }
    result
}

#[cfg(test)]
fn lcg_jump(seed: u32, n: u64) -> u32 {
    let (a, c) = lcg_pow(n);
    a.wrapping_mul(seed).wrapping_add(c)
}

#[cfg(test)]
fn reduce3_checksum_cpu(buf: &[u32], lanes: u32) -> u32 {
    let mut total = 0u32;
    for lane in 0..lanes {
        let mut i = lane as usize;
        let mut h = 2166136261u32;
        while i < buf.len() {
            h = (h ^ (buf[i] ^ (i as u32).wrapping_mul(2654435761)))
                .wrapping_mul(16777619);
            i += lanes as usize;
        }
        total = total.wrapping_add(h);
    }
    total
}

/// (A, C) for `f^n` where f(x) = (CHAIN_CP*x + CHAIN_CQ) mod (mask+1), so the
/// pointer-chase result is `(A*start + C) & mask` — verified in O(log n).
fn affine_pow_mod(n: u64, mask: u32) -> (u32, u32) {
    fn compose(h: (u32, u32), g: (u32, u32), mask: u32) -> (u32, u32) {
        (
            g.0.wrapping_mul(h.0) & mask,
            (g.0.wrapping_mul(h.1).wrapping_add(g.1)) & mask,
        )
    }
    let mut result = (1u32 & mask, 0u32);
    let mut base = (CHAIN_CP & mask, CHAIN_CQ & mask);
    let mut e = n;
    while e > 0 {
        if e & 1 == 1 {
            result = compose(result, base, mask);
        }
        base = compose(base, base, mask);
        e >>= 1;
    }
    result
}

/// A live GPU device for running the battery (set up once, reused per stage).
pub struct GpuCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter_name: String,
    /// Largest single storage buffer we may allocate (bytes).
    pub max_buffer_bytes: u64,
    crashed: Arc<AtomicBool>,
}

impl GpuCtx {
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "no suitable GPU adapter found".to_string())?;
        let adapter_name = adapter.get_info().name;
        // Request the adapter's full limits so we can allocate large
        // VRAM-resident buffers (cache-busting + VRAM coverage).
        let limits = adapter.limits();
        let max_buffer_bytes = limits.max_storage_buffer_binding_size as u64;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nidavellir-gpu-stress"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
            },
            None,
        ))
        .map_err(|e| format!("request_device failed: {e}"))?;

        let crashed = Arc::new(AtomicBool::new(false));
        {
            let crashed = crashed.clone();
            device.on_uncaptured_error(Box::new(move |e| {
                eprintln!("wgpu device error: {e}");
                crashed.store(true, Ordering::SeqCst);
            }));
        }
        Ok(Self { device, queue, adapter_name, max_buffer_bytes, crashed })
    }

    fn verdict(&self, mismatches: u32, mapped_ok: bool) -> StabilityResult {
        if !mapped_ok || self.crashed.load(Ordering::SeqCst) {
            StabilityResult::Crash
        } else if mismatches == 0 {
            StabilityResult::Stable
        } else {
            StabilityResult::SilentError
        }
    }

    /// Sustained ALU known-answer test: dispatches the LCG kernel back-to-back
    /// for ~`target_ms`, keeping the GPU **saturated** (the buffer accumulates,
    /// so after K dispatches every lane has had `iters*K` LCG rounds). Verified
    /// via LCG jump-ahead, so the CPU reference is O(log n) regardless of load.
    pub fn run_alu(&self, name: &str, elements: u32, iters: u32, target_ms: u64) -> StageReport {
        self.run_alu_with_cancel(name, elements, iters, target_ms, None)
    }

    pub fn run_alu_with_cancel(
        &self,
        name: &str,
        elements: u32,
        iters: u32,
        target_ms: u64,
        cancel: Option<&AtomicBool>,
    ) -> StageReport {
        let start = std::time::Instant::now();
        let input: Vec<u32> = (0..elements).collect();
        let byte_size = (elements as usize * 4) as u64;

        let data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("alu-data"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("alu-params"),
            contents: bytemuck::bytes_of(&Params { a: iters, n: elements, _pad: [0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("alu"),
            source: wgpu::ShaderSource::Wgsl(ALU_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("alu"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("alu"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });

        let groups = elements.div_ceil(64);
        let mut k: u64 = 0;
        // Keep the queue fed back-to-back; bound depth with an occasional wait.
        while (start.elapsed().as_millis() as u64) < target_ms
            && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("alu"),
                    timestamp_writes: None,
                });
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // expected[i] = f^(iters*k)(i) where f(x)=a*x+c, via fast exponentiation.
        let (a, c) = lcg_pow((iters as u64).wrapping_mul(k));
        let expected: Vec<u32> = (0..elements).map(|i| a.wrapping_mul(i).wrapping_add(c)).collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, byte_size, &expected);
        StageReport {
            name: name.to_string(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Sustained MAX-POWER load: int vec4 LCG (known-answer checked) + float vec4
    /// FMA (load, drives the FP32 cores) per lane — both datapaths at once for the
    /// hottest draw, so the power sweep's high-voltage points reach the cap. The
    /// integer chains are the known-answer; the float sink is load-only.
    pub fn run_power_load(&self, elements: u32, iters: u32, target_ms: u64) -> StageReport {
        const S1: u32 = 0x9e3779b9;
        const S2: u32 = 0x85ebca6b;
        const S3: u32 = 0xc2b2ae35;
        let start = std::time::Instant::now();

        // Four independent LCG seeds per lane, packed as vec4.
        let mut input: Vec<u32> = Vec::with_capacity(elements as usize * 4);
        for i in 0..elements {
            input.push(i);
            input.push(i ^ S1);
            input.push(i ^ S2);
            input.push(i ^ S3);
        }
        let byte_size = (elements as usize * 16) as u64;

        let data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pwr-data"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        // Float sink: forces the FP FMA work to execute (can't be optimized out).
        let fsink = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pwr-fsink"),
            size: (elements as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pwr-params"),
            contents: bytemuck::bytes_of(&Params { a: iters, n: elements, _pad: [0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pwr"),
            source: wgpu::ShaderSource::Wgsl(POWER_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pwr"), layout: None, module: &module, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pwr"), layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fsink.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = elements.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        let (a, c) = lcg_pow((iters as u64).wrapping_mul(k));
        let mut expected: Vec<u32> = Vec::with_capacity(elements as usize * 4);
        for i in 0..elements {
            expected.push(a.wrapping_mul(i).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S1).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S2).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S3).wrapping_add(c));
        }
        let (mismatches, mapped_ok) = self.readback_compare(&data, byte_size, &expected);
        StageReport {
            name: "PowerLoad".into(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Sustained memory-bound known-answer test. `lanes` output lanes each gather
    /// `gathers` times from a LARGE (~256 MB) VRAM-resident table, striding so the
    /// reads miss cache and traverse the memory controller — the path on the core
    /// voltage rail. Idempotent → CPU reference computed once (table values via
    /// the same on-the-fly hash, so it isn't stored on the CPU).
    pub fn run_memory(&self, name: &str, lanes: u32, gathers: u32, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();

        // Large table sized to ~256 MB (capped by device limits), cache-busting.
        let target_table_bytes = 256u64 * 1024 * 1024;
        let table_bytes = target_table_bytes.min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let table_len = (table_bytes / 4) as u32;
        let out_bytes = (lanes as usize * 4) as u64;

        let table = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mem-table"),
            size: (table_len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let data = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mem-data"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Fill the table on the GPU (avoids a 256 MB upload).
        let fill_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fill-params"),
            contents: bytemuck::bytes_of(&Quad { a: table_len, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fill"),
            source: wgpu::ShaderSource::Wgsl(FILL_SHADER.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fill"),
            layout: &fill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fill_params.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&fill_pipe);
                cp.set_bind_group(0, &fill_bind, &[]);
                cp.dispatch_workgroups(table_len.div_ceil(64).min(65535), 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mem-params"),
            contents: bytemuck::bytes_of(&Quad { a: gathers, b: lanes, c: table_len, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mem"),
            source: wgpu::ShaderSource::Wgsl(MEM_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mem"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mem"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = lanes.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // table[idx] = idx * TABLE_INIT (matches FILL_SHADER) — computed inline.
        let expected: Vec<u32> = (0..lanes)
            .map(|i| {
                let mut acc = 0u32;
                for kk in 0..gathers {
                    let idx = (i.wrapping_mul(HASH1).wrapping_add(kk.wrapping_mul(TABLE_INIT))) % table_len;
                    acc = acc.wrapping_add(idx.wrapping_mul(TABLE_INIT));
                }
                acc
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, out_bytes, &expected);
        StageReport {
            name: name.to_string(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Multi-gigabyte VRAM-pressure qualifier load: several ~256 MB VRAM-resident tables are
    /// gathered with cache-defeating strides, cycling across tables per dispatch so the DRAM
    /// footprint far exceeds any cache — the memory-controller/DRAM co-load on the shared core
    /// rail that render targets (L2-resident) never produce. Known-answer verified: the gather
    /// sum is idempotent per dispatch and identical for every table, so ANY mismatch is a silent
    /// error. Table allocation is OOM-guarded (error scope) and degrades to fewer tables — the
    /// first table always allocates (256 MB fits any supported GPU alongside the working set).
    pub fn run_vram_pressure_with_cancel(
        &self,
        target_ms: u64,
        cancel: Option<&AtomicBool>,
    ) -> StageReport {
        const LANES: u32 = 262_144;
        const GATHERS: u32 = 128;
        /// Upper bound on tables (~2 GB at 256 MB each) — sized to pressure the memory
        /// controller without risking overcommit on 4 GB cards.
        const MAX_TABLES: usize = 8;
        let start = std::time::Instant::now();
        let table_bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let table_len = (table_bytes / 4) as u32;

        let mut tables: Vec<wgpu::Buffer> = Vec::new();
        for _ in 0..MAX_TABLES {
            // OOM-guarded: a failed allocation pops as an error (not a device loss) and we keep
            // the tables already resident.
            self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let table = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vram-pressure-table"),
                size: (table_len as u64) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            if pollster::block_on(self.device.pop_error_scope()).is_some() {
                break;
            }
            tables.push(table);
        }
        if tables.is_empty() {
            // Allocation failed outright — inconclusive hardware state, not proof of stability.
            return StageReport {
                name: "VramPressure".into(),
                result: StabilityResult::Crash,
                mismatches: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Fill every table with the same deterministic pattern (on-GPU; matches FILL_SHADER).
        let fill_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vram-pressure-fill-params"),
            contents: bytemuck::bytes_of(&Quad { a: table_len, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-pressure-fill"),
            source: wgpu::ShaderSource::Wgsl(FILL_SHADER.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-pressure-fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        for table in &tables {
            let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vram-pressure-fill"),
                layout: &fill_pipe.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: table.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: fill_params.as_entire_binding() },
                ],
            });
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&fill_pipe);
                cp.set_bind_group(0, &fill_bind, &[]);
                cp.dispatch_workgroups(table_len.div_ceil(64).min(65535), 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        let data = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vram-pressure-data"),
            size: (LANES as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vram-pressure-params"),
            contents: bytemuck::bytes_of(&Quad { a: GATHERS, b: LANES, c: table_len, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-pressure"),
            source: wgpu::ShaderSource::Wgsl(MEM_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-pressure"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let binds: Vec<wgpu::BindGroup> = tables
            .iter()
            .map(|table| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vram-pressure"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    ],
                })
            })
            .collect();

        let groups = LANES.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms
            && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &binds[(k % binds.len() as u64) as usize], &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // Same known answer as run_memory — every table holds the identical pattern, so the
        // expected gather sum is independent of which table each dispatch hit.
        let expected: Vec<u32> = (0..LANES)
            .map(|i| {
                let mut acc = 0u32;
                for kk in 0..GATHERS {
                    let idx = (i.wrapping_mul(HASH1).wrapping_add(kk.wrapping_mul(TABLE_INIT)))
                        % table_len;
                    acc = acc.wrapping_add(idx.wrapping_mul(TABLE_INIT));
                }
                acc
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, (LANES as u64) * 4, &expected);
        StageReport {
            name: "VramPressure".into(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// VRAM integrity check (roadmap §12 Phase 1): allocate a large VRAM buffer,
    /// write/verify deterministic patterns (address-in-cell, all-1/0,
    /// checkerboard, walking-bit), counting mismatches on the GPU. Run at stock
    /// before tuning — a failure here means the memory itself is unstable.
    pub fn run_vram_check(&self, target_bytes: u64, passes: u32) -> StageReport {
        let start = std::time::Instant::now();
        let bytes = target_bytes.min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let len = (bytes / 4) as u32;

        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vram-buf"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let result = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vram-result"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fill_src = format!(
            "{VRAM_PATTERN_FN}\nstruct P {{ mode: u32, n: u32, p0: u32, p1: u32 }};\n\
             @group(0) @binding(0) var<storage, read_write> buf: array<u32>;\n\
             @group(0) @binding(1) var<uniform> p: P;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n\
               let stride = nwg.x * 64u; var i = g.x; loop {{ if (i >= p.n) {{ break; }} buf[i] = pattern(i, p.mode); i = i + stride; }}\n}}"
        );
        let verify_src = format!(
            "{VRAM_PATTERN_FN}\nstruct P {{ mode: u32, n: u32, p0: u32, p1: u32 }};\n\
             @group(0) @binding(0) var<storage, read> buf: array<u32>;\n\
             @group(0) @binding(1) var<uniform> p: P;\n\
             @group(0) @binding(2) var<storage, read_write> res: atomic<u32>;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n\
               let stride = nwg.x * 64u; var i = g.x; loop {{ if (i >= p.n) {{ break; }} if (buf[i] != pattern(i, p.mode)) {{ atomicAdd(&res, 1u); }} i = i + stride; }}\n}}"
        );
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-fill"),
            source: wgpu::ShaderSource::Wgsl(fill_src.into()),
        });
        let verify_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-verify"),
            source: wgpu::ShaderSource::Wgsl(verify_src.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let verify_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-verify"),
            layout: None,
            module: &verify_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });

        let groups = len.div_ceil(64).min(65535);
        // Repeat write+read cycles per pattern so the memory is actually
        // hammered at its clock (a single fill→verify is too quick to surface
        // marginal instability — exactly why a too-high mem OC used to pass).
        const VRAM_REPS: u32 = 10;
        let mut total_mismatches = 0u32;
        'outer: for _ in 0..passes.max(1) {
            for mode in 0u32..5 {
                let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vram-params"),
                    contents: bytemuck::bytes_of(&Quad { a: mode, b: len, c: 0, d: 0 }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vram-fill"),
                    layout: &fill_pipe.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
                    ],
                });
                let verify_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vram-verify"),
                    layout: &verify_pipe.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: result.as_entire_binding() },
                    ],
                });
                self.queue.write_buffer(&result, 0, &0u32.to_ne_bytes());
                let mut enc = self.device.create_command_encoder(&Default::default());
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    for _ in 0..VRAM_REPS {
                        cp.set_pipeline(&fill_pipe);
                        cp.set_bind_group(0, &fill_bind, &[]);
                        cp.dispatch_workgroups(groups, 1, 1);
                        cp.set_pipeline(&verify_pipe);
                        cp.set_bind_group(0, &verify_bind, &[]);
                        cp.dispatch_workgroups(groups, 1, 1);
                    }
                }
                self.queue.submit(Some(enc.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                total_mismatches = total_mismatches.saturating_add(self.read_u32(&result));
                if self.crashed.load(Ordering::SeqCst) {
                    break 'outer;
                }
            }
        }

        StageReport {
            name: format!("VRAM ({} MB)", bytes / (1024 * 1024)),
            result: self.verdict(total_mismatches, !self.crashed.load(Ordering::SeqCst)),
            mismatches: total_mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Pointer-chasing memory test: data-dependent random reads through a chain
    /// in a large VRAM buffer, sustained for ~`target_ms`. Catches uncorrected
    /// memory/addressing errors that a linear read/verify misses (a wrong read
    /// derails the whole chase). Returns SilentError on any divergence.
    pub fn run_mem_chase(&self, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();
        let want_bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        // Chain length must be a power of two (mask = len-1).
        let mut len: u32 = 1 << 26; // 64M = 256 MB
        while (len as u64) * 4 > want_bytes && len > (1 << 20) {
            len >>= 1;
        }
        let mask = len - 1;
        let lanes: u32 = 65_536;
        let steps: u32 = 8_192;

        let chain = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chain"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chase-out"),
            size: (lanes as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Fill the chain.
        let fill_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chain-fill-params"),
            contents: bytemuck::bytes_of(&Quad { a: len, b: mask, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chain-fill"),
            source: wgpu::ShaderSource::Wgsl(CHAIN_FILL_SHADER.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("chain-fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chain-fill"),
            layout: &fill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fill_params.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&fill_pipe);
                cp.set_bind_group(0, &fill_bind, &[]);
                cp.dispatch_workgroups(len.div_ceil(64).min(65535), 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        // Chase.
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chase-params"),
            contents: bytemuck::bytes_of(&Quad { a: steps, b: mask, c: lanes, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chase"),
            source: wgpu::ShaderSource::Wgsl(CHASE_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("chase"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chase"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = lanes.div_ceil(64);
        let mut k = 0u64;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // Reference: out[lane] = f^steps(start_lane), start_lane = (lane*CP+CQ)&mask.
        let (a, c) = affine_pow_mod(steps as u64, mask);
        let expected: Vec<u32> = (0..lanes)
            .map(|lane| {
                let s = (lane.wrapping_mul(CHAIN_CP).wrapping_add(CHAIN_CQ)) & mask;
                (a.wrapping_mul(s).wrapping_add(c)) & mask
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&out, (lanes as u64) * 4, &expected);
        StageReport {
            name: format!("Mem chase ({} MB)", (len as u64 * 4) / (1024 * 1024)),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Combined core+memory soak: hammers ALU (core), a **bandwidth-saturating**
    /// VRAM stream (memory controller / DRAM throughput), and a pointer-chase
    /// (memory latency/addressing) **at the same time** for ~`target_ms` —
    /// loading the shared voltage rail / power / thermals like a real game, the
    /// condition that exposes instability a single-axis test misses. ALU and
    /// chase are known-answer checked (SilentError on any divergence); the
    /// bandwidth stream is load-only.
    pub fn run_combined(&self, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();

        // --- Fused core+memory (ALU + bandwidth in one kernel) ---
        // Each lane does `fused_iters` LCG rounds (core) and a memory RMW into a
        // large VRAM buffer each round (bandwidth), co-loading both per dispatch.
        let fused_n: u32 = 1 << 20; // 1,048,576 lanes
        let fused_iters: u32 = 512; // LCG + memory RMW per lane per dispatch
        let buf_bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let buf_n = (buf_bytes / 4) as u32;
        let fused_data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-fused-data"),
            contents: bytemuck::cast_slice(&(0..fused_n).collect::<Vec<u32>>()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let fused_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-fused-buf"), size: (buf_n as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false,
        });
        let fused_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-fused-p"),
            contents: bytemuck::bytes_of(&Quad { a: fused_iters, b: fused_n, c: buf_n, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let alu_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-fused"),
            source: wgpu::ShaderSource::Wgsl(FUSED_SHADER.into()),
        });
        let alu_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-fused"), layout: None, module: &alu_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let alu_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-fused"), layout: &alu_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: fused_data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fused_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fused_params.as_entire_binding() },
            ],
        });

        // --- Memory (pointer-chase) ---
        let clen: u32 = 1 << 24; // 64 MB chain
        let mask = clen - 1;
        let lanes: u32 = 65_536;
        let steps: u32 = 3_072;
        let chain = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-chain"), size: (clen as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false,
        });
        let cout = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-out"), size: (lanes as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let cfill_p = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-chain-fp"),
            contents: bytemuck::bytes_of(&Quad { a: clen, b: mask, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let cfill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-chain-fill"), source: wgpu::ShaderSource::Wgsl(CHAIN_FILL_SHADER.into()),
        });
        let cfill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-chain-fill"), layout: None, module: &cfill_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let cfill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-chain-fill"), layout: &cfill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cfill_p.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            { let mut cp = enc.begin_compute_pass(&Default::default());
              cp.set_pipeline(&cfill_pipe); cp.set_bind_group(0, &cfill_bind, &[]);
              cp.dispatch_workgroups(clen.div_ceil(64).min(65535), 1, 1); }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }
        let chase_p = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-chase-p"),
            contents: bytemuck::bytes_of(&Quad { a: steps, b: mask, c: lanes, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let chase_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-chase"), source: wgpu::ShaderSource::Wgsl(CHASE_SHADER.into()),
        });
        let chase_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-chase"), layout: None, module: &chase_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let chase_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-chase"), layout: &chase_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cout.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: chase_p.as_entire_binding() },
            ],
        });

        // Run the fused core+memory kernel (co-loads cores + DRAM) alongside the
        // pointer-chase (memory latency/addressing) each iteration.
        let alu_groups = fused_n.div_ceil(64);
        let chase_groups = lanes.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&alu_pipe);
                cp.set_bind_group(0, &alu_bind, &[]);
                cp.dispatch_workgroups(alu_groups, 1, 1);
                cp.set_pipeline(&chase_pipe);
                cp.set_bind_group(0, &chase_bind, &[]);
                cp.dispatch_workgroups(chase_groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // Verify fused ALU (lcg^(iters*k)) and chase (affine^steps).
        let (aa, ac) = lcg_pow((fused_iters as u64).wrapping_mul(k));
        let alu_exp: Vec<u32> = (0..fused_n).map(|i| aa.wrapping_mul(i).wrapping_add(ac)).collect();
        let (am, mm) = self.readback_compare(&fused_data, (fused_n as u64) * 4, &alu_exp);
        let (ca, cc) = affine_pow_mod(steps as u64, mask);
        let chase_exp: Vec<u32> = (0..lanes)
            .map(|l| {
                let s = (l.wrapping_mul(CHAIN_CP).wrapping_add(CHAIN_CQ)) & mask;
                (ca.wrapping_mul(s).wrapping_add(cc)) & mask
            })
            .collect();
        let (cm, mok) = self.readback_compare(&cout, (lanes as u64) * 4, &chase_exp);

        let mismatches = am.saturating_add(cm);
        StageReport {
            name: "Combined (core+mem)".into(),
            result: self.verdict(mismatches, mm && mok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Real graphics-pipeline stress with a frame-determinism check. Renders a
    /// heavy, deterministic instanced scene (overdraw + fragment ALU) to an
    /// offscreen target for ~`target_ms`, checksumming the framebuffer ~4×/s. A
    /// stable GPU yields an identical checksum every time; a divergence is a
    /// SilentError, a device-lost is a Crash. Closer to a game than any compute
    /// test (raster + ROP + fragment units), so it catches instability that
    /// compute-only validation passes. Returns the verdict plus the rendered
    /// frame count / FPS — the benchmark uses the FPS as its performance metric.
    /// v17.2 sentinel canary: a short TextureRop burst (the empirically BINDING silent-error
    /// detector on this silicon — every forge boundary failure fired here, never in ALU) with
    /// SELF-REFERENCING checksums: the first in-run checksum becomes the reference and later
    /// frames must match it bit-for-bit. Needs no stock golden (works under an applied profile,
    /// any driver): marginal silicon corrupts stochastically, so two identical renders diverge.
    /// `target_ms` must exceed ~600 ms so at least two 250 ms checksum windows land.
    pub fn run_canary_texture_selfcheck(&self, target_ms: u64) -> RenderResult {
        self.run_render_profile(
            target_ms,
            VfWorkload::TextureRop,
            None,
            false,
            true,
            None,
            None,
            None,
        )
    }

    pub fn run_render_stress(&self, target_ms: u64) -> RenderResult {
        self.run_render_profile(
            target_ms,
            VfWorkload::PowerRender,
            None,
            false,
            false,
            None,
            None,
            None,
        )
    }

    pub fn run_render_stress_with_cancel(
        &self,
        target_ms: u64,
        cancel: &AtomicBool,
    ) -> RenderResult {
        self.run_render_profile(
            target_ms,
            VfWorkload::PowerRender,
            None,
            false,
            false,
            None,
            None,
            Some(cancel),
        )
    }

    /// Returns `(checksum, avg_frame_us)` — the frame-time reference feeds the TextureStream and
    /// BoostEdge degradation gates.
    pub fn capture_one_golden(
        &self,
        profile: VfWorkload,
        sample_ms: u64,
    ) -> Result<(u32, u32), String> {
        const DIM: u32 = 1536;
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("golden-render-target"),
            size: wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());

        const SRC: u32 = 1024;
        let src_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("golden-render-src-tex"),
            size: wgpu::Extent3d { width: SRC, height: SRC, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut src_data = vec![0u8; (SRC as usize) * (SRC as usize) * 4];
        for (i, px) in src_data.chunks_exact_mut(4).enumerate() {
            let h = (i as u32).wrapping_mul(2654435761);
            px[0] = (h >> 24) as u8;
            px[1] = (h >> 16) as u8;
            px[2] = (h >> 8) as u8;
            px[3] = 255;
        }
        self.queue.write_texture(
            wgpu::ImageCopyTexture { texture: &src_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &src_data,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(SRC * 4), rows_per_image: Some(SRC) },
            wgpu::Extent3d { width: SRC, height: SRC, depth_or_array_layers: 1 },
        );
        let src_view = src_tex.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("golden-render-samp"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Must match the qualifier exactly: TextureStream goldens sample the same large source.
        let src_view = if profile == VfWorkload::TextureStream {
            self.create_texture_stream_source_view()
        } else {
            src_view
        };

        let (shader_source, instances, blend) = match profile {
            // BoostEntry / CompositeGameLoad slam the same 8-instance frame — share PowerRender's
            // golden (the composite's VRAM gather writes only to its sink, never the render image).
            VfWorkload::PowerRender
            | VfWorkload::BoostEntry
            | VfWorkload::CompositeGameLoad => {
                (RENDER_SHADER, 8, wgpu::BlendState::REPLACE)
            }
            VfWorkload::BoostEdge => (BOOST_EDGE_SHADER, 1, wgpu::BlendState::REPLACE),
            VfWorkload::TextureRop => (TEXTURE_ROP_SHADER, 4, wgpu::BlendState::ALPHA_BLENDING),
            // Must match the FrameCadence qualifier config exactly (same shader, 1 instance).
            VfWorkload::FrameCadence => (RENDER_SHADER, 1, wgpu::BlendState::REPLACE),
            VfWorkload::GeometryDepth => {
                (GEOMETRY_DEPTH_SHADER, GEOMETRY_DEPTH_INSTANCES, wgpu::BlendState::REPLACE)
            }
            VfWorkload::TextureStream => (TEXTURE_STREAM_SHADER, 2, wgpu::BlendState::REPLACE),
            VfWorkload::HeavySpike
            | VfWorkload::IdlePulse
            | VfWorkload::ComputeBurst
            | VfWorkload::MixedGame
            | VfWorkload::VramPressure => {
                return Err(format!("unsupported golden render profile: {profile:?}"));
            }
        };
        // Must match the GeometryDepth qualifier config exactly (same depth state + draw range).
        let geometry_depth = profile == VfWorkload::GeometryDepth;
        let vertex_count: u32 = if geometry_depth { GEOMETRY_DEPTH_TRIS * 3 } else { 3 };
        let depth_view = geometry_depth.then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("golden-render-depth"),
                    size: wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth24Plus,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("golden-render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("golden-render"), layout: None,
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs", buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: geometry_depth.then(|| wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
        });
        let tex_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("golden-render-tex"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let px_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("golden-render-px"),
            size: (DIM as u64) * (DIM as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sum_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("golden-render-sum"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let red_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("golden-render-rp"),
            contents: bytemuck::bytes_of(&Quad { a: DIM * DIM, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let red_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("golden-reduce3"),
            source: wgpu::ShaderSource::Wgsl(REDUCE3_SHADER.into()),
        });
        let red_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("golden-reduce3"),
            layout: None,
            module: &red_mod,
            entry_point: "main",
            compilation_options: Default::default(),
        });
        let red_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("golden-reduce3"),
            layout: &red_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: px_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sum_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: red_params.as_entire_binding() },
            ],
        });

        let start = std::time::Instant::now();
        let mut frames = 0u64;
        let mut golden = None;
        while (start.elapsed().as_millis() as u64) < sample_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("golden-render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: depth_view.as_ref().map(|view| {
                        wgpu::RenderPassDepthStencilAttachment {
                            view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &tex_bind, &[]);
                rp.draw(0..vertex_count, 0..instances);
            }
            enc.copy_texture_to_buffer(
                wgpu::ImageCopyTexture { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::ImageCopyBuffer { buffer: &px_buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(DIM * 4), rows_per_image: Some(DIM) } },
                wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
            );
            enc.clear_buffer(&sum_buf, 0, None);
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&red_pipe);
                cp.set_bind_group(0, &red_bind, &[]);
                cp.dispatch_workgroups(256, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
            if self.crashed.load(Ordering::SeqCst) {
                return Err("device lost while capturing stock golden".into());
            }
            let checksum = self.read_u32(&sum_buf);
            frames = frames.saturating_add(1);
            observe_golden_checksum(&mut golden, checksum, frames)?;
        }
        let checksum = finish_golden_capture(golden, frames)?;
        let avg_frame_us =
            u32::try_from((start.elapsed().as_micros() as u64) / frames.max(1)).unwrap_or(u32::MAX);
        Ok((checksum, avg_frame_us))
    }

    /// Failure-seeking F2 qualification loop. Discovery must keep calling
    /// [`Self::run_render_stress`]; this method is only for Standard/Long reset/reapply passes.
    pub fn run_vf_qualifier_stress(&self, target_ms: u64) -> RenderResult {
        let phase = AtomicU8::new(VfQualifierPhase::NONE_CODE);
        self.run_vf_qualifier_stress_with_phase_and_pattern(
            target_ms,
            &phase,
            VfQualifierPattern::Fsgl1,
        )
    }

    /// Same qualifier with an observable phase id for the service's concurrent NVML sampler.
    pub fn run_vf_qualifier_stress_with_phase(
        &self,
        target_ms: u64,
        phase_state: &AtomicU8,
    ) -> RenderResult {
        self.run_vf_qualifier_stress_with_phase_and_pattern(
            target_ms,
            phase_state,
            VfQualifierPattern::Fsgl1,
        )
    }

    pub fn run_vf_qualifier_stress_with_phase_and_pattern(
        &self,
        target_ms: u64,
        phase_state: &AtomicU8,
        pattern: VfQualifierPattern,
    ) -> RenderResult {
        self.run_vf_qualifier_stress_with_phase_pattern_and_goldens(
            target_ms,
            phase_state,
            pattern,
            None,
        )
    }

    pub fn run_vf_qualifier_stress_with_phase_pattern_and_goldens(
        &self,
        target_ms: u64,
        phase_state: &AtomicU8,
        pattern: VfQualifierPattern,
        goldens: Option<RenderGoldens>,
    ) -> RenderResult {
        self.run_vf_qualifier_stress_with_phase_pattern_goldens_and_cancel(
            target_ms,
            phase_state,
            pattern,
            goldens,
            None,
        )
    }

    pub fn run_vf_qualifier_stress_with_phase_pattern_goldens_and_cancel(
        &self,
        target_ms: u64,
        phase_state: &AtomicU8,
        pattern: VfQualifierPattern,
        goldens: Option<RenderGoldens>,
        cancel: Option<&AtomicBool>,
    ) -> RenderResult {
        let started = std::time::Instant::now();
        let mut frames = 0u64;
        let mut reports = Vec::new();
        let plan = vf_qualifier_plan(target_ms, pattern);

        for segment in plan {
            if cancel.is_some_and(|token| token.load(Ordering::SeqCst)) {
                break;
            }
            phase_state.store(segment.phase.code(), Ordering::SeqCst);
            let single = [segment.workload];
            let mixed = [
                VfWorkload::BoostEdge,
                VfWorkload::TextureRop,
                VfWorkload::PowerRender,
            ];
            let workloads: &[VfWorkload] =
                if segment.workload == VfWorkload::MixedGame { &mixed } else { &single };
            let workload_ms = segment.duration_ms / workloads.len() as u64;
            let mut assigned = 0u64;

            for (index, &workload) in workloads.iter().enumerate() {
                let duration_ms = if index + 1 == workloads.len() {
                    segment.duration_ms.saturating_sub(assigned)
                } else {
                    workload_ms
                };
                assigned = assigned.saturating_add(duration_ms);
                let result = match workload {
                    VfWorkload::ComputeBurst => {
                        let stage = self.run_alu_with_cancel(
                            "VF qualifier compute burst",
                            262_144,
                            256,
                            duration_ms,
                            cancel,
                        );
                        RenderResult {
                            result: stage.result,
                            frames: 0,
                            fps: 0.0,
                            failure_phase: (stage.result != StabilityResult::Stable)
                                .then_some(segment.phase),
                            phase_reports: vec![VfPhaseReport {
                                phase: segment.phase,
                                result: stage.result,
                                frames: 0,
                                checksum_count: 1,
                                elapsed_ms: stage.elapsed_ms,
                            }],
                        }
                    }
                    VfWorkload::VramPressure => {
                        let stage = self.run_vram_pressure_with_cancel(duration_ms, cancel);
                        RenderResult {
                            result: stage.result,
                            frames: 0,
                            fps: 0.0,
                            failure_phase: (stage.result != StabilityResult::Stable)
                                .then_some(segment.phase),
                            phase_reports: vec![VfPhaseReport {
                                phase: segment.phase,
                                result: stage.result,
                                frames: 0,
                                checksum_count: 1,
                                elapsed_ms: stage.elapsed_ms,
                            }],
                        }
                    }
                    other => self.run_render_profile(
                        duration_ms,
                        other,
                        Some(segment.phase),
                        matches!(other, VfWorkload::IdlePulse),
                        true,
                        goldens.and_then(|g| golden_for_workload(g, other)),
                        match other {
                            VfWorkload::TextureStream => goldens
                                .map(|g| u64::from(g.stream_frame_reference_ms) * 1000),
                            VfWorkload::BoostEdge => {
                                goldens.map(|g| u64::from(g.boost_frame_reference_us))
                            }
                            _ => None,
                        },
                        cancel,
                    ),
                };
                frames = frames.saturating_add(result.frames);
                reports.extend(result.phase_reports);
                if result.result != StabilityResult::Stable {
                    phase_state.store(VfQualifierPhase::NONE_CODE, Ordering::SeqCst);
                    let secs = started.elapsed().as_secs_f64().max(0.001);
                    return RenderResult {
                        result: result.result,
                        frames,
                        fps: frames as f64 / secs,
                        failure_phase: result.failure_phase.or(Some(segment.phase)),
                        phase_reports: reports,
                    };
                }
            }
        }

        phase_state.store(VfQualifierPhase::NONE_CODE, Ordering::SeqCst);
        let secs = started.elapsed().as_secs_f64().max(0.001);
        RenderResult {
            result: StabilityResult::Stable,
            frames,
            fps: frames as f64 / secs,
            failure_phase: None,
            phase_reports: reports,
        }
    }

    /// Create + deterministically fill the large VRAM-resident TextureRop source (see
    /// [`TEXTURE_STREAM_DIM`]). Used by BOTH the qualifier and golden capture — content and size
    /// must be identical in both or the golden would diverge for a healthy GPU.
    fn create_texture_stream_source_view(&self) -> wgpu::TextureView {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture-stream-src"),
            size: wgpu::Extent3d {
                width: TEXTURE_STREAM_DIM,
                height: TEXTURE_STREAM_DIM,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("texture-stream-fill"),
            source: wgpu::ShaderSource::Wgsl(TEXTURE_STREAM_FILL_SHADER.into()),
        });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("texture-stream-fill"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs",
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("texture-stream-fill"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&pipeline);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        view
    }

    #[allow(clippy::too_many_arguments)]
    fn run_render_profile(
        &self,
        target_ms: u64,
        profile: VfWorkload,
        phase: Option<VfQualifierPhase>,
        idle_pulses: bool,
        full_workload_duration: bool,
        golden: Option<u32>,
        frame_reference_us: Option<u64>,
        cancel: Option<&AtomicBool>,
    ) -> RenderResult {
        let start = std::time::Instant::now();
        let mut frames: u64 = 0;
        const DIM: u32 = 1536; // 1536*4 = 6144 B/row (256-aligned for copy)
        // Overdraw factor. 8 full-screen triangles at 1536² already swamp the GPU's
        // parallel fragment capacity (18.9M frags ≫ thousands of ALUs) so it stays
        // 100% occupied → full game power (~199W). 128 instances did NOT raise power
        // (already saturated) but made a SINGLE frame's work so large (~1.5-2s) that
        // it grazed the ~2s TDR watchdog — fine at boost clock, but once the power
        // cap throttled the clock a frame crossed 2s and the driver reset (device
        // lost). Keeping per-frame work bounded (many short frames, not one giant
        // one — like a real game) is what makes the load safely repeatable.
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-target"),
            size: wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());

        // Source texture the fragment shader samples heavily (TMU + memory load,
        // like a game's texturing). Filled with a deterministic pattern.
        const SRC: u32 = 1024;
        let src_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-src-tex"),
            size: wgpu::Extent3d { width: SRC, height: SRC, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut src_data = vec![0u8; (SRC as usize) * (SRC as usize) * 4];
        for (i, px) in src_data.chunks_exact_mut(4).enumerate() {
            let h = (i as u32).wrapping_mul(2654435761);
            px[0] = (h >> 24) as u8;
            px[1] = (h >> 16) as u8;
            px[2] = (h >> 8) as u8;
            px[3] = 255;
        }
        self.queue.write_texture(
            wgpu::ImageCopyTexture { texture: &src_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &src_data,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(SRC * 4), rows_per_image: Some(SRC) },
            wgpu::Extent3d { width: SRC, height: SRC, depth_or_array_layers: 1 },
        );
        let src_view = src_tex.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("render-samp"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // TextureStream samples the LARGE VRAM-resident source (cache-defeating; see
        // TEXTURE_STREAM_DIM) instead of the small L2-resident one.
        let src_view = if profile == VfWorkload::TextureStream {
            self.create_texture_stream_source_view()
        } else {
            src_view
        };

        let (shader_source, instances, blend) = match profile {
            VfWorkload::PowerRender
            | VfWorkload::HeavySpike
            | VfWorkload::IdlePulse
            | VfWorkload::BoostEntry
            | VfWorkload::CompositeGameLoad => {
                (RENDER_SHADER, 8, wgpu::BlendState::REPLACE)
            }
            VfWorkload::BoostEdge => (BOOST_EDGE_SHADER, 1, wgpu::BlendState::REPLACE),
            VfWorkload::TextureRop => (TEXTURE_ROP_SHADER, 4, wgpu::BlendState::ALPHA_BLENDING),
            // One instance keeps a single frame at game-frame scale (~10-20 ms of heavy work)
            // so the burst/idle cycle below runs at real frame cadence.
            VfWorkload::FrameCadence => (RENDER_SHADER, 1, wgpu::BlendState::REPLACE),
            VfWorkload::GeometryDepth => {
                (GEOMETRY_DEPTH_SHADER, GEOMETRY_DEPTH_INSTANCES, wgpu::BlendState::REPLACE)
            }
            VfWorkload::TextureStream => (TEXTURE_STREAM_SHADER, 2, wgpu::BlendState::REPLACE),
            VfWorkload::ComputeBurst | VfWorkload::MixedGame | VfWorkload::VramPressure => {
                unreachable!("non-render workload")
            }
        };
        // GeometryDepth draws many small procedural triangles under a depth test; everything else
        // draws overdraw fullscreen triangles with no depth attachment.
        let geometry_depth = profile == VfWorkload::GeometryDepth;
        let vertex_count: u32 = if geometry_depth { GEOMETRY_DEPTH_TRIS * 3 } else { 3 };
        let depth_view = geometry_depth.then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("render-depth"),
                    size: wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth24Plus,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render"), source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render"), layout: None,
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs", buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: geometry_depth.then(|| wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
        });
        let tex_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render-tex"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        // Readback: copy the frame to a buffer, reduce to one checksum u32.
        let px_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render-px"), size: (DIM as u64) * (DIM as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let sum_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render-sum"), size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let n = DIM * DIM;
        let red_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render-rp"), contents: bytemuck::bytes_of(&Quad { a: n, b: 0, c: 0, d: 0 }), usage: wgpu::BufferUsages::UNIFORM,
        });
        let red_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reduce"),
            source: wgpu::ShaderSource::Wgsl(
                if golden.is_some() { REDUCE3_SHADER } else { REDUCE_SHADER }.into(),
            ),
        });
        let red_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reduce"), layout: None, module: &red_mod, entry_point: "main", compilation_options: Default::default(),
        });
        let red_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reduce"), layout: &red_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: px_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sum_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: red_params.as_entire_binding() },
            ],
        });
        let golden_mismatch_buf = golden.map(|_| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render-golden-mismatch"),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let compare_params = golden.map(|g| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("render-golden-params"),
                contents: bytemuck::bytes_of(&Quad { a: g, b: 0, c: 0, d: 0 }),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        let compare_mod = golden.map(|_| {
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("checksum-compare"),
                source: wgpu::ShaderSource::Wgsl(CHECKSUM_COMPARE_SHADER.into()),
            })
        });
        let compare_pipe = compare_mod.as_ref().map(|module| {
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("checksum-compare"),
                layout: None,
                module,
                entry_point: "main",
                compilation_options: Default::default(),
            })
        });
        let compare_bind = match (&compare_pipe, &golden_mismatch_buf, &compare_params) {
            (Some(pipe), Some(mismatch), Some(params)) => Some(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("checksum-compare"),
                    layout: &pipe.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: sum_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: mismatch.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    ],
                }),
            ),
            _ => None,
        };

        let mut reference = None;
        let mut diverged = false;
        let mut checksum_count = 0u32;
        let mut last_check = std::time::Instant::now();
        let workload_start =
            if full_workload_duration { std::time::Instant::now() } else { start };
        let mut last_idle = workload_start;
        let golden_mode = golden.is_some();
        // FrameCadence paces itself per frame (sync + gap after every submit below), so the
        // coarser droop-burst / idle-pulse pacing must not also fire.
        let frame_cadence = profile == VfWorkload::FrameCadence;
        // BoostEntry paces itself too: heavy slam (timed) → true-idle seconds → slam again.
        let boost_entry = profile == VfWorkload::BoostEntry;
        // BoostEdge (v16.3): drain every frame + sub-ms CPU-build bubble → discrete boost edges at
        // the anchor bin (the high-FPS/lobby regime). Times each frame in µs for the degradation gate.
        let boost_edge = profile == VfWorkload::BoostEdge;
        // TextureStream renders in scissor bands, one submit each (preemptible + pre-hang
        // detectable). A stalled band or sustained frame-time collapse fails the dwell as
        // Unstable BEFORE the driver TDR watchdog can fire.
        let stream_banded = profile == VfWorkload::TextureStream;
        let mut stalled = false;
        let mut stream_frame_ms_total: u64 = 0;
        // Sum of per-frame drain times (µs) for the BoostEdge degradation gate.
        let mut boost_frame_us_total: u64 = 0;

        // v16.1 composite: allocate a near-full VRAM-resident pool (OOM-guarded — fills whatever
        // remains after the render's own buffers, degrading on smaller cards) + a scattered-gather
        // pipeline. Each frame issues a gather over one pool table in the SAME submit as the heavy
        // render, so texture hops and DRAM/controller pressure load the shared core rail together.
        const COMPOSITE_MAX_TABLES: usize = 48; // up to ~12 GB attempted; OOM-guard keeps what fits
        const COMPOSITE_LANES: u32 = 262_144;
        const COMPOSITE_GATHERS: u32 = 96;
        let composite_table_len = {
            let bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
            (bytes / 4) as u32
        };
        let composite_pool: Vec<wgpu::Buffer> = if profile == VfWorkload::CompositeGameLoad {
            let mut pool = Vec::new();
            for _ in 0..COMPOSITE_MAX_TABLES {
                self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let table = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("composite-vram-table"),
                    size: (composite_table_len as u64) * 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
                if pollster::block_on(self.device.pop_error_scope()).is_some() {
                    break;
                }
                pool.push(table);
            }
            pool
        } else {
            Vec::new()
        };
        let composite_gather = (!composite_pool.is_empty()).then(|| {
            let sink = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("composite-sink"),
                size: 256 * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite-gather-params"),
                contents: bytemuck::bytes_of(&Quad {
                    a: composite_table_len,
                    b: COMPOSITE_GATHERS,
                    c: 0x9e37_79b9,
                    d: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("composite-gather"),
                source: wgpu::ShaderSource::Wgsl(COMPOSITE_GATHER_SHADER.into()),
            });
            let pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("composite-gather"),
                layout: None,
                module: &module,
                entry_point: "main",
                compilation_options: Default::default(),
            });
            // wgpu keeps bound resources alive via the bind group, so the local sink/params handles
            // may drop; the pool tables are held by `composite_pool` for the whole workload.
            let binds: Vec<wgpu::BindGroup> = composite_pool
                .iter()
                .map(|table| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("composite-gather"),
                        layout: &pipe.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: table.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: sink.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                        ],
                    })
                })
                .collect();
            (pipe, binds)
        });

        while (workload_start.elapsed().as_millis() as u64) < target_ms
            && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
        {
            if frame_cadence {
                // paced after submit
            } else if golden_mode && frames > 0 && frames.is_multiple_of(DROOP_BURST) {
                self.device.poll(wgpu::Maintain::Wait);
                std::thread::sleep(std::time::Duration::from_millis(DROOP_GAP_MS));
            } else if !golden_mode && idle_pulses && last_idle.elapsed().as_millis() >= 750 {
                self.device.poll(wgpu::Maintain::Wait);
                std::thread::sleep(std::time::Duration::from_millis(100));
                last_idle = std::time::Instant::now();
            }
            // BoostEntry: time the whole slam (encode + submit + GPU) from here; read below.
            let burst_start = std::time::Instant::now();
            if stream_banded {
                let frame_start = std::time::Instant::now();
                let band_h = DIM / STREAM_BANDS;
                for band in 0..STREAM_BANDS {
                    let band_start = std::time::Instant::now();
                    let mut benc = self.device.create_command_encoder(&Default::default());
                    {
                        let mut rp = benc.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("render-stream-band"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: if band == 0 {
                                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                                    } else {
                                        wgpu::LoadOp::Load
                                    },
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });
                        rp.set_pipeline(&pipeline);
                        rp.set_bind_group(0, &tex_bind, &[]);
                        rp.set_scissor_rect(0, band * band_h, DIM, band_h);
                        rp.draw(0..vertex_count, 0..instances);
                    }
                    self.queue.submit(Some(benc.finish()));
                    self.device.poll(wgpu::Maintain::Wait);
                    if (band_start.elapsed().as_millis() as u64) > STREAM_PREHANG_BAND_MS {
                        stalled = true;
                    }
                    if stalled
                        || self.crashed.load(Ordering::SeqCst)
                        || cancel.is_some_and(|token| token.load(Ordering::SeqCst))
                    {
                        break;
                    }
                }
                stream_frame_ms_total =
                    stream_frame_ms_total.saturating_add(frame_start.elapsed().as_millis() as u64);
                if stalled || self.crashed.load(Ordering::SeqCst) {
                    // Partial frame — never checksum it (a stall is not a wrong result).
                    break;
                }
            }
            let mut enc = self.device.create_command_encoder(&Default::default());
            if !stream_banded {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: depth_view.as_ref().map(|view| {
                        wgpu::RenderPassDepthStencilAttachment {
                            view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }
                    }),
                    timestamp_writes: None, occlusion_query_set: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &tex_bind, &[]);
                rp.draw(0..vertex_count, 0..instances);
            }
            if golden_mode {
                enc.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::ImageCopyBuffer { buffer: &px_buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(DIM * 4), rows_per_image: Some(DIM) } },
                    wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
                );
                enc.clear_buffer(&sum_buf, 0, None);
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(&red_pipe);
                    cp.set_bind_group(0, &red_bind, &[]);
                    cp.dispatch_workgroups(256, 1, 1);
                }
                if let (Some(pipe), Some(bind)) = (&compare_pipe, &compare_bind) {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(pipe);
                    cp.set_bind_group(0, bind, &[]);
                    cp.dispatch_workgroups(1, 1, 1);
                }
                checksum_count = checksum_count.saturating_add(1);
            }
            // v16.1: the VRAM-resident gather rides in the SAME submit as the render frame — the
            // heavy texture render and the DRAM/memory-controller pressure hit the shared core rail
            // together (highest combined draw in the soak).
            if let Some((gather_pipe, gather_binds)) = &composite_gather {
                let bind = &gather_binds[(frames as usize) % gather_binds.len()];
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(gather_pipe);
                cp.set_bind_group(0, bind, &[]);
                cp.dispatch_workgroups(COMPOSITE_LANES / 64, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            frames += 1;

            // Throttle the queue: bound in-flight frames so we don't flood the
            // submission queue. Without this, a tight submit loop floods the driver
            // — the first dwell survives but leaves it stressed, and the SECOND call
            // TDRs (device lost, unrecoverable). The heavy compute load (run_power_
            // load) polls every 8 submits for the same reason; render frames are far
            // heavier, so bound tighter (every 3). Keeps the GPU saturated (~199 W,
            // game power) while staying safely repeatable across a full sweep.
            if boost_entry {
                // v15: finish the slam frame and TIME it. A post-idle slam that stalls toward the
                // ~2 s driver watchdog is the pre-hang precursor of the in-game BusReset TDR
                // cascade — fail Unstable here, never let the cascade start.
                self.device.poll(wgpu::Maintain::Wait);
                if (burst_start.elapsed().as_millis() as u64) > BOOST_ENTRY_STALL_MS {
                    stalled = true;
                    break;
                }
                // TRUE idle, seconds — long enough for the driver to leave the high P-state, so
                // the NEXT slam re-enters through the full boost VF ramp (the launch transition).
                // Sliced so cooperative Stop and crash detection stay responsive throughout.
                let gap_ms = BOOST_ENTRY_GAPS_MS
                    [(frames % BOOST_ENTRY_GAPS_MS.len() as u64) as usize];
                let idle_start = std::time::Instant::now();
                while (idle_start.elapsed().as_millis() as u64) < gap_ms
                    && !self.crashed.load(Ordering::SeqCst)
                    && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
                {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            } else if frame_cadence {
                // Each frame IS the heavy burst: finish it, then idle a few ms like a
                // present/vsync gap. The heavy→idle→heavy edge at frame period is the VRM
                // droop-release transient that kills undervolts in real games.
                self.device.poll(wgpu::Maintain::Wait);
                let gap_ms = FRAME_CADENCE_GAPS_MS
                    [(frames % FRAME_CADENCE_GAPS_MS.len() as u64) as usize];
                std::thread::sleep(std::time::Duration::from_millis(gap_ms));
            } else if boost_edge {
                // v16.3 lobby cadence: drain THIS light frame fully (a CPU/engine-bound game loop
                // finishes and waits), TIME it for the degradation gate, then spin a sub-ms
                // CPU-frame-build bubble so the next submit re-ramps the boost VF from idle. Each
                // frame is a discrete drain→idle→ramp current edge AT the anchor bin — the field
                // high-FPS killer a saturated submission queue cannot reproduce. Spin, not sleep:
                // Windows thread sleep is ms-coarse and would swamp the sub-ms cadence.
                self.device.poll(wgpu::Maintain::Wait);
                boost_frame_us_total =
                    boost_frame_us_total.saturating_add(burst_start.elapsed().as_micros() as u64);
                let bubble_us =
                    BOOST_EDGE_BUBBLE_US[(frames % BOOST_EDGE_BUBBLE_US.len() as u64) as usize];
                if bubble_us > 0 {
                    let spin_start = std::time::Instant::now();
                    while (spin_start.elapsed().as_micros() as u64) < bubble_us
                        && !self.crashed.load(Ordering::SeqCst)
                        && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
                    {
                        std::hint::spin_loop();
                    }
                }
            } else if frames % 3 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }

            if self.crashed.load(Ordering::SeqCst) {
                break;
            }

            if golden_mode {
                if last_check.elapsed().as_millis() >= 250 {
                    last_check = std::time::Instant::now();
                    self.device.poll(wgpu::Maintain::Wait);
                    if golden_mismatch_buf
                        .as_ref()
                        .is_some_and(|buf| self.read_u32(buf) > 0)
                    {
                        diverged = true;
                        break;
                    }
                }
            } else if last_check.elapsed().as_millis() >= 250 {
                last_check = std::time::Instant::now();
                let mut enc = self.device.create_command_encoder(&Default::default());
                enc.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::ImageCopyBuffer { buffer: &px_buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(DIM * 4), rows_per_image: Some(DIM) } },
                    wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
                );
                enc.clear_buffer(&sum_buf, 0, None);
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(&red_pipe); cp.set_bind_group(0, &red_bind, &[]);
                    cp.dispatch_workgroups(256, 1, 1);
                }
                self.queue.submit(Some(enc.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                let sum = self.read_u32(&sum_buf);
                checksum_count = checksum_count.saturating_add(1);
                match reference {
                    None => reference = Some(sum),
                    Some(r) => {
                        if sum != r {
                            diverged = true;
                            break;
                        }
                    }
                }
            }
            self.device.poll(wgpu::Maintain::Wait);
        }
        if golden_mode
            && !diverged
            && !self.crashed.load(Ordering::SeqCst)
            && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
        {
            self.device.poll(wgpu::Maintain::Wait);
            if golden_mismatch_buf
                .as_ref()
                .is_some_and(|buf| self.read_u32(buf) > 0)
            {
                diverged = true;
            }
        }

        // Marginal-silicon gate: sustained frame time far beyond the stock reference means internal
        // retries/slowdown — reject before it becomes a hang (see [`frame_time_degraded`]).
        let degraded = frame_time_degraded(
            stream_banded,
            boost_edge,
            stalled,
            frames,
            stream_frame_ms_total,
            boost_frame_us_total,
            frame_reference_us,
        );
        let mut result = render_integrity_result(self.crashed.load(Ordering::SeqCst), diverged);
        if result == StabilityResult::Stable
            && (stalled || degraded)
            && !cancel.is_some_and(|token| token.load(Ordering::SeqCst))
        {
            // Not a wrong result (no divergence) and not a crash — the bin is behaviourally
            // unstable: a band stalled toward the TDR watchdog or throughput collapsed.
            result = StabilityResult::Unstable;
        }
        let secs = start.elapsed().as_secs_f64().max(0.001);
        let phase_reports = phase
            .map(|phase| {
                vec![VfPhaseReport {
                    phase,
                    result,
                    frames,
                    checksum_count,
                    elapsed_ms: workload_start.elapsed().as_millis() as u64,
                }]
            })
            .unwrap_or_default();
        RenderResult {
            result,
            frames,
            fps: frames as f64 / secs,
            failure_phase: (result != StabilityResult::Stable).then_some(phase).flatten(),
            phase_reports,
        }
    }

    /// Measure sustained memory bandwidth (GB/s) over ~`target_ms` by streaming
    /// a large VRAM buffer (read+write each element). Used to find the GDDR6
    /// *effective-bandwidth peak* — past it, ECC correction eats the gains.
    pub fn measure_bandwidth_gbps(&self, target_ms: u64) -> f64 {
        self.measure_bandwidth_stats(target_ms).0
    }

    /// Bandwidth over sub-windows → `(peak, min)` GB/s. A large gap between peak
    /// and min means the clock is *inconsistent* (GDDR6 CRC retries / throttle
    /// dips) — the real signal for the intermittent stutters a peak-only number
    /// hides.
    pub fn measure_bandwidth_stats(&self, target_ms: u64) -> (f64, f64) {
        let bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let len = (bytes / 4) as u32;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bw-buf"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bw-params"),
            contents: bytemuck::bytes_of(&Quad { a: len, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bw"),
            source: wgpu::ShaderSource::Wgsl(BW_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bw"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bw"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });
        let groups = len.div_ceil(64).min(65535);

        // Warm-up pass (not timed).
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        // Measure several sub-windows and take the PEAK: the memory clock boosts
        // and throttles dynamically, so a single short window is noisy. The peak
        // reflects the bandwidth the clock can actually deliver (downward noise =
        // throttle dips, which we reject); it can't exceed real bandwidth.
        let window_ms: u64 = 500;
        let windows = (target_ms / window_ms).max(8);
        let mut peak_gbps = 0.0f64;
        let mut min_gbps = f64::INFINITY;
        for _ in 0..windows {
            let ws = std::time::Instant::now();
            let mut passes: u64 = 0;
            while (ws.elapsed().as_millis() as u64) < window_ms {
                let mut enc = self.device.create_command_encoder(&Default::default());
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(&pipeline);
                    cp.set_bind_group(0, &bind, &[]);
                    cp.dispatch_workgroups(groups, 1, 1);
                }
                self.queue.submit(Some(enc.finish()));
                passes += 1;
                if passes % 8 == 0 {
                    self.device.poll(wgpu::Maintain::Wait);
                }
                if self.crashed.load(Ordering::SeqCst) {
                    return (0.0, 0.0);
                }
            }
            self.device.poll(wgpu::Maintain::Wait);
            let secs = ws.elapsed().as_secs_f64();
            if secs > 0.0 {
                // 8 bytes moved per element per pass (read + write).
                let gbps = passes as f64 * len as f64 * 8.0 / secs / 1e9;
                if gbps > peak_gbps {
                    peak_gbps = gbps;
                }
                if gbps < min_gbps {
                    min_gbps = gbps;
                }
            }
        }
        if !min_gbps.is_finite() {
            min_gbps = peak_gbps;
        }
        (peak_gbps, min_gbps)
    }

    /// Read a single u32 from a COPY_SRC buffer.
    fn read_u32(&self, buffer: &wgpu::Buffer) -> u32 {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u32-staging"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, 4);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        if matches!(rx.recv(), Ok(Ok(()))) {
            let data = slice.get_mapped_range();
            u32::from_ne_bytes([data[0], data[1], data[2], data[3]])
        } else {
            0
        }
    }

    fn readback_compare(&self, buffer: &wgpu::Buffer, byte_size: u64, expected: &[u32]) -> (u32, bool) {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: byte_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, byte_size);
        self.queue.submit(Some(enc.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mapped_ok = matches!(rx.recv(), Ok(Ok(())));
        if !mapped_ok {
            return (0, false);
        }
        let data = slice.get_mapped_range();
        let output: &[u32] = bytemuck::cast_slice(&data);
        let mut mismatches = 0u32;
        for (got, exp) in output.iter().zip(expected.iter()) {
            if got != exp {
                mismatches += 1;
            }
        }
        (mismatches, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_reference_is_deterministic() {
        assert_eq!(lcg(0, 1), C2);
        assert_eq!(lcg(123, 1000), lcg(123, 1000));
    }

    #[test]
    fn boost_edge_bubbles_cycle_and_stay_sub_millisecond() {
        // The lobby cadence relies on varied per-frame bubbles; a degenerate all-zero or
        // millisecond-scale table would collapse the drain→idle→ramp edge it exists to create.
        assert!(BOOST_EDGE_BUBBLE_US.iter().any(|&us| us > 0), "some frames must idle");
        assert!(BOOST_EDGE_BUBBLE_US.iter().all(|&us| us < 1000), "bubbles must stay sub-ms");
    }

    #[test]
    fn frame_time_degraded_flags_marginal_slowdown_per_regime() {
        // No reference / too few frames / neither regime ⇒ never degraded.
        assert!(!frame_time_degraded(false, false, false, 100, 0, 0, None));
        assert!(!frame_time_degraded(true, false, false, 3, 999_999, 0, Some(20_000)));
        assert!(!frame_time_degraded(false, false, false, 100, 999_999, 999_999, Some(10)));

        // BoostEdge: healthy dwell frames (faster than reference) pass; a 2×+ slowdown fails.
        // reference 400 µs, factor 2 ⇒ threshold 800 µs avg.
        assert!(!frame_time_degraded(false, true, false, 1000, 0, 500 * 1000, Some(400)));
        assert!(frame_time_degraded(false, true, false, 1000, 0, 900 * 1000, Some(400)));

        // TextureStream: ms totals scale to µs. reference 20 000 µs, factor 2 ⇒ 40 ms avg threshold.
        assert!(!frame_time_degraded(true, false, false, 100, 30 * 100, 0, Some(20_000)));
        assert!(frame_time_degraded(true, false, false, 100, 50 * 100, 0, Some(20_000)));
        // A hard stall already fired ⇒ the stream degradation branch defers (stall owns the verdict).
        assert!(!frame_time_degraded(true, false, true, 100, 50 * 100, 0, Some(20_000)));
    }

    #[test]
    fn lcg_jump_matches_loop() {
        for &(seed, n) in &[(0u32, 0u64), (0, 1), (123, 7), (999, 1000), (42, 65536)] {
            let mut x = seed;
            for _ in 0..n {
                x = x.wrapping_mul(C1).wrapping_add(C2);
            }
            assert_eq!(lcg_jump(seed, n), x, "seed={seed} n={n}");
        }
    }

    #[test]
    fn vf_qualifier_plan_preserves_duration_and_crosses_load_amplitudes() {
        let plan = vf_qualifier_plan(60_000, VfQualifierPattern::Fsgl1);
        assert_eq!(plan.iter().map(|segment| segment.duration_ms).sum::<u64>(), 60_000);
        assert_eq!(plan.first().map(|segment| segment.phase), Some(VfQualifierPhase::PowerOpening));
        assert_eq!(plan.last().map(|segment| segment.phase), Some(VfQualifierPhase::PowerClosing));
        assert!(plan.iter().any(|segment| segment.workload == VfWorkload::BoostEdge));
        assert!(plan.iter().any(|segment| segment.workload == VfWorkload::TextureRop));
        assert!(plan.iter().any(|segment| segment.workload == VfWorkload::ComputeBurst));
        assert!(plan.iter().any(|segment| segment.workload == VfWorkload::IdlePulse));
        assert!(plan.iter().all(|segment| segment.duration_ms > 0));
        assert_eq!(VfQualifierPhase::from_code(VfQualifierPhase::TextureRop.code()),
                   Some(VfQualifierPhase::TextureRop));
    }

    #[test]
    fn fsgl2_patterns_preserve_duration_and_differ_in_order() {
        let a = vf_qualifier_plan(60_000, VfQualifierPattern::Fsgl2A);
        let b = vf_qualifier_plan(60_000, VfQualifierPattern::Fsgl2B);
        assert_eq!(a.iter().map(|segment| segment.duration_ms).sum::<u64>(), 60_000);
        assert_eq!(b.iter().map(|segment| segment.duration_ms).sum::<u64>(), 60_000);
        let a_order: Vec<_> = a.iter().map(|segment| segment.phase).collect();
        let b_order: Vec<_> = b.iter().map(|segment| segment.phase).collect();
        assert_ne!(a_order, b_order);
        assert!(a.iter().any(|segment| segment.workload == VfWorkload::HeavySpike));
        assert!(b.iter().any(|segment| segment.workload == VfWorkload::TextureRop));
        assert_eq!(VfQualifierPattern::Fsgl2A.label(), "fsgl2-a");
        assert_eq!(VfQualifierPattern::Fsgl2B.label(), "fsgl2-b");
    }

    #[test]
    fn fsgl3_patterns_preserve_duration_bias_texrop_and_differ_in_order() {
        let a = vf_qualifier_plan(60_000, VfQualifierPattern::Fsgl3A);
        let b = vf_qualifier_plan(60_000, VfQualifierPattern::Fsgl3B);
        assert_eq!(a.iter().map(|segment| segment.duration_ms).sum::<u64>(), 60_000);
        assert_eq!(b.iter().map(|segment| segment.duration_ms).sum::<u64>(), 60_000);
        assert_eq!(a.first().map(|segment| segment.phase), Some(VfQualifierPhase::PowerOpening));
        assert_eq!(a.last().map(|segment| segment.phase), Some(VfQualifierPhase::PowerClosing));
        assert_eq!(b.first().map(|segment| segment.phase), Some(VfQualifierPhase::PowerOpening));
        assert_eq!(b.last().map(|segment| segment.phase), Some(VfQualifierPhase::PowerClosing));
        let a_order: Vec<_> = a.iter().map(|segment| segment.phase).collect();
        let b_order: Vec<_> = b.iter().map(|segment| segment.phase).collect();
        assert_ne!(a_order, b_order);
        let texrop_ms = |plan: &[VfQualifierSegment]| {
            plan.iter()
                .filter(|segment| segment.workload == VfWorkload::TextureRop)
                .map(|segment| segment.duration_ms)
                .sum::<u64>()
        };
        assert!(texrop_ms(&a) > 15_000);
        assert!(texrop_ms(&b) > 15_000);
        assert!(a.iter().any(|segment| segment.workload == VfWorkload::MixedGame));
        assert!(b.iter().any(|segment| segment.workload == VfWorkload::MixedGame));
        assert_eq!(VfQualifierPattern::Fsgl3A.label(), "fsgl3-a");
        assert_eq!(VfQualifierPattern::Fsgl3B.label(), "fsgl3-b");
    }

    #[test]
    fn v8_patterns_preserve_duration_and_bias_distinct_failure_modes() {
        let high_fps = vf_qualifier_plan(60_000, VfQualifierPattern::V8HighFps);
        let texture = vf_qualifier_plan(60_000, VfQualifierPattern::V8Texture);
        let transitions = vf_qualifier_plan(60_000, VfQualifierPattern::V8Transitions);
        let memory = vf_qualifier_plan(60_000, VfQualifierPattern::V8Memory);
        let required_phases = [
            VfQualifierPhase::PowerOpening,
            VfQualifierPhase::BoostEdge,
            VfQualifierPhase::HeavySpike,
            VfQualifierPhase::TextureRop,
            VfQualifierPhase::ComputeBurst,
            VfQualifierPhase::IdlePulse,
            VfQualifierPhase::MixedGame,
            VfQualifierPhase::PowerClosing,
            VfQualifierPhase::FrameCadence,
        ];
        for plan in [&high_fps, &texture, &transitions, &memory] {
            assert_eq!(
                plan.iter().map(|segment| segment.duration_ms).sum::<u64>(),
                60_000
            );
            assert_eq!(
                plan.first().map(|segment| segment.phase),
                Some(VfQualifierPhase::PowerOpening)
            );
            assert_eq!(
                plan.last().map(|segment| segment.phase),
                Some(VfQualifierPhase::PowerClosing)
            );
            for required in required_phases {
                assert!(plan.iter().any(|segment| segment.phase == required));
            }
        }
        // Memory is VRAM-dominant; the other three each carry exactly one unit-specific extra.
        assert!(
            duration_for_workload(&memory, VfWorkload::VramPressure) > 25_000,
            "V8Memory must be VRAM-pressure dominant"
        );
        assert!(high_fps.iter().any(|s| s.workload == VfWorkload::GeometryDepth));
        assert!(texture.iter().any(|s| s.workload == VfWorkload::VramPressure));
        assert!(transitions.iter().any(|s| s.workload == VfWorkload::GeometryDepth));
        let duration_for = |plan: &[VfQualifierSegment], workload| {
            plan.iter()
                .filter(|segment| segment.workload == workload)
                .map(|segment| segment.duration_ms)
                .sum::<u64>()
        };
        assert!(
            duration_for(&high_fps, VfWorkload::BoostEdge)
                > duration_for(&texture, VfWorkload::BoostEdge)
        );
        assert!(
            duration_for(&texture, VfWorkload::TextureRop)
                > duration_for(&high_fps, VfWorkload::TextureRop)
        );
        assert!(
            transitions
                .iter()
                .filter(|segment| segment.workload == VfWorkload::IdlePulse)
                .count()
                >= 5
        );
        assert_eq!(VfQualifierPattern::V8HighFps.label(), "v8-high-fps");
        assert_eq!(VfQualifierPattern::V8Texture.label(), "v8-texture");
        assert_eq!(VfQualifierPattern::V8Transitions.label(), "v8-transitions");
        assert_eq!(VfQualifierPattern::V8Memory.label(), "v8-memory");
        // v11: Texture and Memory also carry the banded TextureStream phase (severity-last).
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::V8HighFps), 10);
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::V8Texture), 11);
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::V8Transitions), 10);
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::V8Memory), 12);
        // Severity ladder: hang-prone detectors sit AFTER the last graceful TextureRop segment.
        for plan in [&texture, &memory] {
            let last_texrop = plan
                .iter()
                .rposition(|s| s.workload == VfWorkload::TextureRop)
                .unwrap();
            let stream = plan
                .iter()
                .position(|s| s.workload == VfWorkload::TextureStream)
                .unwrap();
            assert!(stream > last_texrop, "TextureStream must run after graceful detectors");
        }
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::Fsgl1), 8);
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::Fsgl3A), 8);
        for phase in [
            VfQualifierPhase::FrameCadence,
            VfQualifierPhase::VramPressure,
            VfQualifierPhase::GeometryDepth,
        ] {
            assert_eq!(VfQualifierPhase::from_code(phase.code()), Some(phase));
        }
    }

    fn duration_for_workload(plan: &[VfQualifierSegment], workload: VfWorkload) -> u64 {
        plan.iter()
            .filter(|segment| segment.workload == workload)
            .map(|segment| segment.duration_ms)
            .sum()
    }

    #[test]
    fn endurance_pattern_is_worst_realistic_soak_with_correct_coverage() {
        // One continuous ~20-min dwell; weights must scale to fill the whole requested duration.
        let plan = vf_qualifier_plan(1_200_000, VfQualifierPattern::Endurance);
        assert_eq!(
            plan.iter().map(|segment| segment.duration_ms).sum::<u64>(),
            1_200_000
        );
        // Worst-realistic ingredients: sustained max-power (HeavySpike) + cap-slam (HeavySpike ↔
        // IdlePulse) + fine droop transients (FrameCadence) + game realism (MixedGame), with the
        // graceful golden-checked TextureRop interleaved to catch a stress-induced silent error.
        for workload in [
            VfWorkload::HeavySpike,
            VfWorkload::IdlePulse,
            VfWorkload::FrameCadence,
            VfWorkload::MixedGame,
            VfWorkload::TextureRop,
            // v16.1 composite: heavy render + near-full VRAM-resident gather SIMULTANEOUSLY
            // (replaces the standalone Memory pass AND the sequential VramPressure segments).
            VfWorkload::CompositeGameLoad,
            // v16.2 lobby regime: sustained light-frame anchor-bin residency (the field killer).
            VfWorkload::BoostEdge,
        ] {
            assert!(
                plan.iter().any(|segment| segment.workload == workload),
                "endurance must exercise {workload:?}"
            );
        }
        // Sustained max-power dominates — this is a worst-case soak, harsher than a game's average
        // load (which MixedGame represents), not a single-detector burst.
        assert!(
            duration_for_workload(&plan, VfWorkload::HeavySpike)
                > duration_for_workload(&plan, VfWorkload::MixedGame)
        );
        // Cap-slam requires BOTH the heavy burst and the idle release to be present.
        assert!(
            duration_for_workload(&plan, VfWorkload::HeavySpike) > 0
                && duration_for_workload(&plan, VfWorkload::IdlePulse) > 0
        );
        // Phase-coverage denominator is exact and never panics (auto-derived from the plan):
        // PowerOpening, HeavySpike, TextureRop, IdlePulse, CompositeGameLoad, BoostEdge,
        // FrameCadence, MixedGame, PowerClosing = 9.
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::Endurance), 9);
        assert_eq!(VfQualifierPattern::Endurance.label(), "endurance");
    }

    #[test]
    fn transition_shock_pattern_cycles_idle_slam_with_graceful_detector() {
        // Fills the whole requested ~8-min dwell; weights scale exactly.
        let plan = vf_qualifier_plan(480_000, VfQualifierPattern::TransitionShock);
        assert_eq!(
            plan.iter().map(|segment| segment.duration_ms).sum::<u64>(),
            480_000
        );
        // Idle→slam cycles dominate (the launch transition under test); the graceful
        // golden-checked TextureRop between rounds catches slam-induced silent corruption.
        assert!(
            duration_for_workload(&plan, VfWorkload::BoostEntry)
                > duration_for_workload(&plan, VfWorkload::TextureRop)
        );
        assert!(duration_for_workload(&plan, VfWorkload::TextureRop) > 0);
        // Coverage denominator auto-derived: BoostEntry + TextureRop = 2 distinct phases; the new
        // phase code (12) must fit the coverage bitmap (COUNT) without panicking.
        assert_eq!(qualifier_expected_phases(VfQualifierPattern::TransitionShock), 2);
        assert!((VfQualifierPhase::BoostEntry.code() as usize) < VfQualifierPhase::COUNT);
        assert_eq!(
            VfQualifierPhase::from_code(VfQualifierPhase::BoostEntry.code()),
            Some(VfQualifierPhase::BoostEntry)
        );
        assert_eq!(VfQualifierPattern::TransitionShock.label(), "transition-shock");
    }

    #[test]
    fn golden_capture_requires_consistent_frames() {
        let mut reference = None;
        for frame in 1..=GOLDEN_MIN_FRAMES {
            observe_golden_checksum(&mut reference, 0x1234_5678, frame).unwrap();
        }
        assert_eq!(
            finish_golden_capture(reference, GOLDEN_MIN_FRAMES).unwrap(),
            0x1234_5678
        );

        let mut divergent = None;
        observe_golden_checksum(&mut divergent, 7, 1).unwrap();
        assert!(observe_golden_checksum(&mut divergent, 8, 2).is_err());
        assert!(finish_golden_capture(Some(7), GOLDEN_MIN_FRAMES - 1).is_err());
    }

    #[test]
    fn golden_mismatch_maps_to_silent_error() {
        assert_eq!(render_integrity_result(false, false), StabilityResult::Stable);
        assert_eq!(
            render_integrity_result(false, true),
            StabilityResult::SilentError
        );
        assert_eq!(render_integrity_result(true, false), StabilityResult::Crash);
    }

    #[test]
    fn reduce3_checksum_changes_with_single_texel_bit_flip() {
        let base = [0x1234_5678, 0x90ab_cdef, 0x0000_0000, 0xffff_ffff];
        let mut flipped = base;
        flipped[2] ^= 1;
        assert_ne!(
            reduce3_checksum_cpu(&base, 256 * 64),
            reduce3_checksum_cpu(&flipped, 256 * 64)
        );
    }
}
