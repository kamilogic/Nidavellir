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
- **Current F2 frontier (2026-06-30)** — code-complete: the live Forge starts at the highest real
  clock, discovers Cmax through power-bound voltage descent, and characterizes every real bin through
  90% Cmax. Autonomous descent has no arbitrary step budget; Fast is provisional discovery;
  Standard/Long use FSGL3 A/B 2×60 s as the default interleaved per-bin qualifier.
  Discovery keeps the homogeneous game-power render while qualification uses versioned
  stock-golden evidence, 100% frame checks and deliberate droop probes; an FSGL3 rejection stops the
  descent with the last FSGL3-qualified physical bin;
  observations checkpoint/resume by GPU UUID; partial UI progress is durable; adjacent clocks reuse
  one bin above the prior minimum and fall back to the prior power-bound boundary; deployable synthesis
  still requires the complete range and successful qualification.
  The explicit F2 Apply path remains verified and legacy F1 remains intact. **Next: supervised
  FSGL3 validation against the known 1920 MHz @ 912 mV and 1935 MHz @ 918 mV failures**, then Standard
  Forge, Apply/reboot validation and separate Phase 3 F1 cleanup review.
- **F1 — Profile model**: 3-profile synthesis (Godforge=clock / Brokkr's=R / Deep
  Calm=MHz/W) + V2 confidence gate.
  - **F1a (DONE)** — pure `synthesize_forge_profiles` + unit tests. Not yet wired.
  - **F1b Phase 1 (DONE)** — policy-driven multi-clock synthesis (pure, service-internal):
    `ForgePolicy` (Balanced 0.98/0.90/0.85; Conservative/Aggressive presets), clock floors,
    Godforge=highest sustained clock (p5-aware), Brokkr's=max R within floor, Deep Calm=max
    MHz/W within floor, single-clock collapse handled, voltage not a selection axis. 44 tests.
  - **F1b Phase 2A (DONE)** — simulated multi-clock outer-loop scaffolding: `build_frontier`
    (generic over an injected probe closure) proves per-target voltage-bin descent, stopping
    rules, known-unsafe boundary, frontier assembly, and synthesis wiring with NO hardware.
    8 sim tests (3060 Ti + 4090 proven through the loop). No `load_and_measure`/`apply_vf_ceiling`
    call, no VF write, no Safe Loop interaction.
  - **F1b Phase 2B (next, NOT started)** — fill the real probe closure (apply ceiling at bin →
    Safe-Loop-armed `load_and_measure` dwell → offset-readback `VerifiedCurve` gate), wire
    `build_frontier` into a supervised/approval-gated entry point; add `target_clock_mhz` to
    points if the real path needs it. Hardware-risky → supervised.
  - **F1b Phase 3** — knowledge keying by (target_clock, vf_table_voltage_bin) + global
    voltage-floor crash boundary; backward-compatible `gpu_knowledge.json` migration.
- **F2** transparency (clock/power deltas) · **F3** Forge modes (Fast/Standard/
  Long; dwell/evidence only, identical frontier) · **F4** reboot→knowledge ·
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
