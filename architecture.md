# Nidavellir — Architecture

Windows-only. Tauri v2 desktop app (Svelte 5 UI) talks to a Rust **Core Service**
over a named pipe; the service does all hardware access (NVAPI, NVML, PawnIO).

## Components
- **apps/ui** (Svelte 5 runes, Tauri): the front end. `Forge.svelte` is the main
  tuning view; `VfChart.svelte` draws the V/F curve. i18n en/pt in `lib/i18n.js`.
- **apps/ui/src-tauri**: Tauri shell; bundles the PawnIO driver resources.
- **crates/core**: hardware detection, sensors, V/F sweep types, the **Safe Loop**,
  and all IPC request/response types (`ipc.rs`). No HW writes here.
- **crates/service**: the Windows service + IPC server (`ipc_server.rs`). Owns the
  background runners: `gpu_power_sweep.rs` (the Brokkr's/Godforge engine),
  `gpu_apply.rs`, `gpu_benchmark.rs`, `gpu_sweep_real.rs`, `gpu_real.rs`.
- **crates/gpu-nvapi**: NVAPI access — read the V/F curve, set offsets, and the
  modern **ClkVfPoints** FFI (the VF ceiling). Most `unsafe` lives here.
- **crates/gpu-stress**: wgpu (Vulkan/DX12) loads — `run_render_stress` (steady
  FurMark-class textured render = game power), `run_vf_qualifier_stress`
  (FailureSeekingGameLoop: render/ROP/texture/compute/idle transients with per-phase checksums),
  `run_power_load` (compute), `run_combined`, bandwidth. Each load returns a
  `StabilityResult` (Stable / SilentError / Crash).
- **crates/driver-pawnio**: MSR / SuperIO access via the PawnIO driver (CPU/RAM
  factory-clock detection, fan/sensor reads).

## IPC
Named pipe `\\.\pipe\NidavellirCore`. **Param-free methods** (the UI/scripts call a
method by name; state lives server-side). `scripts/ipc.ps1 -Method <Name>` is the
headless client used for sweeps/benchmarks. Requests/responses are the
`IpcRequest`/`ResponseData` enums in `core/src/ipc.rs`.

## Key subsystems
- **Safe Loop** (`core/src/safe_loop.rs`): reboot-surviving crash recovery. Arms a
  boot-flag (the tuning point) before a risky apply/measure; on reboot a still-armed
  flag means the last op crashed → don't re-apply; blacklist the region; after 3
  consecutive crashes → Safe Mode (stock, hands-off). Persists to ProgramData.
- **Live F2 Forge** (`gpu_power_sweep.rs` + `gpu_undervolt.rs`): resets to stock,
  then enumerates every real live-VF clock from the highest bin downward. For each clock, it raises a
  lower-voltage anchor to the target and caps higher bins to that target, then runs
  the fail-closed dwell/reset motor until the voltage boundary or physical floor.
  Power-bound clock drops continue the same clock whenever sustained p99 remains at the cap, even
  after that clock previously sustained; the first sustained
  clock is Cmax. The complete profile frontier spans Cmax→90% Cmax so Deep Calm is selected from
  measured data. Fast produces a provisional map; Standard/Long capture three stock render goldens
  with fresh contexts, then use FSGL3 A+B 2×60 s as the default interleaved per-bin qualifier.
  Discovery predicts the next boundary from compatible same-GPU v4 history and a short isotonic
  cross-clock trend, but starts one physical bin above it and remeasures all evidence. In a confirmed
  power-bound region it may skip 4/2/1 bins by p5 deficit, bounded by 25 mV and the existing writer
  offset-step cap. A reset-clean failure after a jump is recovered only upward by midpoint; after the
  first approved off-cap point the exact boundary is finished one physical bin at a time.
  Discovery keeps the homogeneous power render so p5, power-limit and `ClockDrop` stay comparable.
  Discovery contract v4 preserves mean/p99/raw-peak watts and thermal validity separately, rechecks
  anomalous adjacent-bin p99 at the exact bin, and excludes unconfirmed/v3 positives. Standard/Long
  defer FSGL3 while confirmed p99 remains at cap. After the +12 mV apply policy snaps to a physical
  bin, any missing exact-target/apply-bin power telemetry is backfilled with discovery-only
  PowerRender (no repeated FSGL3), and profile synthesis uses that exact bin's confirmed conservative
  p99 and p5;
  qualification uses TextureRop/MixedGame-biased transients, deliberate droop bursts and on-GPU
  verification of every rendered frame. A rejected FSGL3 candidate stops descent at the last
  FSGL3-qualified physical bin. F2 Apply fails closed until current-contract v4 FSGL3 A+B succeeds;
  FSGL1/FSGL2 remain readable legacy evidence and retain their original stress behavior.
- **Anchored VF undervolt** (`gpu-nvapi`): raises exactly one real lower-voltage
  anchor and caps higher-voltage bins to the target via per-point ClkVfPoints
  offsets — no voltage lock / no NVML clock pin, so lower bins retain elasticity.
  The clock ceiling is the stock VF top, and reset is write/readback checked.
- **Legacy F1 sweep/ceiling** (`gpu_power_sweep.rs`): retained for legacy
  `is_undervolt == false` payloads; no longer backs the live Forge button.
- **Continuous knowledge** (`gpu_power_sweep.rs`): `GpuKnowledge` per GPU — a
  severity-separated frontier + per-offset stats, persisted and accumulated across
  runs. Drives the data-driven exploration ceiling.

## Persistence (C:\ProgramData\Nidavellir\)
- `safe_loop.json` — Safe Loop record (state, consecutive_crashes, blacklist).
- `gpu_applied.json` — the currently applied profile (re-applied on boot).
- `gpu_knowledge.json` — per-GPU stability knowledge (frontier + per-point stats).
- `f2_observations.jsonl` — append-only, GPU-UUID-scoped F2 discovery/qualification evidence,
  contract versions, coverage summaries and crash-safe resume checkpoints.
- `forge_state.json` — last complete usable forged profile snapshot; partial F2 runs
  never overwrite it.
- `boot_flag.json` / `heartbeat.txt` — Safe Loop liveness/boot detection.

## Platform constraints
NVIDIA-only (NVAPI). Modern VF curve needs desktop Pascal+ on a current driver
(verified 595.97). Falls back to global offset + NVML clock cap where unavailable.
