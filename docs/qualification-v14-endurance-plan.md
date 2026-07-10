# Qualification v14 — Candidate-only endurance gate + directed fallback

Status: STAGE 1 IMPLEMENTED + REINFORCED to worst-realistic + validated (cargo check/test/clippy
green; NOT HW-tested, NOT committed). STAGE 2 (directed fallback) NOT started — mechanism fully
specced below. Safety audit pending (whole v14 diff). 2026-07-09.

Stage 1 deviation from the sketch below: the endurance pass is folded INTO
`gate_anchored_candidate_fsgl3` (gpu_undervolt.rs), gated on `exact_apply`, right after the 3-pattern
loop — so the frontier descent never pays it and no new runner duplicates the dwell/persist
boilerplate. Distinct `F2QualificationPattern::Endurance` (NOT in `REQUIRED_QUALIFICATION_PATTERNS`,
code 5) is the run-scoped publish marker via `point_has_current_endurance_qualification`; the reconcile
loop's `already_qualified` skip now ALSO requires current-run endurance (closes the resume hole). No
contract bump. `F2_ENDURANCE_QUALIFICATION_DWELL_MS = 900_000`. gpu-stress `Endurance` pattern =
MixedGame-dominant + FrameCadence + interleaved graceful TextureRop, 5 distinct phases.

## Why

The single-detector frontier descent is honest but its per-bin gate (60 s Texture)
and even the exact-Apply gate (3 patterns × 5 min) are systematically *easier* than
real games at the same voltage: an `1800@868`-class point passes here yet is
game-unstable (operator ground truth). Root cause is workload realism + duration,
NOT precision noise — the boundary is repeatable ±1 bin across runs, every failure a
graceful `texture-rop` SilentError. The current exact-Apply gate also runs each
pattern as an INDEPENDENT `run_confirmed_f2_step` (arm→apply→dwell→**reset_to_stock**),
so the 3 patterns never chain into continuous thermal saturation
(`gate_anchored_candidate_fsgl3`, gpu_undervolt.rs:4215-4360).

Fix: spend expensive realism ONLY on the 3 profile candidates (not the 68-bin
frontier), and on failure adapt the candidate in a classification-appropriate
direction instead of just failing closed.

## Confirmed decisions (operator, 2026-07-09)

- **Endurance soak — WORST-REALISTIC** (operator: "se possível pode ser até pior para cobrir
  o pior cenário"): one CONTINUOUS ~20-min dwell per candidate, no mid-soak reset. Sustained
  max-power (HeavySpike) + cap-slam (HeavySpike↔IdlePulse, the 1920@918-class transient) +
  FrameCadence droop + MixedGame realism, with graceful golden-checked TextureRop interleaved.
  Deliberately harsher than a real game (a PASS ⇒ games safe with margin) but NOT a synthetic
  power-virus (that would reject game-stable points and cost clock/efficiency). Calibration
  knobs left for HW tuning: HeavySpike amplitude, burst/idle weight ratio, FrameCadence gap.
  DONE + validated (gpu-stress 10/0).
- **Directed fallback = "preserve identity"** on a candidate failing the endurance gate:
  - **Godforge / Brokkr's** (perf): +1 voltage bin, SAME clock. If that bin would breach
    the off-cap ceiling (`POWER_HEADROOM_FRAC`), step −1 clock instead.
  - **Deep Calm** (efficiency): −1 clock, SAME voltage bin (never add power).
  - Bounded to a few steps; if nothing passes, that profile fails closed (unpublished).

## Stage 1 — endurance gate (candidate-only)

Building blocks exist: `VfWorkload::{MixedGame,FrameCadence}`, `VfQualifierPhase::*`,
pattern = phase-list (gpu-stress/src/lib.rs:214-435), golden capture
(`capture_one_golden` :2021). MixedGame decomposes into golden-checked sub-workloads.

1. **gpu-stress**: add an `Endurance` pattern phase-list mixing MixedGame + FrameCadence
   (+ the graceful Texture detector so silent errors are still caught mid-soak). One
   pass = one continuous dwell of `F2_ENDURANCE_DWELL_MS (≈900_000)`. Reuse existing
   goldens; no new shader.
2. **gpu_undervolt.rs** `run_confirmed_f2_apply_qualification` (:4369): after the existing
   3-pattern set passes, run ONE endurance pass (15-min dwell, `exact_apply=true`) via the
   same `RealF2MultiOps`/`run_confirmed_f2_step` path (arm→apply→15-min dwell→reset). A
   SilentError/Unstable here → `ExactApplyRejected`. Keeps the clock ceiling + all safety.
3. **Gate marker — additive, NO contract bump**: new `endurance_qualified: bool` on the
   Apply-qualification observation/point + a publish-gate requirement that the CURRENT run
   passed endurance at the exact Apply point. Rationale: the endurance gate only tightens
   Apply; it does NOT change frontier descent, so bumping the shared
   `F2_QUALIFICATION_CONTRACT_VERSION` would needlessly quarantine reusable frontier data.
   Backward-compatible (additive field, legacy points read `false` → fail closed = safe).

## Stage 2 — directed fallback (exact-Apply loop) — NOT YET IMPLEMENTED (mechanism fully specced)

Loop today (gpu_power_sweep.rs:~6455-6700): `loop { eligible = classified − excluded;
profiles = synthesize_forge_profiles_capped(&eligible,…); for each selected profile: run
apply-qual; on fail excluded_apply_pairs.insert(key) + changed=true; break → resynth }`.

KEY FACTS discovered (make Stage 2 clean):
- Synthesis reads the mutable `classified` set (via `eligible`) and picks the Apply voltage
  from each point's `vf_table_voltage_mv` (`f2_apply_key` = (target_clock_mhz|clock_mhz,
  vf_table_voltage_mv)).
