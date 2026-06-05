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

## Now — Product model (see product.md)
Reframed around 3 profiles forged from a clock×power frontier. Phases:
- **F1 — Profile model**: 3-profile synthesis (Godforge=clock / Brokkr's=R / Deep
  Calm=MHz/W) + V2 confidence gate.
  - **F1a (DONE)** — pure `synthesize_forge_profiles` + unit tests. Not yet wired.
  - **F1b (next)** — extend the safe flatten sweep to multiple target clocks (real
    game-power frontier); knowledge keying by (clock, offset); wire synthesis in.
- **F2** transparency (clock/power deltas) · **F3** Forge modes (Quick/Deep/
  Continuous; breadth = #clock levels) · **F4** reboot→knowledge + limits 1/2/3 ·
  **F5** lifecycle (Forged→Legendary) · **F6** passive monitoring · **F7** UI
  (Forge GPU / Refine / Forge Progress).
- **V2** (Wilson gate) shipped — reused as the confidence axis for all 3 profiles.
  **V3** (confidence-maturing trials) folds into F1b/F4.

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
- Consolidate the two sweep engines (flatten vs lock-voltage) + redundant
  `synthesize_profiles` onto the safe flatten engine (see `product.md`).
