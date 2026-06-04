# Nidavellir — Project Memory

Honest, safety-first GPU/CPU/RAM auto-tuner for Windows. Tauri v2 + Svelte 5 UI,
Rust core service, NVIDIA-focused undervolting. ~14 K LOC source across 5 Rust
crates + a Svelte UI. Tested on RTX 3060 Ti (driver 595.97), i7-13700K, DDR4-4000.

This file is the continuity index. See also: `architecture.md`, `decisions.md`,
`roadmap.md`, `handoff.md`, and the methodology doc `docs/gpu-forge.md`.

## Current status (2026-06-04)
- `master`, latest tag **v0.3.1**. V2 work on a worktree branch (uncommitted).
- Active work: **Brokkr's Best** profile refinement — the perf/watt undervolt.
- **V1** of the continuous per-GPU stability-knowledge algorithm is implemented,
  committed, and validated on hardware (no crash). Service + UI run; GPU at stock.
- **V2 (confidence-gated selection) is implemented + unit-tested** (compiles clean;
  3 tests pass). Not yet committed. See `decisions.md`.

## Completed work (this arc)
- **Modern NvAPI V/F curve (ClkVfPoints) read + write + apply + reset** work on
  driver 595.97 (the old `nvapi` crate's `SetClockBoostTable` is rejected). Elastic
  "VF ceiling" (Afterburner-style flatten) verified to control the live clock under
  load without a voltage lock. Integrated into `apply_core` (fallback = offset+NVML
  cap). UI shows support. Supported GPUs documented (desktop Pascal+).
- **Game-power dwell**: the sweep now stresses with the FurMark-class textured
  render (~199 W, saturates the 200 W cap like Overwatch), not the old compute load
  (~159 W, never capped). Made repeatable by bounding per-frame work.
- **Brokkr's = max efficiency (MHz/W)**, off-cap, NOT lowest voltage.
- **Continuous per-GPU knowledge (V1)**: severity-separated frontier + per-point
  stats persisted, data-driven margin (no fixed MHz). See `decisions.md`.
- 3-tier failure classification; Safe Loop reboot protection confirmed working.
- **V2 selection (this session)**: Wilson lower-bound confidence gate over
  accumulated trials; picks best `score()` (MHz/W) clearing the profile threshold
  (Balanced .85 active), else falls back to V1. Selection now reads the persisted
  knowledge (V1 only wrote it); off-cap invariant kept via an offset join. Code-only,
  no data-model/schema change. `cargo check` clean; 3 unit tests pass.

## Known issues / open questions
- A deep undervolt (+255 offset / ~855 mV) **hard-rebooted** the PC once — deep
  undervolt bugchecks (not just a recoverable TDR). Now learned + never re-probed.
- In-sweep, a HARD REBOOT does not auto-update `gpu_knowledge.json` (only
  SilentError/TDR do); a reboot is recorded via the Safe Loop boot-flag and must be
  folded into the knowledge (currently manual). → roadmap "Safe-Loop→knowledge".
- Render is heavier than real games → conservative bias (good) but exploring near
  the frontier under it still carries crash risk → SUPERVISED runs only.
- Run-to-run thermal variance in measured power (deferred refinement).
- 2 `.exe` binaries committed inflate the repo — confirm intent vs `.gitignore`/LFS.

## Next recommended actions
1. **Commit V2** (worktree branch) once reviewed; then consider exposing the
   profile via IPC/UI (currently a const).
2. **V3**: dedicated short confidence trials on the safe side so the gate matures —
   today every point has 1 trial → Wilson-LB ≈ 0.21 → V2 falls back to V1.
   `arduous_validate`'s 35 s soak discards its result: the natural hook to record it.
3. Optionally one more supervised sweep → converges at +240 (~870 mV ≈ user's
   hand-tuned 1800 MHz @ 875 mV), then stops by design.
4. In-game apply test of Brokkr's via the VF ceiling (user present).
