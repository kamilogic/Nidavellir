# Qualification v14 — Candidate-only endurance gate + directed fallback

Status: STAGE 1 (worst-realistic endurance) SHIPPED in `0629a9f` and HW-VALIDATED by the
2026-07-10 18:26 run — the endurance soak REJECTED 1890@900 (SilentError texture-rop mid-soak, a
point the old 3-pattern gate had just passed) and the loop resynthesized to 1875@893; 3 profiles
published, all endurance-passed. STAGE 2 (directed fallback) NOT started — specced below.

## v15 (2026-07-10) — TransitionShock gate (IMPLEMENTED, validated, NOT HW-tested, NOT committed)
Motivation: the operator's in-game TDR on a forge-passing point (1860@875) presented as 4-7
`nvlddmkm` Event ID 153 "BusReset TDR" hangs ~2-3 s apart until a hard wedge — the LAUNCH-transition
failure class (idle P-state exit → boost VF ramp + VRM load step), which NO continuous workload
enters (IdlePulse sleeps only 100 ms every 750 ms — the GPU never leaves the high P-state; every
dwell also runs pre-warmed at 63-71 °C).
- gpu-stress: `VfWorkload::BoostEntry` + `VfQualifierPhase::BoostEntry` (code 12, COUNT 13):
  heavy 8-instance golden-checked slam (shares PowerRender's golden) → TRUE idle 10/20/30 s
  (`BOOST_ENTRY_GAPS_MS`, sliced 250 ms so Stop/crash stay responsive) → next slam re-enters
  through the full boost ramp. The slam wall time is the PRE-HANG detector
  (`BOOST_ENTRY_STALL_MS = 500`): a post-idle slam stalling toward the ~2 s driver watchdog sets
  `stalled` → dwell fails `Unstable` — catches the cascade's precursor without reproducing it.
  `VfQualifierPattern::TransitionShock` = BoostEntry-dominant with graceful TextureRop between
  rounds (2 phases).
- service: `F2QualificationPattern::TransitionShock` (code 6, NOT in REQUIRED, no contract bump);
  exact-Apply gate now runs BOTH candidate-only passes in order — TransitionShock
  (`F2_TRANSITION_SHOCK_DWELL_MS = 480_000`, ~8 min, fail cheap first) THEN Endurance (20 min).
- core: `point_has_current_endurance_qualification` now requires BOTH patterns run-scoped
  (shock-only or endurance-only evidence cannot publish; resume hole stays closed).
- Validation: core 81/0, gpu-stress 11/0, service 360/0; clippy baseline exact (8/19/1).
- HW gate for the next supervised run: expect `v8 TransitionShock ... shock idle→slam (8 min)`
  lines before each endurance soak; a launch-fragile candidate must fail there as `Unstable`
  (pre-hang) or SilentError — NOT as an in-game BusReset cascade later. Dwell avg/p5 clocks will
  read LOW during shock phases (deliberate idle) — that is expected, not a regression.

## Off-cap worst-case fix (2026-07-10, IMPLEMENTED after the 23:44 run) — approved "fix 1"
The 23:44 run published the KNOWN-TDR 1920@918 as Brokkr's + at-cap 1935@937 as Godforge: a cool
evening measured PowerRender 10-15 W below the previous day at identical anchors (1935@925: 176 vs
191 W), so the synthesis off-cap gate (fed only by discovery/calibration PowerRender) excluded
nothing — while the Endurance soak measured 1935@937 at p99/peak 200/200 W (AT cap) and 1920@918
peak 189 W (> 188 ceiling). FIX: new core helper
`worst_current_apply_qualification_power_at_anchor` (max p99/peak over ALL current-run validated
ApplyQualification dwells incl. Endurance/TransitionShock) raises the point's `max_power_w` when a
pair qualifies; the existing resynthesis + off-cap gate then re-evaluates against the honest worst
draw and excludes at-cap points from all profiles. Strictly conservative (only raises). Expected
next run: Godforge lands ≈1905 (last clock whose WORST measured draw clears 188 W). Tests: core
82/0, service 360/0. OPERATOR NOTE: do NOT game on the 23:44 run's Godforge/Brokkr's;
Deep Calm 1755@825 is safe.

## v16 spec — composite game-load gate (operator-approved direction, NOT implemented)
Data across all 5 logged runs: EVERY first-failure was `texture-rop` (~40×) except 2× `mixed-game`
and 1× `compute-burst`; the standalone Transitions and Memory 5-min passes NEVER rejected any
candidate at exact-Apply — 10 min/pair of redundant coverage. Operator direction: fold them into
ONE denser composite ("carga real de jogo": ~80% VRAM resident + texture hops + transitions
simultaneously) and cut the per-pair pattern block.
Proposed shape (needs contract bump 13→14 since REQUIRED changes; full re-forge anyway):
- `REQUIRED_QUALIFICATION_PATTERNS = [Texture]` (len 1 — the empirically binding detector; the
  descent per-bin 60 s Texture is unchanged; exact-Apply runs it 1×5 min).
- ENDURANCE becomes the composite: add an ~80%-VRAM-resident allocation (NVML-total-scaled,
  OOM-guarded like VramPressure) held THROUGHOUT the soak while the existing segments (sustained
  max-power, cap-slam, FrameCadence, MixedGame, TextureRop) run against it — VRAM pressure under
  worst load, which the isolated 5-min Memory pass never was. Transitions edges already covered by
  cap-slam + FrameCadence + shock.
- Pair cost: 5 (Texture) + 8 (shock) + 20 (composite endurance) ≈ 33 min +overhead, vs 43 today —
  saves ~30 min/run — while STRENGTHENING coverage (composite > isolated).
- Risks to audit: 80% VRAM alloc on smaller cards (OOM guard mandatory), golden determinism with
  concurrent VRAM residency, contract quarantine.

## Remaining v15 work (approved, NOT implemented)
1. **Runtime TDR sentinel** (service): watch System event log for `nvlddmkm` ID 153 while a
   profile is applied; on FIRST event → reset to stock + Safe Loop blacklist of the applied point
   + UI notification. Breaks the 5-strike cascade at hang #1; turns every real TDR into recorded
   evidence. Design note: poll every ~15-30 s; act only when Nidavellir state is applied.
2. **On-demand profile torture test** (IPC + UI): run TransitionShock+Endurance against the
   APPLIED profile on operator demand — covers the genuinely COLD card (~30 °C first-boot state
   the forge can never reach mid-run). New IpcRequest + Forge UI button.
3. ~~ETA cosmetics~~ DONE (2026-07-10, same session): single-source
   `F2_APPLY_PAIR_DWELL_LADDER_MS` (3×5 min + 8 + 20, + overhead/dwell) now feeds the upper
   estimate, the in-loop remaining countdown, the steps counter and the log line — ETA and gate
   read the SAME const and cannot desync. 1 pair = 2 605 s; 3 pairs = 7 815 s (~2h10).

## Publish-contract audit for v15 (2026-07-10) — one real bug found and FIXED
Full trace of the publish path with shock/endurance evidence present:
- **BUG (fixed)**: `classify_f2_stress_dwell`'s held-clock thermal rule (throttle flag + p5 sag ⇒
  Inconclusive) would misclassify EVERY TransitionShock dwell whenever NVML flagged a throttle
  (routine at ~70 °C — 3 dwells carried `throttle` in the 18:26 run), because a shock dwell is
  ~60% true-idle so p5 is an idle clock BY DESIGN. Result would have been: full run → candidate
  refused at the very end. FIX: TransitionShock is exempt from the p5-sag disqualifier (its own
  detectors — slam-stall ⇒ Unstable, golden ⇒ SilentError — carry the verdict). Regression test in
  `apply_qualification_thermal_slowdown_that_held_clock_is_not_inconclusive`.
- Verified SAFE (no change needed): p95/p99 publish helpers iterate ONLY
  `REQUIRED_QUALIFICATION_PATTERNS` via `required_pattern_index` (shock/endurance observations are
  skipped, never fatal — endurance already HW-proven by the 18:26 publish); `SingleDwell.stable`
  comes purely from the gpu-stress `StabilityResult` (no p5 heuristic); the p95-ceiling check
  can't false-fire (idle lowers p95, never raises); publish requires BOTH gates run-scoped
  (core test); no contract-version bump needed (additive tightening at Apply only).

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
