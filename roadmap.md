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
  - **P0 restart/accounting closure (2026-07-15)** — run/candidate checkpoints, installed-service
    sentinel parity, durable Needs Attention incidents with explicit acknowledgement, run-scoped
    export and content-addressed dirty build identity. Real-use profile failures can be marked from
    their hardware-derived card and become durable local blacklist evidence.

## Now — Product model (see product.md)
Reframed around 3 profiles forged from a clock×power frontier. Phases:
- **Current F2 frontier (2026-07-15)** — code-complete: Forge now normalizes stock
  deterministically before reading the live domain. **Ctable** (sane physical-table ceiling/count),
  **Cboost** (post-preheat observed boost) and **Cmax** (first sustainable clock proved by discovery)
  are distinct facts. Preheat requires two converged usable 10 s windows and fails closed before any
  candidate if stock temperature/p5, throttle or telemetry cannot be trusted. The measured frontier
  still covers Cmax→90% Cmax; Fast remains provisional and Standard/Long remain deployability modes.
  Discovery contract v5 runs each candidate attempt as one Candidate Transaction: one Safe Loop arm
  and curve apply/verify, PowerRender plus active qualification phases without reapplying, then one
  checked reset and boot-flag clear. Qualification is persisted before discovery so resume cannot see
  a positive discovery without its same-curve rejection evidence. A positive observation becomes
  reusable only after cleanup is proven. Numeric p99 classification has hysteresis: NearCap ≥99%,
  OffCap ≤98%, and the middle band retries or ends inconclusive. Qualification contract v17 records
  complete build/workload/graphics/golden provenance; older positives remain readable but are
  ineligible. MixedGame now records BoostEdge + TextureRop + PowerRender in every frame/submit;
  BoostEdge/MixedGame checksum reduction is sparse GPU-side and every sampled mismatch accumulates.
  The exact-Apply closure now adds a native offscreen DX11 stock-golden gate between Texture and the
  longer stages: Texture 5 min, DX11 5 min, TransitionShock 8 min and Endurance 20 min per unique pair.
  It selects the NVIDIA adapter explicitly, records its LUID/provenance, and bounds GPU completion
  polling below the Windows watchdog regime. The additive ForgeProgress preheat/Ctable/Cboost fields
  expose this domain without parsing logs. The explicit F2 Apply path and legacy F1 remain intact.
  The first post-reset Standard hardware cycle completed across a resumed two-run sequence and
  published `1890@893`, `1845@862` and `1740@800`. Endurance rejected stochastic `1905@900` and
  `1860@868`, but the known field discriminator `1845@862` still passed. During unattended descent the
  operator returned to a Windows login after a reported TDR/reboot; no observation or sentinel event
  attributed the active point before checkpoint resume. Restart reconciliation is now implemented;
  the operator also confirmed `1845@862` repeatedly unstable in real use. **Next evidence/safety
  gate:** deploy this build, mark every confirmed profile unstable from its card, then perform a clean
  Forge evaluation of qualification v17. No final-gate duration was reduced; the new DX11 stage must
  first demonstrate field discrimination without false positives on safe controls.
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
- **Hardware verification of P0:** deploy the rebuilt service, exercise a controlled interrupted run,
  and verify Needs Attention/acknowledgement plus run-scoped export without inducing a TDR on purpose.
- **Godforge** as a real OC profile (currently the max-voltage stock point).
- **In-game apply test** of Brokkr's via the VF ceiling (user present) — the final
  consistency verdict.
- **Safe-Loop → knowledge integration**: on a boot-flag-detected reboot, fold the
  crash offset (with `Reboot` severity) into `gpu_knowledge.json` automatically
  (today only in-sweep SilentError/TDR auto-record).
- **Physical qualifier calibration**: compare 1845 MHz @ 862 mV with known-safe bins under the new
  DX11 gate before considering any shortening of Texture/DX11/TransitionShock/Endurance.

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
