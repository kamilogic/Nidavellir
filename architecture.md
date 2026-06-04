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
- **crates/gpu-stress**: wgpu (Vulkan/DX12) loads — `run_render_stress` (FurMark-
  class textured render = game power), `run_power_load` (compute), `run_combined`,
  bandwidth. Each load returns a `StabilityResult` (Stable / SilentError / Crash).
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
- **Power sweep** (`gpu_power_sweep.rs`): holds the stock target clock, raises a
  curve-flatten **offset**, and measures (voltage out, clock, power, cap%) under the
  game-power render. Synthesizes **Godforge** (max-voltage / OC-oriented) and
  **Brokkr's** (best MHz/W, off-cap). Arduous 35 s soak validates the pick.
- **VF ceiling** (`gpu-nvapi`): the apply mechanism. Flattens every curve point at
  or above a chosen voltage to a target clock via per-point ClkVfPoints offsets — no
  voltage lock / no NVML clock pin, so the GPU keeps power-management elasticity.
- **Continuous knowledge** (`gpu_power_sweep.rs`): `GpuKnowledge` per GPU — a
  severity-separated frontier + per-offset stats, persisted and accumulated across
  runs. Drives the data-driven exploration ceiling.

## Persistence (C:\ProgramData\Nidavellir\)
- `safe_loop.json` — Safe Loop record (state, consecutive_crashes, blacklist).
- `gpu_applied.json` — the currently applied profile (re-applied on boot).
- `gpu_knowledge.json` — per-GPU stability knowledge (frontier + per-point stats).
- `boot_flag.json` / `heartbeat.txt` — Safe Loop liveness/boot detection.

## Platform constraints
NVIDIA-only (NVAPI). Modern VF curve needs desktop Pascal+ on a current driver
(verified 595.97). Falls back to global offset + NVML clock cap where unavailable.
