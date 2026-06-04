# Nidavellir — Session Handoff

How to pick this up cold. State as of 2026-06-04, `master` (clean, latest commit
`2f785cb`). Deep NvAPI struct details live in `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Where things stand
- **Brokkr's V1 (continuous per-GPU knowledge) is implemented + validated on HW.**
- **V2 (confidence-gated selection) is implemented + unit-tested (uncommitted, on
  worktree branch `claude/vigilant-gagarin-213d23`).** `cargo check` clean;
  `cargo test -p nidavellir-service` = 3/3. Active profile = Balanced (.85), a const.
  With today's 1-trial data the gate falls back to V1, so the chosen point is
  unchanged — but the decision is now logged.
- Last supervised sweep explored to **+210 offset (~881 mV)** with NO crash and
  found Brokkr's = **1830 MHz @ 881 mV · 179 W · 10.24 MHz/W (off-cap)** — essentially
  the user's hand-tuned 1800 MHz @ 875 mV. Godforge = stock max-voltage point.
- GPU is at **stock**; service + UI were running; Safe Loop baseline is clean.

## Build / run (from repo root C:\Users\leona\dev\nidavellir)
- Build service: `cargo build --release -p nidavellir-service`
  (STOP the service first or the .exe is locked:
  `Get-Process nidavellir-service | Stop-Process -Force`).
- Run service (headless): `./target/release/nidavellir-service.exe console`
  (logs → `target/svc.log`).
- Run UI: `cd apps/ui && npm run tauri:dev`.
- Headless control (named-pipe client): `scripts/ipc.ps1 -Method <Name>`:
  `StartPowerSweep` / `GetPowerSweepProgress` / `StopPowerSweep` /
  `ResetGpuTuning` / `ApplyPowerBrokkrs` / `GetGpuCurve` / `GetSafeLoopStatus`.

## Learned knowledge (C:\ProgramData\Nidavellir\gpu_knowledge.json)
- `boundary`: highest_clean **210**, lowest_reboot **255** (silent_error/tdr null).
- 15 per-offset PointStats (0→210), 1 trial each, 0 failures.
- Next sweep's data-driven ceiling = **+240** (~870 mV), then it CONVERGES (cap
  ABS_MAX_OFFSET=240; never re-touches the 255 reboot).

## Pending / next actions
1. **Commit V2** (worktree branch `claude/vigilant-gagarin-213d23`) once reviewed;
   then consider exposing the profile via IPC/UI (currently a const).
2. **V3**: confidence trials so the gate matures (today 1 trial/point → falls back to
   V1). `arduous_validate`'s 35 s soak discards its result — the natural hook.
3. Optional: one more supervised sweep → +240, closes the exploration.
4. In-game apply test of Brokkr's (`ApplyPowerBrokkrs`) — confirm consistency in
   Overwatch. Small TDR risk; user present.

## Gotchas / safety
- **Deep undervolt can HARD-REBOOT** (not just TDR). +255/~855 mV did. Never
  auto-run deep exploration — supervised only. The knowledge bounds the search.
- The render is **heavier than real games**, so it destabilizes at a higher voltage
  than games → the validated point is conservative (good), but probing near the
  frontier under it still risks a reboot.
- **In-sweep a hard reboot does NOT auto-update `gpu_knowledge.json`** (only
  SilentError/TDR do). After a reboot, read the Safe Loop `boot_flag.json` offset
  and set `lowest_reboot` in the knowledge manually (until the integration lands).
- Rebuilding requires stopping the running service (file lock).
- Run-to-run thermal variance: start sweeps on a cool GPU for representative numbers.

## Files to know
- `crates/service/src/gpu_power_sweep.rs` — the sweep + knowledge model (V1) + the
  3-tier `FailTier` + `GpuKnowledge`/`BoundaryKnowledge`/`PointStat` + **V2**
  (`wilson_lower_bound`, `SweepProfile`, `select_brokkrs_v2`, unit tests).
- `crates/gpu-nvapi/src/lib.rs` — `vfcurve` mod (ClkVfPoints FFI), `apply_vf_ceiling`,
  `read_vf_curve_modern`, `vf_curve_supported`.
- `crates/service/src/gpu_apply.rs` — apply via VF ceiling (NVML cap = fallback).
- `crates/gpu-stress/src/lib.rs` — `run_render_stress` (game-power dwell).
- `crates/core/src/safe_loop.rs` — crash recovery.
- `docs/gpu-forge.md` — methodology + supported-GPU table.
