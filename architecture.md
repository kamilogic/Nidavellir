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
  (FailureSeekingGameLoop: render/ROP/texture/compute/idle transients),
  `run_power_load` (compute), `run_combined`, bandwidth. `MixedGame` records
  BoostEdge + TextureRop + PowerRender in every frame/submit; BoostEdge and
  MixedGame use sparse GPU-side reduction/compare and accumulate every sampled
  mismatch. Each load returns a `StabilityResult` (Stable / SilentError / Crash).
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
- **Live F2 Forge** (`gpu_power_sweep.rs` + `gpu_undervolt.rs`, current
  2026-07-15): first proves a deterministic stock preheat, then keeps three clock
  facts separate: **Ctable** is the ceiling/count of sane physical base-table bins,
  **Cboost** is the maximum live boost observed after preheat, and **Cmax** is the
  first reset-clean sustainable clock proved by discovery. Preheat uses up to six
  10 s stock windows and requires two consecutive usable windows with no throttle,
  temperature convergence within 2 °C and p5 convergence within 30 MHz; an
  inconclusive preheat aborts before tuning. The frontier remains Cmax→90% Cmax.
  Each discovery attempt is a **Candidate Transaction**: arm Safe Loop and
  apply/verify the curve once, run PowerRender plus any active qualifier phases
  under that same curve, then perform one checked reset/boot-flag cleanup.
  Qualification observations are persisted before their discovery observation;
  positive evidence is reusable only after both reset-to-stock and flag clear are
  proven. Power-cap classification is hysteretic: NearCap at p99 ≥99% of the
  numeric limit, OffCap at ≤98%, and the interval between them is Ambiguous and
  must retry or end inconclusive. Qualification contract **v16** records build
  version/revision, workload fingerprint, selected backend/adapter/driver,
  checksum method and golden configuration. Pre-v16 positives remain readable
  but cannot unlock Apply. `MixedGame` is truly interleaved per frame, with sparse
  GPU-side checks whose mismatch total is cumulative. Standard/Long keep the
  exact-Apply gate at every selected pair: Texture 5 min, TransitionShock 8 min,
  then Endurance 20 min. DX11 coverage and any reduction of that final gate remain
  gated on physical A/B calibration of 1845 MHz @ 862 mV against known-safe bins.
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
  contract versions, full v16 provenance, cleanup proof, coverage summaries and
  crash-safe resume checkpoints.
- `forge_state.json` — last complete usable forged profile snapshot; partial F2 runs
  never overwrite it.
- `boot_flag.json` / `heartbeat.txt` — Safe Loop liveness/boot detection.

## Platform constraints
NVIDIA-only (NVAPI). Modern VF curve needs desktop Pascal+ on a current driver
(verified 595.97). Falls back to global offset + NVML clock cap where unavailable.
