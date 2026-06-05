# Nidavellir — Project Memory

Honest, safety-first GPU/CPU/RAM auto-tuner for Windows. Tauri v2 + Svelte 5 UI,
Rust core service, NVIDIA-focused undervolting. ~14 K LOC source across 5 Rust
crates + a Svelte UI. Tested on RTX 3060 Ti (driver 595.97), i7-13700K, DDR4-4000.

This file is the continuity index. See also: `architecture.md`, `decisions.md`,
`roadmap.md`, `handoff.md`, and the methodology doc `docs/gpu-forge.md`.

## Current status (2026-06-04)
- `master`, tag **v0.3.1**. Work on worktree branch `claude/vigilant-gagarin-213d23`.
- Active work: **product model** — 3 profiles forged from a clock×power frontier
  (Godforge/Brokkr's/Deep Calm). See `product.md`.
- **V1** continuous per-GPU stability knowledge: implemented, committed, HW-validated.
- **V2** confidence-gated selection: implemented + unit-tested, **committed** (5d72342).
- **F1a (this session)**: pure 3-profile synthesis (`synthesize_forge_profiles`) +
  tests — Godforge=clock / Brokkr's=R / Deep Calm=MHz/W; not yet wired (F1b).
  6 tests pass; uncommitted. See `decisions.md`.

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
1. **Commit F1a** (synthesis + tests + vision docs) once reviewed.
2. **F1b**: extend the safe flatten sweep to multiple target clocks → real game-power
   clock×power frontier; decide knowledge keying by (clock, offset); wire
   `synthesize_forge_profiles` into the live sweep (replaces the single-clock picks).
3. Then F2–F7 (see `product.md` / `roadmap.md`).
4. In-game apply test (user present); optional one more supervised sweep → +240.
