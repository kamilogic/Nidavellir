# v13 — Absolute clock ceiling (NVML locked clocks) for dwells and Apply

Status: IMPLEMENTED phases A–C (2026-07-06, code-complete, NOT HW-tested — 488/0 tests).
Remaining: safety audit + the three supervised HW gates in phase D.
Owner: backend. Predecessor: `docs/qualification-v8-plan.md` (v8–v12).

## Problem

The anchored apply already builds the Afterburner-style fixed point (anchor raised to target,
every higher-voltage bin capped down to target — `apply_bounded_anchored_positive_offset`,
verified per bin by `verify_anchored_positive_offset` incl. `HigherBinAboveTarget`). But the
caps are **offsets relative to the base curve**, and the driver shifts the whole base V/F curve
with temperature. Result, visible across the entire 2026-07-06 run: every pair measured
p5/p95 = label **+15/+30 MHz** (1770→1800, 1800→1830, …). Consequences:

- The published calm profile "1770 @ 856" actually runs the 1800 regime, sitting at the LOWEST
  bin of the shifted flat plateau — effectively ~1800 @ ~856 when cool. The operator's
  hand-validated point is 1800 @ 875; 1800 @ 868 is KNOWN unstable in game.
- The overshoot magnitude depends on ambient/temperature → non-deterministic delivered regime.
- The whole v12 regime-lift/reconciliation machinery exists only to compensate this symptom by
  voltage, and it relabels pairs: the operator's 1800@875 was displaced into the "1830 regime"
  rung and never selected for a profile slot.

## Fix (operator-approved)

Add an **absolute clock ceiling** via NVML locked clocks — `lock_core_clock_max_mhz(target)`
(`crates/core/src/nvml_gpu.rs:34`, min=210 keeps idle/downward elasticity; release =
`reset_core_clock_lock()`, :59) — **both during every F2 dwell and at Apply**. The ceiling is
absolute in MHz, immune to the thermal curve shift. The anchored VF curve keeps owning the
voltage axis; the lock owns the clock axis. Measured point == labeled point == applied point.

NOT the rejected rigid pin: decisions.md rejected `min=max` pin / voltage lock (removes power
management → TDR under power cap). Max-only cap preserves elasticity — record the distinction
in `decisions.md`.

## Invariants after v13

- **p95 == target** for every dwell (ceiling held). `p95 > target + tol` ⇒ hard dwell failure
  (behavioral lock verification — NVML has no locked-clocks getter).
- **p5 == target** when the point is not power/thermal-bound; p5 sag below target keeps its
  existing `ClockDrop`/boundary semantics (the lock only removes UPWARD variation).
- Regime == target by construction → v12 lift removed; strict p95 reconciliation
  (`f2_regime_support`) and the exact-Apply resynthesis loop KEPT as dormant fail-closed nets
  (they now only fire if a lock silently failed).

## Phases

### A — Lock lifecycle in the F2 motor + Apply path
1. `crates/service/src/gpu_undervolt.rs` (`RealF2Ops` apply site): after
   `AnchoredRaiseVerified`, set `lock_core_clock_max_mhz(target)`; lock failure ⇒ full unwind
   (reset curve + release lock) + `Err`, fail-closed. Add lock/unlock to the `F2Ops` trait
   (:1626) so mocks can assert the lifecycle.
2. `reset_to_stock` (gpu_undervolt.rs:2539/:2620 impls) and
   `gpu_power_sweep::reset_to_stock` (:4110 — comment already says "release any NVML clock
   cap"; verify and extend): always `reset_core_clock_lock()`, idempotent, on EVERY exit path.
3. `gpu_apply.rs`: `apply_and_persist_undervolt` sets the lock after the verified write (same
   unwind rule); `reapply_on_boot` (undervolt branch) re-applies curve AND lock;
   `reset` (ResetGpuTuning body) + Safe Loop recovery release the lock. Locks do not survive
   reboot — releasing anyway is cheap and idempotent.
4. No persistence change: `AppliedProfile.undervolt.target_mhz` already carries the lock value.

### B — Measurement under the ceiling
1. Every dwell that applies the anchored curve (discovery PowerRender, v8 qualification,
   p99 calibration, exact-Apply soaks) runs with the lock set to its target; released in the
   same band's reset.
2. Golden captures stay STOCK and UNLOCKED (stock reference). Note:
   `stream_frame_reference_ms` (A3 gate, 2× tolerance) was captured at stock boost; locked
   dwells run ≤ ~7% slower at our clock range — inside the 2× margin, no change.
3. Classifier (`classify_f2_stress_dwell`): new hard failure `p95 > target + tol`.
4. Contract bumps (`crates/core/src/f2_observation.rs`):
   `F2_DISCOVERY_CONTRACT_VERSION` 4 → **5**, `F2_QUALIFICATION_CONTRACT_VERSION` 11 → **12**.
   All pre-v13 evidence carries shifted regimes (+15/+30) and must not seed warm-starts nor
   unlock Apply. Full re-forge required.
5. Optional hardening (B4): at sustained target clock, gate measured voltage ≥ anchor − tol
   (v4 telemetry already records measured voltage). Covers the residual case where a cold
   shift lets an elastic bin BELOW the anchor reach target (only possible when the anchor
   raise is smaller than shift + one bin).

### C — Remove the v12 lift; keep the nets
1. `apply_f2_margin_policy` (gpu_power_sweep.rs:1417): drop the lift step; keep recording
   `base_apply_mv` (= apply, additive IPC field stays for UI compat).
2. `f2_regime_support` (:912) unchanged — dormant fail-closed net.
3. Adjust/remove lift unit tests (gpu_power_sweep.rs:6797-6831 region); add no-cascade test
   replacement asserting reconciliation passes trivially when p95 == target.
4. The 1875 lift-ordering follow-up from the 2026-07-06 run dies by construction.

### D — Validation and gates
1. Unit: mock `F2Ops` asserts lock set/release ordering on every path (success, verify-fail,
   dwell failure, Stop, panic-unwind); classifier p95-above-target failure; contract gating
   (old versions rejected). `cargo check`/`test --workspace`, clippy on new code.
2. `nidavellir-safety-auditor` on the diff BEFORE commit (hardware semantics + reset paths +
   contract change).
3. Supervised HW gates (operator-run, in order):
   a. **Lock sanity (cheap, no forge)**: set the lock at 1800 on the rig (CLI probe or
      `nvidia-smi -lgc 210,1800`), run a game/stress ~5 min, confirm the clock NEVER exceeds
      1800 (this is the operator's Afterburner-observed behavior), release, confirm stock
      boost returns. Proves driver 595.97 / RTX 3060 Ti support.
   b. **Re-forge Standard** (state cleared). Expect: every dwell p5≈p95≈target (no +15/+30);
      boundaries land ~1–2 bins HIGHER in voltage than the v12 run for the same label;
      **Brokkr's/Deep Calm publish ≈ 1800 @ ~875 (operator ground truth — the primary gate)**;
      no regime-lift/reconciliation lines in the log; published p99 watts slightly lower for
      the same label (previously measured at label+30).
   c. **Apply gate**: apply Brokkr's; confirm in-game the clock ceiling holds exactly at
      target under load and idle downclock still works; reboot; confirm reapply-on-boot
      restores curve + lock.
4. `decisions.md`: new entry — max-only NVML ceiling adopted for F2 dwells+Apply; distinct
   from the rejected rigid pin (min=max), which stays rejected.
