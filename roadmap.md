# Nidavellir — Roadmap

## Shipped
- **v0.1** — detection, sensors, capability report.
- **v0.2** — Safe Loop (reboot-surviving crash recovery).
- **v0.3 / v0.3.0** — staged GPU undervolt sweep engine.
- **v0.3.1** — real NVAPI read/write, wgpu stress battery, VRAM→core→memory→package
  soak, apply/persist/reapply-on-boot, "Forge everything" pipeline.
- **Post-v0.3.1 (this arc, on `master`, untagged):**
  - Modern ClkVfPoints VF curve read/write/apply/reset (elastic ceiling).
  - Game-power render dwell (repeatable); validation under game power.
  - Brokkr's = best MHz/W, off-cap; 3-tier failure classes; supported-GPU docs.
  - **V1 continuous per-GPU stability knowledge** (severity frontier, per-point
    stats, data-driven margin — no fixed MHz).

## Now — Brokkr's refinement
- **V2 (DONE this session, uncommitted):** Wilson lower-bound confidence gate;
  selection = max `score()` s.t. confidence ≥ profile threshold; profiles
  **Conservative .95 / Balanced .85 (active) / Aggressive .70**; V1 fallback when
  none clears the gate. Implemented + unit-tested in `gpu_power_sweep.rs`. Profile
  is a const (IPC/UI plumbing is a follow-up).
- **V3 (next):** dedicated short confidence trials on the safe side so the gate
  matures (today 1 trial/point → always falls back to V1); `arduous_validate`'s soak
  is the natural place to record confidence; reinforce promising points; tune
  `target_clock_mhz` selection.

## Near-term
- **Godforge** as a real OC profile (currently the max-voltage stock point).
- **In-game apply test** of Brokkr's via the VF ceiling (user present) — the final
  consistency verdict.
- **Safe-Loop → knowledge integration**: on a boot-flag-detected reboot, fold the
  crash offset (with `Reboot` severity) into `gpu_knowledge.json` automatically
  (today only in-sweep SilentError/TDR auto-record).
- Address thermal run-to-run variance (e.g. temp-gate the sweep start / settle).

## Deferred (per project design)
- AMD path (ADLX) — currently NVIDIA/NVAPI only.
- CPU and RAM tuning (GPU-first by design).
- Persisted community priors / knowledge base sharing (v0.7+).
- UI polish (test-layer UI for now).

## Housekeeping
- Confirm the 2 committed `.exe` binaries are intentional (PawnIO runtime) vs
  candidates for `.gitignore` / git-lfs (~17 K "LOC" of binary in the repo).