- Synthesis ALREADY runs the off-cap invariant internally (`is_off_cap_safe`, using the
  point's `max_power_w`.max(`power_p99_w`) vs `off_cap_ceiling_w` = cap·(1−6%)).
- Classification of a failing key = compare it to `profiles.{godforge,brokkrs,deep_calm}`.

MECHANISM (preserve-identity, elegant — let synthesis's off-cap gate make the raise-vs-drop
decision instead of duplicating it):
1. On a GRACEFUL rejection only (summary.qualified==false && !aborted && !cancelled — a hard
   DeviceLost still aborts the forge; do NOT fallback on abort).
2. Determine classification of `key`.
   - PERF (godforge/brokkrs): let `a_next = f2_next_bin_above(sane, key.1)`. If Some and the
     per-clock raise budget isn't spent:
       a. `run_confirmed_f2_power_calibration(T, a_next, …)` → measures the neighbor's p99 (so
          `is_off_cap_safe` has REAL power; a naive clone underestimates).
       b. `run_confirmed_f2_apply_qualification(T, a_next, …)` → 3-pattern + 20-min endurance.
       c. If qualified: MUTATE clock T's `classified` point → `vf_table_voltage_mv = a_next`,
          `max_power_w`/`power_p99_w`/`p95_clock_mhz` = measured, `apply_qualified = true`,
          version = current. `changed = true; break` → resynth. Synthesis then either KEEPS
          clock T @ a_next (still off-cap) or, if the higher voltage is now at-cap, its off-cap
          gate drops it → Godforge falls to the next lower clock automatically. No separate
          off-cap pre-check needed.
       d. If not qualified: `excluded_apply_pairs.insert((T,a_next))`, bump the raise budget,
          and either retry a_next2 (if budget) or exclude (T,key.1) → step-down resynth.
   - CALM (deep_calm): exclude `key` → existing resynth already steps to a lower/other point
     (matches "−1 clock" intent; no new code).
3. Bound: `F2_ENDURANCE_FALLBACK_MAX_RAISES` (≈1–2; each raise costs a p99 calib + 20-min
   soak ≈ 25 min, so keep TIGHT). Terminates: every step either raises voltage (monotonic, ends
   at f2_next_bin_above→None or off-cap) or drops clock (monotonic, ends at frontier floor).
4. Test: a pure helper for classification + neighbor selection + budget (the loop itself is
   windows-only / not mock-tested, like the rest of the reconcile loop).

NOTE: the reinforced endurance gate (Stage 1) ALREADY makes failure-adaptation SAFE — the
existing exclude→resynth converges to a point that survives the worst-realistic soak. Stage 2
only changes the PREFERENCE (perf keeps its clock by adding voltage). No safety gap without it.

## Tests

- gpu-stress: endurance pattern present in the exact-Apply set; phase coverage denominator
  correct (no `[false; N]` panic — cf. the v8 phase-code-8 fix).
- Pure: directed-neighbor helper — Godforge/Brokkr's → +1 V-bin, cap breach → −1 clock;
  Deep Calm → −1 clock same V; bound honored; fail-closed when exhausted.
- Publish gate: a point WITHOUT current-run `endurance_qualified` is never published.

## Safety audit checklist (before HW)

- Endurance dwell uses the SAME arm→apply→verify→dwell→reset motor + clock ceiling; a
  15-min continuous dwell must still cooperatively cancel (Stop) within a band and
  reset-clean on every exit.
- Fallback neighbors re-run the FULL safety precheck (Safe Mode / boot flag / blacklist)
  per `select`; a blacklisted neighbor is skipped, not applied.
- Off-cap invariant still holds for raised-voltage Godforge/Brokkr's neighbors.
- Fail-closed: no publish path bypasses `endurance_qualified` for the current run.
- Directed search terminates (monotonic + bounded).

## Deferred

- Godforge near-cap cap-slam burst (only if a published off-cap point still TDRs in-game).
- Q3 calibrated per-clock margin + Q4 golden regression gate (separate track; the
  endurance gate may make the blanket margin unnecessary).
