# Nidavellir — Project Memory

Honest, safety-first GPU/CPU/RAM auto-tuner for Windows. Tauri v2 + Svelte 5 UI,
Rust core service, NVIDIA-focused undervolting. ~14 K LOC source across 5 Rust
crates + a Svelte UI. Tested on RTX 3060 Ti (driver 595.97), i7-13700K, DDR4-4000.

This file is the continuity index. See also: `AGENTS.md` (canonical product/agent
governance), `architecture.md`, `decisions.md`, `roadmap.md`, `handoff.md`,
`product.md`, and the methodology doc `docs/gpu-forge.md`.

## Latest (2026-07-01) — F2 frontier and profile calibration use confirmed sustained p99 power
- Discovery keeps the existing textured `PowerRender`; the compute-only `POWER_SHADER` remains
  outside the live F2 path. Mean, sustained p99 and the highest post-ramp sample remain distinct
  through `SingleDwell`, the F2 step report, append-only observations and `PowerSweepPoint`.
- `POWER_PEAK_PERCENTILE = 99`; p99 uses every retained post-ramp power sample. Fewer than 100 samples
  fall back explicitly to the measured raw maximum; zero samples produce no value and fail closed.
- `F2_DISCOVERY_CONTRACT_VERSION = 4`. v3 positive and power-bound evidence cannot seed the new
  frontier. Adjacent-bin p99 jumps larger than both 8 W and 5% in the same p5 regime repeat the exact
  bin up to three total attempts; two must agree and the highest measured p99 is retained. No
  consensus is neutral/ineligible, never interpolated.
- Profile synthesis first applies the unchanged +12 mV policy, then resolves a current reset-clean
  PowerRender observation for that exact apply bin. It scores Godforge/Brokkr's/Deep Calm with p99
  power and the p5 clock observed at the apply bin, not boundary-bin mean or a one-sample maximum.
  When cross-clock warm-start pruning skipped that exact target/apply pair, Forge fills only the
  missing power telemetry with supervised discovery-only PowerRender and the same v4 p99 consensus;
  FSGL3 is not repeated.
- A discovery `ClockDrop` whose p99 remains at 99%+ of the numeric cap is `PowerBoundClockDrop` and
  continues voltage descent even after the clock previously sustained. It remains calibration
  telemetry, never stability evidence; off-cap `ClockDrop` retains the normal boundary behavior.
  `Validated` at cap also continues, and Standard/Long defer FSGL3 until confirmed p99 is off-cap.
  NVML software/hardware thermal slowdown makes a discovery dwell `Inconclusive`, not a bad undervolt.
- UI cards and Forge Progress show sustained p99 with an explicit “not a hard power limit” caveat;
  raw mean/maximum remain available. Apply rejects restored F2 profiles without valid p99.
  FSGL3/goldens, qualification contract v4 and Leva 1 margin/recovery semantics are unchanged.
- Hardware on 2026-07-01 confirmed p99 kept a 1950 MHz descent moving through power-bound clock
  drops while the cap stayed near 200 W. The first reset-clean FSGL3 rejection exposed a ladder
  control bug: `completed = false` stopped every lower clock. It now completes only that target and
  continues toward the real qualified Cmax; FSGL3/p5 policy itself is unchanged. Hardware rerun pending.

## Latest (2026-06-30) — F2 margin boundary, continuity and supervised recovery
- FSGL3 qualification now derives a like-for-like heavy-phase p5 signal per A/B pattern. A candidate
  becomes `ClockDrop` when that p5 falls more than `MARGIN_DROP_TOL_MHZ = 30` below the median of at
  least two prior stable candidates at the same clock/pattern, or more than 30 MHz below target.
- `Inconclusive` gets `INCONCLUSIVE_RETRY_BUDGET = 2` retries at the same point; retries use a 1.5×
  dwell. Exhaustion skips only the current clock and never becomes a global Forge abort. Hard device,
  reset, arm, apply, verify and persistence failures remain fail-closed.
- `finished` is now reserved for a complete frontier with qualified profiles. Complete Fast results
  are `provisional`; partial safe endings are `incomplete`; retained recovery is `interrupted`.
- Safe Loop classifies VIDEO TDR 0x116/0x117 as OC instability. An exact
  `f2_undervolt_probe` TDR/Unknown recovery blacklists and recedes without consuming the normal-use
  Safe Mode crash budget; unrelated crashes and non-Forge phases still count. DeviceLost is accounted
  once at startup, and duplicate blacklist regions are not appended.
- An interrupted Forge automatically performs the existing non-destructive Reset+Start sequence once
  when the UI reconnects, using the persisted original mode. Manual Stop does not create an
  auto-resume state; F2 observations remain the resume source.
- Apply policy requests `APPLY_MARGIN_MV = 12`, snaps upward to the first exact valid physical VF bin
  and clamps to the highest valid anchor. `boundary_voltage_mv` and `apply_margin_mv` are additive IPC
  fields; `vf_table_voltage_mv` remains the exact applied bin.
- `PREHANG_STALL_MS = 300` is telemetry-only in this Leva. Proactive reset remains disabled until the
  hardware gate validates signal precision and a cooperative stress cancellation path exists.
- No hardware Forge, VF write, Apply or reboot was run during implementation. Leva 2 remains blocked
  on the supervised hardware gate.

## Latest (2026-06-30) — F2 FSGL3 golden-sample qualifier
- Discovery remains the proven steady `PowerRender`; it only measures/characterizes power,
  p5-clock, cap behavior and `ClockDrop`. It no longer decides deployable stability.
- Before Standard/Long descent, stock captures one deterministic REDUCE3 golden for each render
  configuration (power, boost and texture/ROP), using a fresh `GpuCtx` per configuration. Any stock
  divergence or device loss aborts Forge; goldens are session-only and are not persisted.
- FSGL3 A/B is now the interleaved per-bin qualifier. It biases TextureRop/MixedGame, introduces
  short six-frame/4 ms droop bursts and compares every rendered frame on-GPU against the stock
  golden. FSGL1/FSGL2 remain available with their previous self-reference/250 ms behavior.
- `F2_QUALIFICATION_CONTRACT_VERSION = 4`. Apply counts only current-contract FSGL3 `Pass` evidence
  with distinct A+B patterns; FSGL1, FSGL2, discovery and old-contract positives remain provisional.
- If FSGL3 rejects a candidate, the service records that bin as unstable and keeps the last
  FSGL3-qualified physical bin as the accepted boundary. `Inconclusive` retries once and then blocks
  Apply without marking the bin bad.
- Standard/Long no longer qualify an old `prior_good` directly. Previous positives can guide resume,
  but a deployable boundary must be rediscovered by the current run before qualification begins.
- `ResetGpuTuning` is now an explicit recovery path outside the normal start/apply lease: after a
  TDR/interrupted Forge it can stop marked-running work, reset to stock, clear Safe Loop, and release
  the Forge handle. It also clears the visible `forge_state.json` checkpoint so the UI can start from
  an idle run state again, without deleting automatic F2 observation history.
- UI recovery now separates the normal path from destructive reset: post-TDR Needs Attention /
  Interrupted offers **Recover & continue** (ResetGpuTuning, then selected StartPowerSweep mode,
  preserving F2 observations) and a clearly separate **Full reset** for `ResetGpuTuningFull`.
- No IPC or frontend payload changed. No hardware Forge was run. Before the first FSGL3 run, clear
  persisted Forge state so FSGL2 floors cannot seed the trial; then verify the known 1920 MHz @ 912 mV
  and 1935 MHz @ 918 mV failure points under supervision.

## Latest (2026-06-28) — F2 qualification refinement
- Fast traverses the full physical frontier with 10 s discovery dwells but remains provisional;
  frontend and backend block F2 Apply until `profiles_qualified`.
- Standard qualifies each discovered boundary with 2 independent 60 s reset/reapply passes; Long
  uses 3×120 s. A failed qualification backs off one physical VF bin and restarts all passes.
- Frontier coverage now reaches 90% Cmax so Deep Calm has measured candidates matching its policy.
- Each lower clock starts one real VF bin above the previous minimum stable anchor; the previous
  power-bound ClockDrop is retained as fallback if that optimized warm-start is rejected.
- No hardware Forge was run for this implementation. Next step is supervised Fast/Standard QA.

## Latest (2026-06-28) — F2 durability, cross-clock reuse and live progress
- Real Fast Forge evidence proved learning was durable (72 JSONL observations) but partial progress
  was not restored visibly, and reset-clean SilentErrors incorrectly polluted the crash counter.
- Fixed crash semantics, durable partial `forge_state` checkpoints, structured ETA/progress fields,
  permanent Technical Power Sweep log, and conservative cross-clock voltage reuse.
- Deployable profiles require the complete Cmax→90% frontier plus Standard/Long qualification; partial learning remains available
  for resume and previous complete profiles are retained.
- Do not run another supervised hardware Forge automatically. Next manual validation should confirm
  1905 MHz is no longer refused after SilentError, the next clock starts near the prior boundary, and
  UI progress/log update every dwell.

## Latest (2026-06-28) — F2 integrated frontier corrected — code-complete, hardware checkpoint pending
- **Clock discovery**: the live Forge resets to stock, then starts at the highest real live-VF clock bin (1950 MHz on the
  current RTX 3060 Ti table), not a short preselected list. A pre-sustain `ClockDrop` at 99–100% of
  the numeric power cap keeps the same clock and lowers voltage; once off-cap it moves to the next
  real clock. The first reset-clean sustained target becomes Cmax.
- **Unlimited voltage discovery**: autonomous target/ladder/live Forge paths have no arbitrary 3- or
  6-step cap. They walk every physically valid VF anchor until the first silent error, instability,
  sustained-clock drop after the target has held, device loss/TDR, reset failure, cancellation, or
  the hardware floor. Explicit `--steps N` remains an operator-selected manual boundary only.
- **Complete frontier**: after Cmax, every real clock bin down through 90% of Cmax is characterized.
  Profiles are synthesized only after that complete Cmax→90% frontier exists; partial,
  cancelled, or safety-aborted runs leave the last good forge snapshot untouched.
- **Modes are evidence, not breadth**: Fast / Standard / Long traverse the same complete frontier.
  Fast = provisional 10 s discovery; Standard = 10 s discovery + 2×60 s qualification; Long =
  10 s discovery + 3×120 s qualification. Confidence comes from real dwell duration, sample count,
  and repeated evidence.
- **Learning and resume**: every candidate is appended immediately to `f2_observations.jsonl`,
  scoped by NVML GPU UUID. A restart resumes below the deepest reset-clean observation or reuses an
  existing good/bad bracket; later failures invalidate stale equal/deeper validations.
- **Safety closeout**: service-wide IPC GPU lease; arm failures stop before writes; modern VF reset is
  write/readback checked; startup recovery retains the boot flag until reapply accounting; direct
  `DeviceLost`/unconfirmed reset aborts the Forge and keeps recovery armed. F2 cap evidence remains
  diagnostic and does not make an actually sustained F2 point ineligible for profile synthesis.
- **Validation**: `cargo check --workspace`; core 64 / NVAPI 40 / service 309 tests; `git diff --check`
  clean. Read-only 1950 MHz auto-sweep dry-run now reports stock ceiling 1950 and 83 physical anchors
  (previously failed at legacy 1920/+15). No confirmed Forge, VF write, TDR attempt, apply, reboot, or
  other hardware execution was performed. Next action is the explicit supervised hardware checkpoint.

## Latest (2026-06-27) — F2 Phase 2 Apply contract closed — implemented and validated
- **Backend**: `ApplyPower*` routes F2 profiles to the anchored-undervolt writer and preserves legacy F1.
  Apply is Safe-Loop armed, verified, persisted, reapplied on boot, reversible, and fail-closed.
- **Frontend**: F2 profile Apply is enabled; Discovered yields to Active after success. Matching uses the
  deterministic target/anchor carried in the existing `GpuApplyStatus.core` point.
- **Contract evidence**: `PowerSweepPoint.confidence` / `validation_count` and
  `PowerSweepProgress.power_bound_collapse` are structured, backward-compatible payload fields.
- **Safety closeout**: a memory-offset failure after the F2 core write now resets the GPU to stock before
  returning an error. Apply/reapply also requires the exact validated VF anchor bin; a changed table fails
  closed instead of silently selecting a deeper undervolt. The legacy read-only F1 curve verifier reports
  F2 profiles as metadata-only.
- **Hardware status**: code/tests only. No Apply click, VF write, `--confirm`, reboot, or hardware run was
  performed during this closeout; one supervised manual apply remains the next operational validation.
- **Validation**: workspace `cargo check`; core 61 / NVAPI 38 / service 300 tests; UI production build;
  clippy completed with no new warnings (repository baseline warnings remain); `git diff --check` clean.

## Earlier (2026-06-27) — Forge mode split-button dropdown — implemented, committed via 9119eec
- **Frontend only**: Forge GPU / Refine Profiles is now a compact split action. The main segment
  starts the selected mode; the mode segment opens a product-styled Fast / Standard / Long dropdown.
  Standard is the initial default; modes map to `StartPowerSweepFast`, `StartPowerSweep`, and
  `StartPowerSweepLong`.
- **UX/safety**: each mode explains depth, relative duration, and confidence behavior. All copy keeps
  the run supervised and fail-closed, states that nothing is auto-applied, and leaves profile apply
  as a separate confirm-in-game step. The dropdown uses the existing forge tokens and closes on
  selection, outside click/focus, or Escape.
- **Compatibility**: stop/progress/apply paths are unchanged; the UI does not parse `note` or `log`
  for mode logic. Files: `Forge.svelte`, `RecommendedAction.svelte`, UI contract.
- **Validation**: `npm.cmd run build` and `git diff --check` passed; committed via `9119eec`.

## Earlier (2026-06-26) — multi-clock/confidence UI contract pass — implemented, pushed via e60a6f7
- **Frontend only**: Forge profile cards/progress now distinguish target vs measured/p5 clock, label
  the deterministic VF bin separately, show all 3 profile points, and surface optional Wilson
  confidence + exact-point confirmation count when the backend provides them.
- **Honest collapse**: structured `power_bound_collapse` is preferred; identical Godforge/Brokkr's
  points remain a backward-compatible fallback. The UI explicitly refuses to invent a difference.
- **IPC blocker documented**: `docs/contracts/ui-backend.md` requests optional `confidence`,
  `validation_count`, `power_bound_collapse`, and an additive start request carrying bounded
  `validation_passes`. "Build confidence now" remains unsurfaced until it can be functional.
- **Scope/validation**: no backend/Rust/IPC implementation changed. `npm.cmd run build` and
  `git diff --check` passed; merged and pushed via `e60a6f7`.

## Earlier (2026-06-23) — F2 multi-clock profile package (Brokkr's 0.95 + descending ladder + confidence opt-in) — implemented, NOT committed
- **What**: 3 approved backend changes toward the v0.5 multi-clock profile frontier. Implemented + validated +
  safety-audited (GO). No hardware run; NOT committed (awaiting operator approval).
- **Margin answer**: applied 906 vs reached 868 is the **Wilson confidence gate** (0.85), NOT a voltage margin —
  `synthesize_forge_profiles` selection is voltage-agnostic; a once-validated point (~0.21 confidence) is filtered
  until it earns repeat confirmations.
- **Part 1**: Brokkr's floor 0.98→0.95 (`ForgePolicy::balanced`; Deep Calm 0.90, gate 0.85). Selection-only.
- **Part 2 (Caminho B)**: `ladder_target_descent_bounds` — descending ladder starts each lower clock at the prior
  clock's last-good (ceiling) with the base floor; ascending unchanged.
- **Part 3**: `--validation-passes N` (default 1, cap 20) — opt-in re-validates the deepest point N-1 extra times in
  one session to earn confidence WITHOUT lowering the gate; default = no-op; idle auto-validation = future.
- **UI**: contract for Codex in `docs/contracts/ui-backend.md` (multi-clock profiles, 95% Brokkr's, honest collapse,
  confidence-gate messaging, "Build confidence now" opt-in default OFF, idle future).
- **Validation**: nvapi 38 / core 59 / service 292 pass; clippy clean; safety audit GO (8/8). No apply/persist/promote,
  no hardware, no commit. Observation store unchanged (8 records / last_good 962 mV).
- **Next**: operator approves → commit/push; then a supervised confirmed descending ladder to build the frontier.

## Earlier (2026-06-22) — F2 LEARNED OFFSET HORIZON implemented (+210 abs / +15 step); HW run HELD
- **What**: target-sweep-specific progressive absolute-offset horizon. Commit `c40a78d`
  (`feat(service): add f2 target sweep learned offset horizon`) + docs commit, pushed to `origin/master`.
- **Change**: gpu-nvapi `TARGET_SWEEP_HORIZON_MAX_MHZ = +210` + `PositiveOffsetLimits::target_sweep_learning_horizon`
  (abs +210, per-step STILL +15 — unlike `manual_prior` which widens both). Only `--auto-sweep` uses it;
  default/ladder/manual-prior keep `conservative` (+30/+15). The +210 is reachable ONLY via validated chained +15
  steps. NOT a global cap widening. 8 new tests.
- **Validation**: cargo check clean; gpu-nvapi 38 / core 59 / service 284 tests pass; clippy zero new warnings;
  independent safety audit **GO** (all 11 PASS — no unsafe clock/floor bypass; no single +210 jump; confirmed
  sweep still bounded by `F2_CONFIRMED_MAX_STEPS`=3; no profile persist).
- **MATERIAL FINDING**: today's live curve has 3 bins within +30 at the top (981/975/968), so a confirmed run
  (cap 3, descent restarts from the curve top) reaches only **968 mV** — shallower than the 962 frontier — and
  would NOT advance discovery. The +30 cap is NOT today's binding limit; the 3-step budget + descent-start is.
  The horizon correctly unblocks the PLANNER (dry-run plans 962/+45, 956/+45, 950/+60) but the confirmed run
  can't reach those without resuming the descent START near the baseline.
- **Decision — HW run HELD** (operator choice): no `--confirm`. A TDR-risk run that only re-validates known-good
  points without advancing the frontier is poor value. Observation store still 8 records / `last_good 962 mV` /
  `first_bad None`; no profile apply/persist/promotion; no Safe Loop change. Tree clean.
- **Next**: scoped, separately-reviewed follow-up so the confirmed sweep RESUMES ITS DESCENT START near the
  validated baseline (deep candidates then fall within the 3-step budget) → one supervised run advances the
  frontier. Alt: bounded LADDER over 1815/1830.

## Earlier (2026-06-22) — F2 1800 MHz second confirmed chained run; frontier saturated at +30 cap — PASS
- **What**: third confirmed official target sweep `undervolt-probe --target-mhz 1800 --auto-sweep --confirm`
  at HEAD `01b97ca` (no code change — hardware validation only). One confirmed command, operator present.
- **Result — PASS** (exit 0): **3/3 Validated**, `CompletedAllPlanned`. #1 981/+15 (1815/1815, 191 W),
  #2 975/+15 (1803/1800, 198 W), #3 968/+30 (1815/1815, 193 W). All reset + boot-flag cleared; no TDR/crash/
  DeviceLost/Unstable/ClockDrop. `first_bad None`, frontier updated, ended safe.
- **Key finding**: the 1800 MHz conservative sweep is **absolute-cap-bounded** at +30. This session's VF read
  sat higher (boost top 1935), so the deepest reachable bin was 968 mV/+30 (next needs +45 → fail-closed). The
  chained baseline relaxes only the per-step cap, never the absolute cap, so `last_good` stays **962 mV** (the
  prior run's deeper point). Re-running 1800 only adds confidence — the frontier is at its conservative floor.
- **Cleanup all correct**: `gpu_applied.json`/`boot_flag.json` absent; forge_state/gpu_knowledge/heartbeat/
  safe_loop byte-identical (no persist/apply/promote, no new blacklist); `f2_observations.jsonl` 5→8 (7
  validated + 1 preserved abort). git clean.
- **Next**: pivot to a bounded multi-target LADDER (1815/1830) for the multi-clock frontier — supervised, one
  confirmed run at a time — rather than re-running the saturated 1800 sweep.

## Earlier (2026-06-22) — F2 CHAINED DESCENT refinement + first FULL-descent HW run (1800 @ 962 mV) — PASS
- **What**: implemented observation-aware chained same-target descent (commit `fcdf04d`), then ran the first
  confirmed sweep with it: `undervolt-probe --target-mhz 1800 --auto-sweep --confirm`. One confirmed command,
  operator present, no second run.
- **Fix**: the confirmed motor bounds each candidate's per-step increase against the LAST VALIDATED offset
  (prior candidate this run, only reached after it validated; or the deepest prior validated same-target/
  same-GPU observation for candidate 0; 0 when none) instead of stock +0. The absolute +30 cap still bounds
  each candidate. `validated_descent_baseline` (core) + `chained_prev_offset` (service); gpu-nvapi writer,
  `apply_vf_ceiling_monotone`, verifier, and manual-prior (+250) cap UNCHANGED. A no-write `AbortedBySafetyGate`
  record is never a baseline/first_bad/blacklist, so the prior 968/+30 abort does not block replanning.
- **Result — PASS** (exit 0): **3/3 Validated**, `CompletedAllPlanned`. #1 975/+15 (avg 1803/p5 1770, 198 W),
  #2 968/+15 (1800/1800, 190 W), #3 **962/+30** (1800/1800, 191 W) — the +30 point that aborted in the
  PASS-PARTIAL run now validates. New min stable voltage **962 mV** (was 975), `first_bad None`, frontier
  updated. No TDR/DeviceLost/Unstable/ClockDrop.
- **Cleanup all correct**: reset + boot-flag cleared for all 3; `gpu_applied.json`/`boot_flag.json` absent
  after; forge_state/gpu_knowledge/heartbeat/safe_loop byte-identical (no persist/apply/promote, no new
  blacklist). `f2_observations.jsonl` 2→5 (prior 2 incl. old abort preserved). git clean. Tests: core 59,
  service 282, gpu-nvapi 33 — all green.

## Earlier (2026-06-22) — F2 OFFICIAL target sweep FIRST HARDWARE RUN (1800 @ 975 mV) — PASS-PARTIAL
- **What**: first bounded hardware run of the OFFICIAL F2 target sweep (progressive anchored descent, NOT
  manual-prior): `undervolt-probe --target-mhz 1800 --auto-sweep --confirm` at HEAD `8dbd296`. One confirmed
  command, operator present, no second run.
- **Result — PASS-PARTIAL** (exit 0): #1 **Validated** 975 mV / base 1785 / +15 → 1800; **RaiseVerified**;
  dwell **Stable** avg/p5 **1815 MHz**, **191 W**, no silent error. #2 **aborted_by_safety_gate** (planner
  per-step +30 > +15 cap; **no VF write**, `not_run`). `last_good=975 mV`, `first_bad=None`, frontier updated.
  No TDR/DeviceLost/Unstable/ClockDrop/reboot.
- **Cleanup all correct**: reset_to_stock + boot_flag_cleared true for both; `gpu_applied.json`/`boot_flag.json`
  absent after; forge_state/gpu_knowledge/heartbeat byte-identical; safe_loop unchanged (safe_mode=false). 2
  observations now in `f2_observations.jsonl` (first official observation file). No profile persisted/applied/
  promoted. git clean.
- **Algorithm insight (not changed this task)**: candidates restart from stock (+0); the +15 per-step cap
  makes only the base-within-+15 anchor (1785→+15) reachable, so deeper anchors self-abort and the 1800 sweep
  validates ONE point per run. Bracketing below 975 mV needs a planner that carries the prior validated offset
  forward (same-target descent) — a future reviewed refinement, not this run.

## Earlier (2026-06-22) — F2 discovery/learning algorithm IMPLEMENTED (not yet HW-validated)
- **What**: the four-block F2 discovery/learning algorithm. **Code + tests + docs only — no hardware, no
  `--confirm`, no VF write, no profile apply/persist/promote.** Commits `0df6179` (store + target sweep),
  `cb125b6` (ladder + learned frontier).
- **Block 1 (observation store)**: `crates/core/src/f2_observation.rs` — `F2Observation` + append-only JSONL
  `F2ObservationStore` at `default_data_dir()/f2_observations.jsonl` (learning data, NOT a profile). Pure
  queries: last_good (lowest validated), first_bad (highest failure), bracket (Vmin in (first_bad,
  last_good]), is_known_bad, learned_frontier.
- **Block 2 (target sweep)**: `undervolt-probe --auto-sweep` — autonomous same-target min-stable-voltage
  discovery via the OFFICIAL progressive anchored descent (conservative +30/+15 caps, NOT manual-prior);
  bounded by F2_CONFIRMED_MAX_STEPS; records one observation per candidate on --confirm only.
- **Block 3 (ladder sweep)**: `--ladder-sweep --targets a,b,c` — per-target sweeps in order; a lower
  target's last-good is a conservative FLOOR only (never assumed to hold a higher clock); stops the ladder
  on a safety failure (ResetFailed/crash). A normal bad candidate stops only that target.
- **Block 4 (learned frontier + bridge)**: `learned_frontier` → per-target `F2FrontierEntry`;
  `to_power_sweep_point` builds the canonical `(PowerSweepPoint, conf)`; `classify_f2_frontier_summary`
  runs the EXISTING `synthesize_forge_profiles` (balanced) READ-ONLY to preview Godforge/Brokkr's/Deep
  Calm — no new scoring, nothing applied/persisted/promoted.
- **Untouched**: default progressive + manual-prior; F1/build-frontier; apply_vf_ceiling_monotone; Safe
  Loop; reset_to_stock; verifier; synthesize_forge_profiles. v1 GPU-only; CPU/RAM/UI deferred. Instability
  that resets clean is learning data, not a safety failure.
- **Validated (no HW)**: core 56/0, service 278/0, nvapi 33/0; clippy clean; dry-runs write nothing
  (`f2_observations.jsonl`/`boot_flag.json`/`gpu_applied.json` absent). **NEXT**: first bounded hardware
  run of the official target sweep — `undervolt-probe --target-mhz 1800 --auto-sweep --confirm` (operator
  present); not another manual validation.

## Earlier (2026-06-21) — F2 MANUAL-PRIOR anchor mode HARDWARE VALIDATED (1800 @ 875 mV, +210) — PASS
- **What**: opt-in `--manual-prior` for `undervolt-probe` — anchor at an explicit `--start-mv` with a
  SEPARATE larger bounded offset cap, to validate a KNOWN point fast (`1800 MHz @ 875 mV`). NOT the
  default, NOT for unknown GPUs. **Code + tests + docs only — no hardware, no `--confirm`, no VF write.**
- **Default unchanged**: progressive anchored descent + conservative caps (+30/+15) remain the official
  unknown-GPU path; manual-prior branches BEFORE the default dispatch (gated on `args.manual_prior`).
- **Cap**: `F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ = 250` (default +30 untouched); fail-closed (offset
  above cap REFUSED, never clamped; stock clock ceiling still caps effective clock). Gate: `--manual-prior`
  requires `--start-mv`; confirmed requires `--steps 1`; reuses `run_confirmed_f2_step`/`RealF2Ops` with
  manual limits; one candidate; no persist/apply/promote. F1/`apply_vf_ceiling_monotone`/Safe Loop/reset/
  verifier untouched.
- **Dry-run `1800 @ 875`**: selected 875 mV, base 1590 MHz, required +210 MHz, cap +250, within bounds,
  AnchoredRaiseVerified, no-op/no-write. Default `1800 --steps 3` unchanged (975/968/962). 269 service + 33
  nvapi tests pass; manual safety review no blockers.
- **HARDWARE PASS (one confirmed run, operator present)**: `undervolt-probe --target-mhz 1800 --start-mv
  875 --steps 1 --manual-prior --confirm` → exit 0, **Validated**. Anchor 875 mV / base 1590 / +210 → 1800;
  **AnchoredRaiseVerified**; dwell **Stable** avg/p5 **1815 MHz**, **157 W** (~26 W under the 975 mV/183 W
  run — same clock, lower voltage); reset_to_stock OK (all bins cleared); boot flag cleared; not
  blacklisted; **no persist/apply/promote** (`last_validated` null). No TDR/crash/reboot; `safe_loop.json`
  byte-identical (mtime-only); `boot_flag.json`/`gpu_applied.json` absent. Impl commit `34581d0`.
- **NEXT**: clocks above 1800 at 875 mV NOT assumed (discover progressively). Either descend below 875 mV
  for 1800 (min stable voltage) or progressive discovery for 1815+. No second confirmed run made.

## Earlier (2026-06-21) — F2 ANCHORED multi-step descent IMPLEMENTED (not yet HW-validated)
- **What**: bounded SAME-TARGET ANCHORED multi-step descent for `undervolt-probe`. `--steps 2..=3` (anchored)
  runs a short sequence of anchored candidates at ONE target, safer/higher voltage → lower voltage, stopping
  at the first non-stable candidate and keeping the last good point. **Code + tests + docs only — no hardware,
  no `--confirm`, no VF write, no Safe Loop mutation outside tests.** Files: `gpu_undervolt.rs` (+ 12 tests),
  `main.rs` (doc comment only).
- **Cap**: `F2_CONFIRMED_MAX_STEPS = 3`, enforced by `confirmed_f2_multi_refusal` (`--steps` 1..=3 else fail
  closed). `--steps 1` keeps the validated single-step path; `--simple` stays single-step.
- **Design**: `plan_anchored_undervolt_descent` (anchored analog of `plan_undervolt_probe`, chains the +15
  per-step cap, stops at first rejection) → `run_confirmed_f2_multi_step` drives the SAME validated
  `run_confirmed_f2_step` motor per candidate via the `F2MultiStepOps` cursor trait (`select(i)` re-checks
  Safe Loop + blacklist before each write). Continues only on stable `Validated`; stops on VerifierFailed/
  Unstable/DeviceLost/ClockDrop/ResetFailed/Blacklisted. New `F2DwellOutcome::ClockDrop` (p5 < target − 30 MHz
  on an otherwise-stable dwell) — additive; single-step Stable/Unstable/DeviceLost unchanged.
- **Validated (no HW)**: 256 service + 33 nvapi tests pass (incl. F1/build-frontier + single-step). Dry-run
  `--target-mhz 1800 --steps 3` → 3 candidates (975 mV +15 → 968 mV +30 → 962 mV +30, stop=budget), preflight
  OK, no-op line; `--help`/`--steps 1` unchanged. **NEXT HW (one confirmed run, operator, stop after first
  non-stable)**: `undervolt-probe --target-mhz 1800 --steps 3 --confirm`. No persist/apply/promote.

## Earlier (2026-06-21) — F2 ANCHORED undervolt FIRST confirmed hardware validation (1800 MHz @ 975 mV, +15) — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second) of the `747a11b` anchored
  branch: `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. **First real ANCHORED positive-offset VF write.**
  HEAD = origin/master = `747a11b`, tree clean; fresh worktree binary built first (was absent; mtime > `747a11b`).
- **Preflight PASS**: `gpu_applied.json`/`boot_flag.json` absent; `safe_mode=false`; `boot_flag_armed=false`;
  `consecutive_crashes=1`; anchored point NOT blacklisted. Help = usage only; dry-run = mode ANCHORED, exactly 1
  candidate + no-op line.
- **Result: exit 0, `Validated`.** No TDR/black-screen/reboot/DeviceLost/Unstable/silent-error. Candidate: target
  **1800 MHz**, anchor bin **975 mV**, base **1785 MHz**, **+15 MHz**; **27** bins capped to 1800 (max -150), **59**
  elastic (within +15 step / +30 abs caps).
- **Motor end-to-end**: Safe Loop armed BEFORE write → `apply_bounded_anchored_positive_offset` →
  `verify_anchored_positive_offset` = **`AnchoredRaiseVerified`** → dwell **Stable** (avg **1815 MHz**, p5 **1815 MHz**,
  **183 W**) → `reset_to_stock` ran + confirmed stock (all written bins cleared) → boot flag cleared after clean reset.
  Not blacklisted; **no profile persisted/applied/promoted** (Validated reported only).
- **Post-run**: `boot_flag.json`/`gpu_applied.json` absent; `safe_loop.json` byte-identical (mtime-only);
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; tree clean; HEAD `747a11b`.
- **Boost constrained vs prior simple F2**: simple boosted above target (avg **1868**, p5 **1845**, **199 W**);
  anchored pins a flat plateau (avg **1815** = p5 **1815**, **183 W**, ~**16 W** lower) → caps prevent boost above
  1800; +15 over target is within the 15 MHz verifier tolerance.
- **Meaning**: F2 ANCHORED-undervolt hardware path PROVEN at one bounded point — classic `MHz @ mV` SHAPE (anchor
  raise + plateau cap + elastic lower bins) holds + the **arm → write → verify → dwell → reset → clear** motor is
  recoverable. First direct support for the method (map stable voltage per clock → repeat → synthesize Godforge /
  Brokkr's Best / Deep Calm). Does NOT yet prove the MINIMUM stable voltage for 1800 MHz.
- **Next (don't re-run a confirmed command yet)**: bounded, supervised, same-target **MULTI-STEP** anchored probe at
  1800 MHz descending voltage until verifier fail / instability / clock drop / floor / budget. Detail in
  `decisions.md` / `handoff.md` (top entries).

## Latest (2026-06-21) — F2 ANCHORED undervolt planning IMPLEMENTED (no hardware)
- **What**: F2 now plans a true CLASSIC anchored undervolt point — RAISE the anchor bin to target AND
  CAP every higher-voltage bin DOWN to the same target (≤ 0 offsets), lower bins elastic. **ANCHORED is
  the DEFAULT** mode; `--simple` keeps the old single-bin descent. Code + tests + docs only — **no
  `--confirm`, no VF write, no Safe Loop mutation, no build-frontier/stress/sweep.**
- **Why**: the prior confirmed run proved the positive-offset MOTOR but was not anchored — the GPU still
  boosted ABOVE the 1800 MHz target. Classic `MHz @ mV` undervolt must test an anchored curve point.
- **New symbols** (SEPARATE from F1; `apply_vf_ceiling_monotone` UNTOUCHED):
  `plan_bounded_anchored_positive_offset` / `apply_bounded_anchored_positive_offset` /
  `AnchoredPositiveOffsetPlan` (gpu-nvapi — anchor reuses the bounded single-bin planner →inherits all
  bounds); `verify_anchored_positive_offset` / `AnchoredOffsetVerification::AnchoredRaiseVerified`
  (gpu_verify); `UndervoltMode` / `plan_anchored_undervolt` / `anchored_plan_lines` (gpu_undervolt).
- **Confirmed branch (anchored, NOT executed)**: ONE anchored curve plan, single-step (`--steps 1`), arms
  Safe Loop before write, resets on every post-arm exit, clears boot flag only after a confirmed reset,
  confirms ALL written bins read ~0, no persistence/apply/promotion.
- **Validation**: `cargo check` clean; service **240** tests + gpu-nvapi **33** tests pass (F1 + simple-F2
  still green). Read-only dry-run (`--target-mhz 1800 --steps 1`): anchor **981 mV/1785 +15 → 1800**, **25**
  bins capped to 1800 (max -135), **61** elastic, self-check `AnchoredRaiseVerified`, no write.
- **Hardware NOT validated for anchored mode.** First future anchored run: `--target-mhz 1800 --steps 1
  --confirm` — ONE candidate, operator present, NOT multi-step. Detail in `decisions.md`/`handoff.md` (top).

## (2026-06-21) — F2 true-undervolt FIRST confirmed hardware validation (1800 MHz @ 981 mV, +15) — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second) of the `78ecfc7` F2
  branch: `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. **First real positive-offset VF write.**
  HEAD = origin/master = `78ecfc7`, tree clean; fresh worktree binary built first (was absent; mtime > `78ecfc7`).
- **Preflight PASS**: `gpu_applied.json`/`boot_flag.json` absent; `safe_mode=false`; `boot_flag_armed=false`;
  `consecutive_crashes=1`; point NOT blacklisted (`blacklisted_points=0`). Help = usage only; dry-run = exactly 1
  candidate + no-op line.
- **Result: exit 0, `Validated`.** No TDR/black-screen/reboot/DeviceLost/Unstable/silent-error. Candidate: target
  **1800 MHz**, bin **981 mV**, base **1785 MHz**, **+15 MHz** (within +15 step / +30 abs caps).
- **Motor end-to-end**: Safe Loop armed BEFORE write → `apply_bounded_positive_offset` → `verify_positive_offset`
  = **`RaiseVerified`** → dwell **Stable** (avg **1868 MHz**, p5 **1845 MHz**, **199 W**) → `reset_to_stock` ran +
  confirmed stock → boot flag cleared after clean reset. Not blacklisted; **no profile persisted/applied/promoted**
  (Validated reported only, no `last_validated` write).
- **Post-run**: `boot_flag.json`/`gpu_applied.json` absent; `safe_loop.json` byte-identical (mtime-only);
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; tree clean; HEAD `78ecfc7`.
- **Meaning**: F2 hardware path PROVEN at one bounded positive-offset point — **arm → write → verify → dwell →
  reset → clear** is viable + recoverable. Does NOT prove an optimal profile (minimum-viable only). Dwell clock
  above 1800 MHz (1868 avg) is EXPECTED — probe doesn't lock the clock; GPU still boosts per curve/power.
- **Next (don't re-run a confirmed command yet)**: (1) bounded F2 MULTI-STEP for the same target, (2) explicit
  `--start-mv` confirmed single-step if unsupported, or (3) result recording / Forge Knowledge for validated F2
  candidates without promotion. First optimization = search the lower-voltage limit around 1800 MHz with the same
  Safe Loop / verification / reset guarantees. Detail in `decisions.md` / `handoff.md` (top entries).

## (2026-06-20) — F2 CONFIRMED single-step branch IMPLEMENTED, not executed (no hardware)
- **First real confirmed F2 branch** (`undervolt-probe --confirm`): single-target, single-step only.
  IMPLEMENTED but NOT run — no `--confirm`, no VF write, no Safe Loop mutation this task.
- **State machine** (`gpu_undervolt.rs`, trait-isolated + mock-tested `run_confirmed_f2_step`/`F2Ops`):
  arm boot flag → apply ONE bounded positive offset → verify (offset-presence, idle freq=None) → dwell →
  `reset_to_stock` on EVERY exit → clear flag ONLY after a CONFIRMED reset (real reset re-reads the bin and
  fails closed if not ~0). DeviceLost/reset-fail RETAIN the flag; DeviceLost/Unstable blacklist; only
  Stable+confirmed-reset → Validated (reported only, no `last_validated` write). No persist/apply/promote.
- **Preflight** `confirmed_f2_refusal`: requires `--steps 1`; refuses Safe Mode / armed flag /
  consecutive_crashes ≥ 3 / no candidate / out-of-bounds / blacklisted (3-axis + 2-axis). `--confirm` runs
  startup recovery first.
- **Help fixed**: `--help`/`-h` prints usage + `--confirm` may-write-VF/operator warning; no hardware read.
- **F1 untouched**: `apply_vf_ceiling_monotone` + build-frontier unchanged; `gpu_power_sweep.rs` edits are
  additive/visibility (`reset_to_stock` pub(crate); `single_load_dwell`/`SingleDwell` reuse
  `load_and_measure`). Dry-run output unchanged except footer + help.
- **Validation**: `cargo check` clean; service tests **228/0** (+15); gpu-nvapi **25/0**; dry-run + help
  read-only. **Hardware NOT validated.** First future run: `undervolt-probe --target-mhz 1800 --steps 1
  --confirm` (operator present, one run). Detail in `decisions.md`/`handoff.md` (top entries).

## Checkpoint (2026-06-20) — F2 true-undervolt foundation IMPLEMENTED (pure, no hardware)
- **First isolated F2 path** for TRUE undervolt: bounded POSITIVE VF offsets (raise a lower-voltage bin to hold
  the target clock) — the opposite of F1/build-frontier flatten-down. F1 stays intact; F2 gets its own symbols.
- **gpu-nvapi**: pure `plan_bounded_positive_offset` + windows `apply_bounded_positive_offset`;
  `PositiveOffsetPlan`/`PositiveOffsetLimits`; consts `POS_OFFSET_MAX_MHZ=+30`, `POS_OFFSET_STEP_MAX_MHZ=+15`.
  `apply_vf_ceiling_monotone` NOT touched.
- **gpu_verify**: pure `verify_positive_offset` → `PositiveOffsetVerification` (RaiseVerified/RaiseIncomplete/
  OverRaise/Unverifiable); intended raise is the success case (no flatten-down overshoot veto); flatten-down
  verifier unchanged.
- **gpu_undervolt.rs (NEW)**: pure `plan_undervolt_probe` (descend real bins, compute bounded offset to hold
  target, stop at first bound/floor violation) + pure `undervolt_preflight` (Safe Loop read-only refusal) +
  windows `run_undervolt_probe` (dry-run; `--confirm` fails closed). CLI: `undervolt-probe` (`--target-mhz`,
  `--start-mv`, `--steps`; `--confirm` parsed but REFUSED this task).
- **Fail-closed**: empty/foreign/non-sane base, non-real bin, below floor, offset≤0, offset>+30, step>+15,
  clock>ceiling all → explicit Err (never clamps); bounds are constants, not CLI-widenable.
- **NOT touched**: F1 flatten-down writer/verifier, Safe Loop, boot flag, reset_to_stock, blacklist,
  last-known-good, power-limit/TDP/clock-lock. No persist/apply/promote, no multi-target loop, no crash-seeking.
- **Hardware BLOCKED.** Next: dry-run review of `undervolt-probe`, THEN a first supervised one-step confirmed
  F2 validation. Detail in `decisions.md`/`handoff.md` (top entries).

## Checkpoint (2026-06-20) — F1c bounded-tail confirmed PASS + tail-richness follow-up
- **Confirmed run (2026-06-20) of the bounded tail (`8667bf0`) = PASS**. Safety clean (exit 0, no
  TDR/crash/reboot, reset_to_stock ran, no persist/apply, state byte-identical, monotone positive_offsets=0).
  Phase B focus 1800, started 1056 mV (below 1062 floor), crossed knee (pcf 1.000@1012 → **0.215@1006 mV**),
  **continued past the first off-cap point to 1000 mV, captured 2 useful points**, `KneeTailComplete`;
  **synthesis became `differentiated`** (was collapse).
- **Remaining issue**: both tail points ~199 W → Godforge/Brokkr's/Deep Calm coincided (~1811 MHz/1006 mV/199 W).
  Differentiated but THIN.
- **Follow-up (2026-06-20)**: enrich the tail — `PHASE_B_MIN_USEFUL_POINTS` 2→**4**,
  `PHASE_B_POST_KNEE_TAIL_BINS` 3→**5** (synthesis collapse threshold `MIN_USEFUL_FRONTIER_POINTS` stays 2).
  Bounded: 4 useful OR 5 post-knee bins; opt-in/default OFF; no new CLI flag; `--phase-b-probes`/global
  `--max-probes` bound it; failure/verifier/instability/floor/budget keep precedence.
- **Unchanged**: Phase A, synthesis, bind-seeking, safety chain. File: `gpu_power_sweep.rs` only.
- **Hardware**: one confirmed validation authorized (same flags) to test whether power drops below the knee and
  the three profiles separate. Detail in `decisions.md`/`handoff.md` (top entries).

## Checkpoint (2026-06-16) — F1c follow-up: Phase B captures a bounded below-knee TAIL (commit 8667bf0) — pure, no hardware
- **Driver**: FIRST confirmed knee-seeking run (2026-06-16) = **PASS-PARTIAL**. Found the real knee at
  **~1025 mV** (Phase B started 1056 mV, below the 1062 Phase-A floor; pcf dropped **1.000→0.437 in one 6 mV
  bin** — steep knee). Safety PASS (exit 0, no TDR/crash/reboot, reset_to_stock ran, no persist/apply, state
  byte-identical, monotone writer positive_offsets=0). But Phase B stopped at the FIRST off-cap point → only
  **1** useful point → synthesis correctly still `POWER-BOUND COLLAPSE`. Stop policy, not budget, was the limit.
- **What landed**: `descend_phase_b` now captures a BOUNDED below-knee tail. After the knee crossing (first
  `pcf < POWER_BOUND_FRAC` point) it keeps descending until `PHASE_B_MIN_USEFUL_POINTS` (=2) useful off-cap
  points OR `PHASE_B_POST_KNEE_TAIL_BINS` (=3) post-knee bins, then stops cleanly as new
  `BracketStop::KneeTailComplete`. ≥2 useful → existing synthesis differentiates; 1 → honest collapse.
- **Safety precedence preserved**: crash/abort/global-drain/verifier-fail/instability are checked BEFORE the
  tail and stop immediately; floor / `--phase-b-probes` / global `--max-probes` still bound it.
- **Unchanged**: Phase A, synthesis, bind-seeking, safety chain (writer/verifier/Safe Loop/reset_to_stock/
  floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; no new CLI flag. File:
  `crates/service/src/gpu_power_sweep.rs` only.
- **Validation**: `cargo check` clean; `cargo test -p nidavellir-service` **203 / 0** (8 new). No hardware.
- **Hardware STILL BLOCKED**. Next: NEW dry-run-only review of the bounded-tail plan. Detail in
  `decisions.md` / `handoff.md` (top entries).

## Checkpoint (2026-06-16) — F1c follow-up: Phase B continues BELOW Phase-A floor (commit 9f35ec0) — pure, no hardware
- **What**: budget-efficiency fix for F1c Phase B (dry-run-review finding). Phase B now CONTINUES below the
  deepest bin Phase A already explored for the focused target, instead of re-probing the inert top bins.
  File: `crates/service/src/gpu_power_sweep.rs` only. Pure: no hardware, no `--confirm`.
- **Why**: fine VF curve (~6–7 mV/bin) — `0ef4e68` Phase B re-started from the cap, so `--phase-b-probes 12`
  reached only ~1006 mV (re-covered 1075/1068/1062), ~75 mV above the ~930 mV knee. Now each probe is a new,
  deeper bin.
- **How**: pure helpers `phase_a_deepest_bin` (focus target's deepest retained Phase-A bin) +
  `phase_b_start_below` (highest real bin strictly below it) → Phase-B start. Fallbacks: no Phase-A history
  → safe-start cap; Phase A at the floor → Phase B skipped cleanly. Dry-run plan gains a `knee start` line.
- **Unchanged**: Phase A, `descend_phase_b`, synthesis, safety chain (writer/verifier/Safe Loop/
  reset_to_stock/floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; global
  `--max-probes` master cap.
- **Validation**: `cargo check` clean; `cargo test -p nidavellir-service` **195 / 0** (5 new). No hardware.
- **Hardware STILL BLOCKED**. Next: NEW dry-run-only review of the improved plan. Budget sizing still the
  operator's call (~20+ Phase-B probes to cross a ~930 mV knee from a ~1062 mV floor); default budget
  unchanged (12). Detail in `decisions.md` / `handoff.md` (top entries).

## Checkpoint (2026-06-15) — F1c power-bound knee-seeking two-phase prototype IMPLEMENTED (commit 0ef4e68) — pure, no hardware
- **What landed**: OPT-IN (default OFF) two-phase power-bound knee-seeking for `build-frontier` — the
  design-audit direction `NEED DEEPER POWER-BOUND DESCENT`. Files: `crates/service/src/gpu_power_sweep.rs`
  + `crates/service/src/main.rs` (2 CLI flags). Pure: no hardware, no `--confirm`, no dry-run, no VF write.
- **Why shallow collapse ≠ terminal**: the validated `0996769` run only walked the top ~13 mV (bins
  `1075/1068/1062`), ~130 mV above the card's ~930 mV operating voltage, so the VF ceiling was INERT and
  pcf stayed 1.000 — honest diagnostic for a SHALLOW descent, not proof no frontier exists.
- **Phase A** = the existing broad/shallow single-pass descent, extracted VERBATIM into
  `run_target_descents` → byte-for-byte unchanged when OFF. **Phase B** runs ONLY after a Phase-A
  power-bound collapse AND the opt-in is set: `detect_plateau_clock` (median power-bound clock) →
  `select_phase_b_target` (lowest candidate ≥ plateau) → `descend_phase_b` (deep descent on ONE focused
  target through real VF bins, bounded budget, full trajectory) → `detect_power_bound_knee` (first pcf
  crossing below 0.95). Merge + re-synthesize via existing `synthesize_forge_profiles`.
- **Knee mental model**: above-knee `pcf ≥ 0.95` (ceiling inert — keep descending); knee = first pcf drop
  < 0.95; clean deep stop at `pcf ≤ 0.50`; below-knee tail → Brokkr's/Deep Calm; Godforge = highest
  sustained off-cap clock (knee region), NOT highest requested clock. No knee ⇒ honest `PowerBoundCollapse`
  preserved.
- **Flags**: `--power-bound-knee-seeking` (default OFF) + `--phase-b-probes N` (default None → 12).
  Global `--max-probes` stays the MASTER cap; Phase-B budget only bounds the focused descent depth;
  `--phase-b-probes 0` fails closed.
- **Safety surfaces UNCHANGED**: VF (monotone static-base) writer, verifier gates, Safe Loop,
  `reset_to_stock` (runs after every build, both paths), floor/cluster derivation, per-target cap,
  warm-start default OFF, persistence/knowledge writes, power-limit/TDP/clock-lock.
- **Validation**: `cargo check -p nidavellir-service` clean (0 warnings); `cargo test -p
  nidavellir-service` **190 passed / 0 failed** (17 new). No hardware run.
- **Hardware STILL BLOCKED.** Next: SEPARATE dry-run-only review of the new opt-in `--power-bound-knee-seeking`
  plan output (no `--confirm`); no confirmed run until that review; no same-config rerun. Detail in
  `decisions.md` / `handoff.md` (top entries).

## Checkpoint (2026-06-15) — F1b power-bound collapse classification FIRST CONFIRMED HARDWARE VALIDATION (commit 0996769) — PASS
- **One supervised confirmed run** validating `0996769` (docs `4880153`); HEAD = origin/master = `4880153`,
  tree clean; fresh worktree binary. Dry-run gate passed first. `build-frontier --confirm --max-targets 7
  --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`. **Exit 0; ~5.7 min.**
- **Safety PASS**: no TDR/crash/reset/reboot; `reset_to_stock` ran; GPU back at stock/idle. After:
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/disarmed (mtime-only change);
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; tree clean. Every probe
  `write_mode=monotone_static`, `positive_offsets=0`; no overshoot veto.
- **Mechanics**: 19 probes / 17 dwells; `--max-probes 21` not exhausted; 6/7 targets characterized (1920 dropped
  on benign verifier `LiveMismatch`, run-variance; 1890 later LiveMismatch kept deepest verified). All dwells
  PowerLimited, `power_capped_frac=1.000`, ~199 W, ~1784–1825 MHz.
- **Reporting honesty PASS**: no `BoundBinding`, no `reason=Clock`. **Clock-arm retirement validated** (probes
  that would false-bind under v2 avg-clock did NOT bind → `PerTargetCap`). **`LeftPowerRegime` validated
  negatively** (no false-fire; no target had pcf ≤ 0.50 so none stopped by it). **`PowerBound`/collapse positive**:
  6 `[power-bound]` / 0 useful; explicit *"power-bound collapse — cannot build a differentiated VF frontier under
  this workload/regime"*; Godforge/Brokkr's/Deep Calm collapsed to one best-effort point (1815 MHz/199 W, R=0.00,
  conf 0.21), flagged not-differentiated — no fake frontier.
- **Verdict PASS** (safety + honesty). Frontier still not useful: card pinned at ~199 W cap, now reported
  honestly. **Caveat**: `LeftPowerRegime` validated negatively only (positive stop needs pcf ≤ 0.50). **Next**:
  accept patch; **keep hardware BLOCKED for this config**; don't repeat the run / bump per-target cap / touch
  power-limit yet. Design decision next: non-cap-saturating workload, targets below the power-bound plateau, or a
  "cannot differentiate" presentation pass. See `decisions.md` + `handoff.md`.

## (2026-06-15) — F1b power-bound collapse classification IMPLEMENTED (commit 0996769) — pure, no hardware
- **Commit `0996769 fix(service): classify power-bound frontier collapse`** (pushed to `origin/master`). Scope:
  `crates/service/src/gpu_power_sweep.rs` ONLY. The SIMPLIFY patch from the audit below. No hardware.
- **Retired bind-seeking's Clock arm** → `classify_binding` regime-only: bind (stop early) ONLY on leaving the
  power-limited regime (`power_capped_frac <= 0.50`). Removed `BIND_OVERSHOOT_MHZ` / `overshoot_mhz` /
  `BindReason::Clock`; start-bin guard kept. Renamed `BracketStop::BoundBinding → LeftPowerRegime`.
- **Power-bound classification** (`POWER_BOUND_FRAC = 0.95`, pure helpers `is_power_bound_frac/_point`,
  `useful_frontier_points`, `frontier_power_bound_collapse`): a pcf-saturated stable dwell is a valid raw
  bracket but NOT useful clock-frontier diversity; invalid/missing pcf → not power-bound (fail open), still
  fail-closed for regime binding.
- **Collapse-aware synthesis**: `synthesize_forge_profiles` excludes power-bound points; < 2 useful → flagged
  best-effort + "power-bound collapse — cannot build a differentiated VF frontier…" (new
  `ForgeProfiles.power_bound_excluded` / `power_bound_collapse`). Catches jittery ~1798–1819 MHz @ pcf 1.0 that
  exact-distinct-clock missed. No power-bound points → legacy path unchanged. RESULT prints per-point pcf +
  `frontier classes` summary.
- **Unchanged safety surfaces**: writer, verifier, Safe Loop, reset_to_stock, floor/cluster, per-target cap,
  warm-start default OFF, persistence/knowledge, power-limit/clock-lock. `cargo check` clean; `cargo test`
  **173 passed**. **Hardware STILL BLOCKED** (pure patch; review new diagnostics in a dry-run before any run).
  See `handoff.md` + `decisions.md`.

## (2026-06-15) — build-frontier / F1b algorithm audit — verdict SIMPLIFY (read-only, pre-implementation)
- **Read-only audit** of `crates/service/src/gpu_power_sweep.rs` + continuity docs. **No code/tests/hardware/
  `--confirm`/VF-write/stress/power-sweep** run. Recorded BEFORE implementation to set the next patch's north
  star. Full rationale: `decisions.md` (top) + `handoff.md` (Latest backend checkpoint).
- **Verdict: SIMPLIFY CURRENT DIRECTION** — not a redesign, not a full rollback. Don't run more hardware before
  the next pure/pure-ish patch; don't keep adding bind-seeking complexity. The discovery → descent → synthesis
  skeleton is still valid; the drift is concentrated in **bind-seeking / `BoundBinding`** semantics.
- **Bind-seeking conclusion**: `BoundBinding` is the wrong combined abstraction — a **bad Clock arm**
  (false-binds under power cap) + a **useful Regime arm** (`pcf <= 0.5`). The v2 start-bin guard was useful +
  validated but did NOT fix physical frontier collapse: the confirmed v2 run (`bf02971`) stayed power-limited
  (`power_capped_frac=1.000`, ~199 W, ~1798–1819 MHz, confidence 0.21, profiles collapsed). ⇒ remaining issue =
  **regime/power-bound collapse, not scheduler depth or per-target probe count.**
- **Decision**: retire/neutralize the Clock arm; keep the regime signal as **`LeftPowerRegime`**; add a
  first-class **`PowerBound`/`PowerBoundCollapse`** classification; strengthen `synthesize_forge_profiles` to
  detect the pcf-saturated plateau (today it keys on exact-distinct clocks, so jittery ~1800 MHz reads as
  "differentiated" and the warning never fires) and emit *"power-bound collapse — cannot build a differentiated
  VF frontier under this workload/regime."* Power-limited samples = valid bracket, **not** useful clock-frontier
  diversity; raw synthesis input but filtered out of differentiated selection; primary collapse signal.
- **KEEP (load-bearing)**: hardware-derived floor; cluster selection / sane-core filtering; real-bin descent;
  per-target cap; typed hard/soft stops; confidence gate + best-effort fallback; monotone writer; verifier
  gates; Safe Loop; `reset_to_stock`; no persistence during build-frontier.
- **Non-goals / hardware**: pure-ish patch in `gpu_power_sweep.rs` + synthetic-sample tests only; no hardware
  run, no power-limit/TDP/clock-lock changes, no target-gen redesign yet, no warm-start/per-target-cap/safety
  changes, no version bump. **Hardware BLOCKED** until the classification + collapse report land and a fresh
  dry-run shows them.

## (2026-06-15) — FIRST confirmed hardware validation of bind-seeking F1b v2 strictness (commit bf02971) — mechanism PASS, frontier PARTIAL
- Supervised confirmed run (operator present) validating `bf02971`; docs at `3b8774c`
  (HEAD/origin/master = `3b8774c`, tree clean). A **fresh worktree binary was built first** — the
  worktree-local `target/debug/nidavellir-service.exe` was absent and the only existing binary was stale
  (main-repo, built 2026-06-07, predating bind-seeking): `cargo build -p nidavellir-service` → worktree
  binary created after the build; stale main-repo binary NOT used; tree stayed clean.
- **Dry-run gate passed** (no `--confirm`): bind-seeking ENABLED; v2 start-bin-not-eligible note; thresholds
  `avg_clock_overshoot <= 30 MHz` + `power_capped_frac <= 0.50`; coverage-bounded scheduler;
  `max_probes=Some(21)`; `max_probes_per_target=Some(3)`; targets `[1935,1905,1875,1845,1815,1785,1755]`;
  first-pass bins `[1075,1068,1062]`; warm-start OFF; no applied-profile / Safe Loop warning; dry-run no-op
  line. Confirmed: `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`.
- **Safety PASS**: exit 0; no TDR/driver-reset/black-screen/reboot/crash; `reset_to_stock` ran; GPU back at
  stock idle. After: `boot_flag.json`/`gpu_applied.json` absent; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` idle (`safe_mode:false`), size unchanged, mtime touched by
  startup recovery only.
- **Probe**: 15 dwells; all 7 targets characterized; 6 via **`BoundBinding`** (1935/1905/1875/1845/1815/1785),
  1 via **`PerTargetCap`** (1755); none dropped; `--max-probes 21` not exhausted (15/21); no `overshoot_veto`;
  all probes `write_mode=monotone_static`, `positive_offsets=0`.
- **v2 mechanism PASS (start-bin guard)**: every 1075 mV start bin `eligible=false/bound=false`; all 7
  descended to 1068, 1755 to 1062; earliest bind only after a real descent (6 bound at 1068, `reason=Clock`);
  bind telemetry present (eligible/bound/reason/avg_clock_mhz/p5_clock_mhz/power_capped_frac); regime arm never
  fired (pcf=1.000).
- **Frontier PARTIAL (did NOT de-collapse)**: all dwells PowerLimited, `power_capped_frac=1.000` throughout
  (~199 W flat); clocks clustered **~1798–1819 MHz**; confidence stayed **0.21**; Godforge/Brokkr/Deep Calm
  collapsed to ~1800 MHz/199 W. v2 fixed the **procedural** start-bin bug; the remaining collapse is
  **power/regime**, not scheduler depth and not the per-target cap.
- **Direction**: don't repeat the run; don't bump the per-target cap as the immediate next step; don't jump to
  risky power-limit/clock-lock changes. Next design (analysis first): **regime-aware binding**, distinguish
  `Clock` from `PowerLimitedPlateau`/`PowerBoundCollapse`, veto `Clock` binding when pcf is saturated ~1.0,
  add collapse diagnostics + power-headroom/power-drop telemetry. **Stop for analysis before any further
  confirmed run.** See `handoff.md` + `decisions.md`.

## (2026-06-15) — bind-seeking F1b v2 strictness IMPLEMENTED + pushed (commit bf02971) — hardware-validated (see entry above)
- **Commit `bf02971 fix(service): tighten bind-seeking stop criteria`**, pushed to `origin/master`
  (HEAD = origin/master = `bf02971`). Scope: `crates/service/src/gpu_power_sweep.rs` only.
- **Why**: v1's first supervised hardware run was safety/mechanics **PASS** but semantic **PARTIAL** — v1
  bound on the **first/start bin (1075 mV)**, so every viable target stopped immediately, no descent occurred,
  frontier stayed degenerate (single-bin ~1075 mV / ~199 W, Forge confidence ~0.21).
- **v2**: start bin NOT bind-eligible (earliest bind = 2nd probed real VF bin); clock binding uses the
  **average/achieved clock** (`avg - target <= 30`), not p5/sustained (p5 = telemetry only); regime arm
  `power_capped_frac <= 0.5` kept but **invalid/missing cap_frac fails closed**. New `BindReason`/`BindDecision`
  + per-probe bind telemetry (eligible / bound / reason / avg_clock_mhz / p5_clock_mhz / power_capped_frac);
  dry-run reports the start-bin-not-eligible caveat.
- **Precedence preserved**: crash → abort → budget drain → verifier failure → dwell instability → binding →
  per-target cap → floor. **Safety unchanged**: monotone writer, verifier gates, Safe Loop, `reset_to_stock`,
  persistence/apply, hardware-floor derivation, warm-start default OFF.
- **Validation (no hardware)**: `cargo check` clean; `cargo test -p nidavellir-service` **169 passed**;
  dry-run only passed; no hardware boundary crossed.
- **Hardware-validated 2026-06-15** (see the FIRST confirmed hardware validation entry above) via
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking` — mechanism PASS (start-bin guard), frontier PARTIAL (still power-limited / collapsed).

## 2026-06-14 — bind-seeking F1b v1 IMPLEMENTED + pushed (commit 08f745e), hardware-validated PARTIAL → superseded by v2
- **Commit `08f745e feat(service): add opt-in bind-seeking to build-frontier`**, pushed to `origin/master`
  (HEAD = origin/master = `08f745e`). Scope: `crates/service/src/gpu_power_sweep.rs` +
  `crates/service/src/main.rs` only. Builds the bind-seeking direction from the `5248758` run.
- **Feature**: opt-in CLI flag **`--bind-seeking`** + `FrontierLimits.bind_seeking`, **default OFF** (absent =
  current behavior byte-for-byte). Per target the descent stops at the first verified+stable **binding** point
  instead of walking a fixed bin count, so targets can differentiate (vs the prior 1832–1867 MHz / 194–199 W
  collapse).
- **Binding v1 (Clock + regime)** — pure `classify_binding`: verified + stable AND either
  `sustained - target <= BIND_OVERSHOOT_MHZ (30)` (sustained = p5 else avg) OR `power_capped_frac <=
  BIND_CAP_FRAC (0.5)`. **Power-drop is intentionally NOT a v1 stop-condition** (no top-power reference
  tracking; telemetry/log later, not binding logic now).
- **Scheduler**: new `BracketStop::BoundBinding` — clean (`is_hard_failed()==false`), carry-forward eligible
  with a `lowest_verified_mv`. Binding checked only on a verified+stable sample, after the failure arms.
  Precedence preserved: crash → aborted → global budget drained → verifier-failure/unverified →
  dwell-unstable/silent-error → **binding** → per-target cap / floor.
- **Invariants**: `--max-probes` = hard global cap; `--max-probes-per-target` = per-target attempt/depth cap
  (bind-seeking may stop earlier); **warm-start default OFF**. Unchanged: monotone static-base writer, verifier
  gates, Safe Loop, `reset_to_stock`, hardware-derived floor, persistence/profile apply. No
  power-limit/clock-lock changes.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **165 passed / 0 failed**. Dry-run only (no `--confirm`): `--max-targets 7 --max-probes 21
  --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking` → exit 0; `bind-seeking: ENABLED`, thresholds
  + caveat, warm-start OFF, no Safe Loop arm / apply / dwell / VF write.
- **Hardware validation NOT yet done for `08f745e`.** Next (separate, operator-present): clean confirming
  dry-run, then `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`. No hardware commands run in the implementation or docs pass. See
  `handoff.md` + `decisions.md`.

## Latest (2026-06-13) — FIRST confirmed hardware validation of F1b `--max-probes-per-target` (commit 5248758) — coverage PASS, profile PARTIAL
- Supervised run, operator present, after a clean confirming dry-run (no plan drift; HEAD/origin/master
  `5248758`; `47f39be`/`f90981d`/`8503182` present; `gpu_applied.json`/`boot_flag.json` absent;
  `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 14 --max-probes-per-target 2 --safe-start-cap 1075`
  — **warm-start OFF**. Exit 0; ~4 min; no TDR/driver-reset/black-screen/reboot/crash.
- **Safety PASS**: Safe Loop armed→cleared **per probe** (idle); `reset_to_stock` ran ("GPU restored to stock;
  no profile applied or persisted"). `boot_flag.json`/`gpu_applied.json` absent before+after;
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged (no forge-state persistence, no knowledge
  write); `safe_loop.json` content/size unchanged (idle, `safe_mode` false), mtime touched only — **no new
  blacklist/crash entry**. GPU back at stock idle.
- **Coverage PASS (the fix works)**: **13 dwells across all 7 targets** (vs prior 34-on-1935 depth-first). 6
  targets stopped via **`PerTargetCap`** (`probes_used=2`, bins **1075 + 1068 mV**); global `--max-probes 14`
  **not exhausted** (13 used) — the cap stopped one target from eating the whole budget. **1905 dropped** at
  probe 1 (`LiveMismatch`, `overshoot_veto=true`, `eff_cov=0.963` — conservative verifier reject; neighbors
  passed). All probes `write_mode=monotone_static`, `positive_offsets=0`; `NoDownCapNeededCeiling` (1935) +
  `VerifiedCurve` (1875–1755). No writer/verifier/Safe-Loop/reset/persistence regression. Shallow only
  (1075/1068 mV); did **not** touch 875/868/862/856/850.
- **Profile PARTIAL**: achieved clocks clustered **1832–1867 MHz**, power **194–199 W**; lower targets not
  distinct; Godforge/Brokkr/Deep Calm collapsed to **1860 MHz / 194 W**; FORGE confidence 0.21. **Shallow
  near-stock coverage (1075/1068 mV) is non-binding on this hard power-capped 3060 Ti** — the ceiling does
  not govern the achieved clock at that high-voltage band; the cap solved budget *distribution*, not binding.
- **Direction — bind-seeking F1b**: don't repeat the flags / use cap 3 / enable warm-start / jump to
  power-limit/clock-lock changes next. Per target: descend while stable-but-non-binding, stop when it BINDS /
  fails verifier/dwell / hits a cap. Goal = first useful (binding) point per target, not deepest voltage. No
  further hardware commands were run. See `handoff.md` + `decisions.md`.

## (2026-06-13) — FIRST confirmed hardware validation of the bin-based floor (commit f90981d) — PASS (safe)
- Supervised bounded run, operator present, after a clean dry-run on a fresh debug build (HEAD/origin/master
  at `c99dbf1`+`f90981d`; `23b70c4`/`8503182` present; `gpu_applied.json`/`boot_flag.json` absent;
  `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 34 --safe-start-cap 1075 --warm-start-brackets`.
  `--max-probes 34` reaches 868 mV (one bin below the old 875 floor) and stops before 862 mV (reboot-zone).
- **Safety PASS**: exit 0; no TDR/driver-reset/black-screen/reboot/crash. Startup recovery clean; Safe Loop
  armed→cleared (idle); `reset_to_stock` ran ("GPU restored to stock; no profile applied or persisted").
  `boot_flag.json`/`gpu_applied.json` absent before+after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` byte-identical (idle, `safe_mode` false, size unchanged),
  mtime touched at run start only — no new blacklist/crash entry. GPU back at stock idle.
- **Coverage**: 34 dwells **all on target 1935** (1075→868 mV). Reached 875 + 868 mV; **did not reach 862**
  (no `ceiling_mv=862`; the 35th scheduler step hit `BudgetExhausted` before write/dwell). 1905/1875/1845/
  1815/1785/1755 NOT physically characterized (budget spent on the hardest target). Warm-start: B1 1935 from
  cap, B2 1905 carried 893 mV (868+25). All probes `write_mode=monotone_static`, `positive_offsets=0`,
  `down_caps=0`, no `overshoot_veto`, all `NoDownCapNeededCeiling`, `eff_cov=1.000`.
- **Interpretation**: validated safe WRITING/descent of the static ceiling to 868 mV; did NOT prove core
  stability when RUN at 868 — GPU stayed **power-limited ~198 W**, ceiling **non-binding**. Frontier point
  1935 → 1839 MHz @ 868 mV vf_bin / 198 W. **PASS for first bin-based floor validation; partial for profile
  synthesis** (single clock 1800 MHz → FORGE confidence 0.21, profiles collapse). 862/855 reboot-zone
  blacklist is offset-keyed (different regime from this zero-offset power-limited descent).
- **Direction**: don't jump to `--max-probes 40`; `--max-probes 35` could deliberately touch 862 for
  boundary mapping (862 blacklist keyed freq=1755 won't match a 1935 ceiling → Safe Loop is backstop) but
  won't yield useful profiles. **Primary next: pivot to F1b / multi-clock, and/or make the ceiling BIND
  (raise power limit) before descending deeper.** No further hardware commands were run. See `handoff.md`
  + `decisions.md`.

## Bin-based floor shipped (2026-06-13) — build-frontier floor is hardware-derived / bin-based (commit f90981d, pushed)
- `f90981d feat(service): derive build-frontier floor from real VF bins` removes the hardcoded active
  **875 mV** descent floor. The floor is now the lowest real graphics-core VF bin
  (`seed.cluster_v_min_mv`); no replacement constant (no 825/800). Descent is **bin-based**: walks real
  VF/core-cluster bins only, never invents off-curve 25 mV voltages. Warm-start maps its margin to the
  conservative real bin **≥** the requested target and never starts below the previous
  `lowest_verified_mv`. `--max-probes` stays the exposure cap. Empty bin domain → fail closed before any
  hardware write. Dry-run prints the hardware floor + exact bin sequence + bin/dwell counts.
- Scope: only `crates/service/src/gpu_power_sweep.rs`. **Unchanged**: monotone static-base writer,
  verifier gates, Safe Loop, `reset_to_stock`, persistence, profile apply. `cargo check` clean; service
  tests 142 passed.
- The historical `1755 @ 875` validations remain valid for that point but are no longer an active floor;
  runs may now go **below 875**. **First confirmed hardware validation done 2026-06-13 (safe to 868 mV;
  see the Latest entry above).** First runs must be bounded
  (`--safe-start-cap`/`--max-probes`), dry-run reviewed before `--confirm`, operator present (descent may
  reach **below the ~855 mV reboot zone**). See `handoff.md` / `decisions.md` for the suggested dry-run.

## Current status (2026-06-05)
- `master`, tag **v0.3.1** (forge-state persistence pushed). Worktree branch
  `claude/vibrant-almeida-dfb6c7`.
- Active work: **foundation reviews before F1b** (F1b on hold, direction not final).
  Review 1 (persistence/startup) **done** → forge-state persistence shipped (below).
  Applied-Curve-Verification review **done** (investigation; see handoff).
  Review 2 (Sensor Quality Audit) **done** (investigation; key decision below).
- **F1b warm-start voltage-bracket carry-forward — SHIPPED + HARDWARE-VALIDATED (2026-06-13, commits
  23b70c4, 6f2f061)**: generic ordered-descent scheduler primitive (NOT Godforge-specific) behind
  opt-in **`--warm-start-brackets` (default OFF)** — an easier target reuses the previous harder
  target's verified + dwell-stable bracket (`lowest_verified_mv + 1 step`), skipping dominated
  high-V probes. Preserves monotone writer / verifier gates / `overshoot_veto` / Safe Loop /
  `reset_to_stock` / persistence / 875 mV floor. **Validation PASS**: supervised
  `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075 --warm-start-brackets`
  — exit 0; no TDR/reboot; Safe Loop clean; `reset_to_stock` ran; `boot_flag.json`/`gpu_applied.json`
  absent after; `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; GPU stock idle.
  **33 probes**, all 7 targets produced points; **B1/B2/B3 held, B2 exercised** (1905 failed verify at
  warm-start 900 mV → fell back once to cap 1075, target preserved). **−5 probes vs from-cap (38)** for
  an identical frontier (32 baseline ≈ flat); modest on RTX 3060 Ti (mid targets stop early on
  verify-axis residual overshoot). `1755 @ 900`/`@ 875` re-validated (`NoDownCapNeededCeiling`,
  overshoot 0, plateau 1665..1755 / 1620..1755, ≈1755 MHz @ 875 mV ≈176 W); `write_mode=monotone_static`,
  `positive_offsets=0`. Follow-up `6f2f061` surfaces scheduler `result.log` before `result.profiles.log`
  (log-only, deduped). **Keep default OFF**; next (later): more runs, benign-zero-only seeding
  refinement, broader confidence work; do NOT mix with persistence. See `handoff.md` + `decisions.md`.
- **F1b Phase 2B.2-c — monotone static-base VF writer HARDWARE-VALIDATED (2026-06-12, commit
  8503182)**: supervised `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap
  1075` on a fresh `origin/master` debug build at `8503182`, after a clean bounded dry-run.
  **Safety held** (exit 0; no TDR/reboot; Safe Loop armed/cleared; `reset_to_stock` ran;
  `boot_flag.json`/`gpu_applied.json` absent after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; GPU back at stock idle). **Writer confirmed**: all 32 probes
  `write_mode=monotone_static`, `positive_offsets=0`, `static_base_points=132`. **Primary fix —
  `1755 @ 900 mV`**: OLD plateau 1755..1845 / `overshoot_veto=true` / `LiveMismatch` → NEW plateau
  1665..1755 / overshoot=0 / `NoDownCapNeededCeiling` (pass). Run continued to **`1755 @ 875 mV`**
  and verified (`NoDownCapNeededCeiling`, overshoot=0, plateau 1620..1755, ~19 s dwell, ≈1755 MHz @
  875 mV ≈179 W). Minor residual: a few non-1755 low-ceiling probes still show single-bin 15 MHz
  overshoot (not a blocker). FORGE synthesis low confidence (best 0.21) is the unrelated Wilson
  metric. **Next**: design warm-started voltage-bracket reuse for F1b/Godforge; do NOT mix with
  persistence/profile apply yet. See `handoff.md` + `decisions.md`.
- **F1b Phase 2B.2-c — FIRST confirmed run (2026-06-11, SAFE) + c.1 verifier fix (IMPLEMENTED, not
  committed)**: first supervised `build-frontier --confirm --max-targets 1 --max-probes 6
  --safe-start-cap 1075` ran after a Fable 5 GO audit + clean dry-run. **Safety held end-to-end**
  (no TDR/reboot; Safe Loop armed/cleared per probe; reset-to-stock on reject + at run end; no
  persistence; GPU back at stock) but **0 frontier points**: the target=1935 (stock boost top)
  probe was rejected `LiveMismatch offsets=20/27 plateau=1935..1935 overshoot=0` — flatten-to-top
  needs zero offset on bins already at target, so the ≥90% presence gate under-counts. **c.1**:
  narrow stock-equivalent path (`is_stock_equivalent_ceiling`, gpu_verify.rs) — only on
  LiveMismatch, only for targets within tol of the caller-passed stock top, all offsets readable,
  no overshoot (even in-tol), all bins in-tol below target, zero-offset bins EXACTLY at target;
  surfaced as service-internal `LiveCeilingEval.stock_equivalent` (IPC untouched);
  `verify_applied_curve` passes None (byte-identical); probe logs `verify=StockEquivalentCeiling`.
  Condition 1 directional (rejects targets above stock top). `cargo check` clean · service
  **109/109** (+11) · core 46/46. Files: `gpu_verify.rs`,
  `gpu_power_sweep.rs`. Next: bounded dry-run on rebuilt binary, then re-attempt the same bounded
  --confirm (user approval). Chain b.1→c.0 IS pushed (6881cd7); c.1 awaits commit approval.
- **F1b Phase 2B.2-c.0 — first-run limiter flags (2026-06-08) — pushed (6881cd7)**: added
  `build-frontier` flags `--max-targets N` / `--max-probes N` / `--safe-start-cap MV` to bound the
  first supervised run. Pure `FrontierLimits`/`validate_limits`/`apply_frontier_limits`
  (gpu_power_sweep) + `parse_frontier_limits` (main.rs). FAIL CLOSED on absurd values (0 / cap ≤
  crash floor / non-numeric / missing); cap never raises above the derived top nor below the floor;
  max-probes short-circuits remaining probes then resets to stock. Defaults preserve the full plan.
  No IPC/core/contract/apps-ui/Safe-Loop/persistence change, no hardware. `cargo check` clean ·
  service **95/95** (+7) · core 46/46. Files: `gpu_power_sweep.rs`, `main.rs`. **Dry-run QA**
  (`--max-targets 1 --max-probes 6 --safe-start-cap 1075`, stock, no --confirm, no state writes):
  targets=[1935], descent 1075→875 (9 bins), 6 dwells (~120 s capped). --confirm still forbidden.
- **F1b Phase 2B.2-b.4 — stock core VF cluster seeding (2026-06-07) — IMPLEMENTED, not pushed**:
  refines b.3 so `safe_start`/boost come from the actual contiguous core VF cluster, not the global
  max of sane points (which gave 1150 mV). `select_core_cluster` (pure): sort by voltage, split on
  gaps > 60 mV, pick the largest run (≥ 8 pts else FAIL CLOSED), derive safe_start/boost from the
  cluster top; isolated high-V points reported as rejected outliers. b.3 generic hard guards
  (500..3500 MHz, 600..1150 mV) retained. Dry-run prints cluster range + outliers + safe_start
  source + applied-profile warning. No IPC/core/contract/apps-ui/Safe-Loop/gpu_apply/nvml_gpu/
  Phase-3/11D change, no auto-reset, no hardware. `cargo check` clean · service **88/88** · core
  46/46. File: `gpu_power_sweep.rs`. **Stock dry-run QA pending user's manual reset; --confirm still
  forbidden.** (b.3 + b.4 both uncommitted — eventual commit bundles them unless split.)
- **F1b Phase 2B.2-b.3 — core-domain seeding guard (2026-06-07) — IMPLEMENTED, not pushed**: the
  first dry-run exposed seeding from the UNFILTERED global max of `read_vf_curve_modern()` (picked up
  memory-domain points → bogus plan: targets 7001..6311 MHz, safe_start 1237 mV; the dry-run gate
  blocked it, no hardware). Fix (pure): `sane_core_points` (freq 500..3500 MHz, voltage 600..1150 mV)
  + `derive_core_seed` (seed from sane points only; reject diagnostics; soft-warn >3200 MHz / >1125
  mV; FAIL CLOSED if no sane points or > hard guard). `run_build_frontier` aborts with no
  arm/apply/dwell/VF-write on fail-closed or any target > 3500 MHz. Re-run dry-run: 132 raw → 88 sane
  / 44 rejected (incl. 7001/1237), boost~1935, targets 1755..1935, 84 dwells (~1680 s), safe_start
  1150 mV (flagged soft-max). NO hardware, NO state writes (mtimes unchanged), NO --confirm. `cargo
  check` clean · service **86/86** (+5) · core 46/46. File: `gpu_power_sweep.rs`. `--confirm` still
  forbidden pending review. NB: plan reflects the currently-applied curve; a stock read is cleaner.
- **F1b Phase 2B.2-b.2 — real probe + supervised `build-frontier` (2026-06-07) — IMPLEMENTED (code
  only, NOT run), not pushed**: added the real Windows probe `real_probe_step` (snap bin → arm Safe
  Loop → `apply_vf_ceiling` → shared `classify_live_ceiling` verify + 11C diag → `load_and_measure`
  → clear → `measured_to_probe` + `vf_bin_mv`; dwell-crash → reset + abort-flag short-circuit) and
  `run_build_frontier(store, confirm)` (always prints the plan; dry-run read-only; `--confirm` runs
  the real frontier then ALWAYS resets to stock). Console subcommand `build-frontier` in `main.rs`
  (`--confirm` runs startup recovery first; dry-run does not). **No auto-apply, no forge_state, no
  gpu_knowledge writes, no IPC/contract/core/apps-ui change, hardware path NOT executed.** Conservative
  first-run consts (lowest_safe=875 mV, 25 mV step, idle Unconstrained→PowerLimited). `cargo check`
  clean · service **81/81** (+1) · core 46/46. Files: `gpu_power_sweep.rs`, `main.rs`. Dry-run:
  `nidavellir-service.exe build-frontier`; confirmed (NOT run): `... build-frontier --confirm`.
  Supervised QA = 2B.2-c (separately gated); 11D after Phase 2B.
- **F1b Phase 2B.2-b.1 — seeding + dry-run plan + vf_bin propagation (2026-06-07) — IMPLEMENTED, not
  pushed**: exposed `classify_live_ceiling`/`LiveCeilingEval`/`CurveDiag` `pub(crate)` (intra-crate;
  no IPC/contract change); added pure `derive_descent` (FrontierDescent from live curve bins + crash
  floor) + read-only `plan_frontier` (dry-run worst-case dwell count/wall-time + safety notice);
  added internal `ProbeSample.vf_bin_mv` (NOT IPC) so `probe_to_point` records the actually-applied
  snapped bin (fallback = descent vbin); `measured_to_probe` leaves it None (the real probe fills it).
  NO real probe / apply / load / sweep / stress / subcommand / Safe-Loop / startup-recovery /
  persistence / Phase-3 / 11D / apps-ui / core / contract change, NO hardware. `cargo check` clean ·
  service **80/80** (+7) · core 46/46. Files: `gpu_power_sweep.rs`, `gpu_verify.rs`. 2B.2-b.2 (real
  probe + supervised `--confirm`) separately gated.
- **F1b Phase 2B.2-a — shared live-ceiling classifier (2026-06-07) — IMPLEMENTED, not pushed**:
  factored `classify_live_ceiling` (read-only) + pure `eval_ceiling_evidence` → `LiveCeilingEval`
  out of `verify_applied_curve` so the verifier and the future transient-ceiling probe (2B.2-b)
  share one classification path. **VerifyAppliedProfile output byte-identical** (same offset-presence
  `classify_curve` gate + 11C diag; voltage never affects classification). Service-internal only —
  no core/contract/apps-ui/Safe-Loop/synthesis change, no hardware, no apply/load/sweep/stress.
  `cargo check` clean · service **73/73** (+5 pure tests) · core 46/46. File:
  `crates/service/src/gpu_verify.rs`. Seeding helpers deferred to 2B.2-b (avoid dead code). 2B.2-b
  (real probe + supervised `--confirm` console entry) separately gated.
- **F1b Phase 2B.1 — pure probe-mapping prep (2026-06-07) — IMPLEMENTED, not pushed**: added pure
  `measured_to_probe` (Measured→ProbeSample, conservative: Stable only on ≥Medium clock/power
  telemetry + p5 present; SilentError/Crash/TDR→Unstable; p5 preserved 0→None; missing voltage None
  not 0) + additive `PowerSweepPoint.target_clock_mhz: Option<u32>` (serde default, backward-
  compatible, no schema bump). Phase 2A `probe_to_point` stamps the target; single-clock live sweep
  sets None. **NO hardware path, NO real probe, NO apply/sweep/stress, NO apps-ui/Safe-Loop/synthesis
  /Phase-3/11D change.** `cargo check` clean · service **68/68** (+7) · core **46/46** (+2). Files:
  `crates/service/src/gpu_power_sweep.rs`, `crates/core/src/ipc.rs`, contract, decisions/memory/
  handoff. Phase 2B.2 (real probe closure + supervised console entry) and the hardware QA run remain
  separately gated; 11D deferred to after Phase 2B.
- **Patch 11C — read-only live VF-ceiling diagnostic (2026-06-06) — IMPLEMENTED, not pushed**:
  extended the read-only verifier (`gpu_verify::verify_applied_curve` / `verify-applied`) with a pure
  `compute_curve_diag` (first modified bin idx/mv, modified vs expected count, GetStatus freq-match,
  GetStatus plateau min/max, max target overshoot/undershoot, 3 offset samples) + a single read-only
  `LiveSnapshot` (NVAPI voltage + first NVML clock/power/util/temp/limit/cap). Surfaced via additive
  `Option`/`serde(default)` fields on `ApplyVerificationStatus` + one `apply_verify_diag:` log line.
  **Classifier unchanged** (offset-presence gate; live voltage above anchor never downgrades; GetStatus
  freq diagnostic only). Exact-offset verification deferred (needs persisted stock base or validating
  the GetStatus `base` tuple). Files: `crates/service/src/gpu_verify.rs`, `crates/core/src/ipc.rs`,
  `docs/contracts/ui-backend.md`, decisions/memory/handoff. **No apply/Safe-Loop/synthesis/`apps/ui`/
  `nvml_gpu.rs` change; no hardware writes.** `cargo check` clean · service **61/61** (+9 diag) · core
  44/44. **Runtime QA** (`verify-applied`, read-only, no writes — all state-file mtimes unchanged):
  `VerifiedCurve` 62/64 offsets present, but diagnostic showed `anchor_offset=+255000`,
  `highest_bin_offset=−120000`, GetStatus plateau **1770–1830** (overshoot 45), live
  `voltage=1068 mV, clock=1815, util=6%` — consistent with both a curve-flatten-shaped offset set AND
  the open overshoot suspect; GetStatus idle noise (18/64) makes it non-conclusive (as designed).
- **Applied voltage behavior — investigation + Patch 11A docs (2026-06-06) — DOCS ONLY, not pushed**:
  confirmed (read-only) that the elastic VF ceiling (`apply_vf_ceiling`) writes **per-point
  FREQUENCY offsets** to every modern VF point at/above the deterministic `vf_table_voltage_mv`
  bin (flatten to `target_mhz`); it writes **no voltage** and does **not** hard-cap measured/rail
  voltage. `vf_table_voltage_mv` (VF/curve bin) = the deterministic apply/verify/frontier key;
  `measured_voltage_mv` / HWiNFO "GPU Core Voltage" are a different (rail, load-line/droop) domain
  and may read ABOVE the bin (idle ~1.075 V and in-game ~0.887–0.956 V for an ~850 mV bin are
  EXPECTED, not a mismatch). Nidavellir must not imply a hard voltage cap; a true cap = the legacy
  voltage-lock (TDR) path, rejected by safety-first. **Patch 11A** records this in `decisions.md` +
  `docs/contracts/ui-backend.md` (incl. a Codex wording request: drop "MHz @ mV", use "target" +
  "VF bin", keep measured voltage separate) + `handoff.md`. **No backend code, no `apps/ui`, no
  apply/verify/F1b/hardware change.** Open suspect (read-only-testable, deferred to 11C): apply
  offsets are `target − GetStatus_base` and GetStatus under-reports at idle → a plateau applied at
  idle may land above target (~1815–1830 vs ~1785, on top of normal 15 MHz boost-bin quantization).
- **Applied curve verifier — Patch A (this session) — IMPLEMENTED, not pushed**: read-only
  `VerifyAppliedProfile` IPC + `crate::gpu_verify`. Classifies the live modern VF curve vs
  the applied profile into `CurveVerification` = NotApplicable / MetadataOnly /
  VerifiedCurve / LiveMismatch / VerificationFailed. **Table-to-table only**: re-derives the
  deterministic ceiling bin via `nearest_vf_bin_at_or_above(core.voltage_mv)` (same as apply),
  reads `read_vf_curve_modern` (GetStatus) + `vf_get_point_khz` (offset corroboration, logged);
  expected = points ≥ ceiling read target ±15 MHz, ≥90% match → VerifiedCurve. **Read-only**:
  no apply/reapply/write/stress. No telemetry/load/context/stock-fingerprint yet (Patches B/C).
  Additive IPC (`ApplyVerificationStatus`), contract noted. Tests: check clean · service 26/26
  (+7 verifier). **Read-only runtime path**: `nidavellir-service.exe verify-applied` console
  subcommand runs the verifier with NO startup-recovery/heartbeat/`reapply_on_boot`/pipe server
  → no apply, no VF write (proven: `gpu_applied.json` mtime unchanged).
- **F1b Phase 2A — simulated multi-clock outer-loop scaffolding (2026-06-06) — DONE, not pushed**:
  `build_frontier(candidate_clocks, &FrontierDescent, &ForgePolicy, probe: impl Fn(u32,u32)->
  ProbeSample)` in `gpu_power_sweep.rs` proves the outer loop, per-target voltage-bin descent,
  stopping rules, known-unsafe boundary, frontier assembly, and synthesis wiring **with NO
  hardware** — the probe closure is the only seam to (future) hardware. No `load_and_measure`,
  no `apply_vf_ceiling`, no VF write, no GPU stress, no Safe Loop interaction, no real power sweep.
  Frontier points use `vf_table_voltage_mv` (deterministic bin); measured voltage stays telemetry.
  Inner loop keeps deepest stable, stops on first instability or simulated `curve_verified=false`,
  never probes below `lowest_safe_mv`. 3060 Ti (1830/1815/1740) and 4090 (2880/2860/2700) proven
  through the loop. No IPC/persistence field added. `cargo check` clean · service **52/52** (+8 sim).
  **Phase 2B (future)**: real probe closure (apply ceiling → Safe-Loop-armed dwell → offset-readback
  VerifiedCurve gate) behind a supervised/approval-gated run. **Phase 3** (knowledge re-keying)
  remains future. See `decisions.md`.
- **F1b Phase 1 — policy-driven multi-clock synthesis (2026-06-06) — DONE, not pushed**: pure
  service-internal logic in `gpu_power_sweep.rs`. `ForgePolicy` (Balanced 0.98/0.90/0.85 +
  Conservative/Aggressive presets); `synthesize_forge_profiles` now takes `&ForgePolicy` and
  applies clock floors: Godforge = highest **sustained** clock (prefers `p5_clock_mhz`, falls back
  to `clock_mhz`; ties→lowest power); Brokkr's = **max R within the Brokkr's clock floor**; Deep
  Calm = max MHz/W within the Deep Calm floor. Measured voltage is NOT a selection axis
  (`vf_table_voltage_mv` stays the deterministic apply axis). Single-clock collapse detected +
  logged (returns all three, no panic). Added `Regime` enum + pure `classify_regime` +
  `candidate_clocks` (Phase-2 helpers). **4090 example resolved: Brokkr's = 2860** (max-R-within-
  floor). No IPC/apps-ui/Safe-Loop/hardware change. `cargo check` clean · service **44/44** (F1a
  assertions unchanged, +9 F1b tests). **Phase 2 NOT started** — needs simulated outer-loop
  scaffolding before any (supervised/approval-gated) hardware multi-clock sweep. See `decisions.md`.
- **Forge action consolidation audit (2026-06-06) — recorded, no code change**: backend has two
  engine generations. **Canonical = `gpu_power_sweep.rs` (Power Sweep)**: offset + elastic VF
  ceiling, game-power dwell, no voltage lock → the Forge GPU core path (apply via
  `ApplyPower*`). **Legacy (voltage-lock, TDR risk) = `gpu_sweep_real.rs` (Real Sweep) +
  `gpu_forge_all.rs` (Forge Everything)** + the legacy `ApplyGodforge/Brokkrs/DeepCalm` trio →
  hide from normal UI, remove later (keep IPC wired for now). **Memory sweep** (`gpu_mem_sweep.rs`)
  = no core voltage lock but runs independent of the forged core → Advanced Diagnostic until the
  VRAM redesign. **Product action model**: primary = **Forge GPU** (→ **Refine Profiles** once
  profiles exist); **Advanced Diagnostics** = GetGpuCurve / StartGpuValidation / StartBenchmark /
  VerifyAppliedProfile / StartMemSweep; legacy paths hidden/developer-only. VRAM = future Forge GPU
  pipeline step, never a separate primary button. See `decisions.md` + `docs/contracts/ui-backend.md`.
- **Patch B — load-state classification (2026-06-06) — DONE, not pushed**: adds a second
  orthogonal LOAD axis to `ApplyVerificationStatus` (`load_state: LoadVerification` =
  NotEvaluated / VerifiedUnderLoad / TelemetryInsufficient / LoadMismatch /
  WorkloadStateMismatch(reserved) / LoadVerificationFailed) + diagnostic dwell fields. Derived
  from the applied point's EXISTING synthetic-dwell stats (read-only `load_restored_progress()`
  reads `forge_state.json`; matches the point by label→named slot, fallback unique points entry).
  Rules: load only evaluated when curve verified; `p5_clock ≥ target−30 MHz` + `telemetry_quality
  ≥ Medium` → VerifiedUnderLoad; voltage is telemetry-only; `stable=false`→LoadMismatch; bad power
  →LoadVerificationFailed. Derivation: load upgrades VerifiedCurve→VerifiedUnderLoad, never
  downgrades. `status` stays the curve axis; additive serde-default fields. **Runtime QA**
  (`verify-applied`, read-only, no writes): curve=VerifiedCurve(63/65), load=TelemetryInsufficient
  ("legacy point without dwell quality" — persisted point predates the dwell-stats patch),
  status=verified_curve. Tests: check clean · service 35/35 (+10). Next: Forge Action Consolidation.
- **Patch A.1 — offset-based curve verification (2026-06-06) — DONE, not pushed**: runtime QA
  proved GetStatus actual-freq is unreliable at idle (it under-reported the plateau 31/65 even
  though the flatten offsets were resident 63/65). `classify_curve` now gates on the **GET-control
  offset readback** (`vf_get_point_khz`): a point ≥ ceiling counts as flattened if it carries a
  **non-zero** offset (presence, not exact value — per-point stock base isn't persisted); ≥90% →
  VerifiedCurve. GetStatus freq match is kept as logged diagnostic only. Unreadable offsets →
  VerificationFailed (safer than mismatch). **Re-ran `verify-applied`**: now `VerifiedCurve`
  (offset_match 63/65, getstatus 31/65 diagnostic), no write (`gpu_applied.json` mtime unchanged).
  Tests: check clean · service 25/25 (6 offset-based verifier tests). Patch B unblocked.
- **Richer dwell stats (this session) — IMPLEMENTED, not pushed**: second patch off the
  Sensor Audit. `PowerSweepPoint` gains optional `min_clock_mhz`/`p5_clock_mhz`,
  measured-voltage `avg/min/max` + `voltage_sample_count`, `dwell_sample_count`/
  `dwell_duration_ms`, `start/end/avg_temp_c`, and `voltage_quality`/`telemetry_quality`
  (new `DwellQuality` enum: high/medium/low/unavailable). Voltage stats are now
  **ramp-filtered + sanity-checked (500–1250 mV)**; the legacy unfiltered voltage max is
  **unchanged** so the apply-key behavior is untouched. Per-point `dwell_stats:` log line.
  No UI / Safe Loop / synthesis / F1b change; additive serde-default fields (old
  `forge_state.json` loads; `PowerSweepPoint` stays `Copy`). Tests: `cargo check -p
  nidavellir-service` clean · core 44/44 · service 19/19. **Limitations**: full NVML
  limiter reasons deferred; voltage cadence still ~480 ms; no per-sample timestamps; no
  hotspot/fan; `arduous_validate` soak path doesn't yet use the richer stats.
- **Voltage field separation (this session) — IMPLEMENTED, not pushed**: first patch
  off the Sensor Audit decision. `PowerSweepPoint` now separates `measured_voltage_mv`
  (telemetry) from `vf_table_voltage_mv` (deterministic apply/frontier key); legacy
  `voltage_mv` kept for compat/display. **Apply path snaps the measured voltage to a
  real VF-table bin (`nearest_vf_bin_at_or_above`) before `apply_vf_ceiling`** — it no
  longer keys the ceiling on raw measured voltage. Persisted state stays
  backward-compatible (no schema bump; old JSON loads new optional fields as `None`;
  `VfPoint`/`gpu_applied.json` unchanged → apply re-snaps at runtime). Additive IPC
  fields noted in `docs/contracts/ui-backend.md`. Tests: `cargo check -p
  nidavellir-service` clean · gpu-nvapi 5/5 · service 15/15. **Limitations**: the
  frequency-only flatten is unchanged; the ~1062 mV unfocused/desktop state is NOT
  solved by this patch; richer dwell stats + apply verification still pending.
- **Sensor Quality Audit (this session, investigation-only — no code)**: GPU telemetry
  sources are right (NVML clock/power/cap/temp/util; NVAPI curve), but three structural
  gaps found: (1) two disconnected telemetry worlds — "sensor world" (`SensorEngine`/
  `GpuSensors`, **30 s cache, `voltage_mv` always `None`** → UI never gets GPU voltage)
  vs "sweep world" (`load_and_measure`, NVML 30 ms + NVAPI voltage ~480 ms **max**);
  (2) voltage is the weakest signal — string-parsed, sparse, max-only, then **reused as
  the deterministic apply ceiling key**; (3) one type name `voltage_mv` carries three
  incompatible meanings. **Key decision** (see `decisions.md`): split voltage into
  `vf_table_voltage_mv` (apply/frontier key) · `measured_voltage_mv` (telemetry only) ·
  `effective_rail_voltage_mv` (future). **F1b must NOT key on measured dwell voltage.**
- **Forge-state persistence (this session)**: new `forge_state.json` persists the
  final `PowerSweepProgress` (profiles, points, stock baseline) on successful sweep
  completion; startup restores the `PowerSweepHandle` from it when the GPU key
  matches (else idle). Fixes a service restart losing forged profiles/points/apply
  buttons. Backend-only; no UI, IPC, Safe Loop, synthesis or knowledge-schema change.
  **Does not** solve live VF-curve ownership/mismatch — deferred.
- Product model: 3 profiles forged from a clock×power frontier
  (Godforge/Brokkr's/Deep Calm). See `product.md`.
- **V1** continuous per-GPU stability knowledge: implemented, committed, HW-validated.
- **V2** confidence-gated selection: implemented + unit-tested, **committed** (5d72342).
- **F1a**: pure 3-profile synthesis (`synthesize_forge_profiles`) + tests —
  Godforge=clock / Brokkr's=R / Deep Calm=MHz/W; not yet wired (F1b). 6 tests pass.
  **committed** (95753de). See `decisions.md`.
- Branch **reconciled with master governance** (AGENTS.md / CLAUDE.md /
  docs/contracts/ui-backend.md + Codex UI Phases 1–3). UI owned by Codex.

## Completed work (this arc)
- **GPU-first UI Phase 1 cleanup**: Forge is now the default post-onboarding
  screen, and the large `Forge.svelte` view was split into focused UI components
  under `apps/ui/src/lib/components/forge`. No tuning, IPC, or service logic changed.
- **GPU-first UI Phase 2 IA pass**: the Forge view is now organized as GPU Hero
  Status -> Recommended Action -> Profile Comparison -> Forge Knowledge -> Forge
  Progress -> Advanced Diagnostics. Safe Loop status is surfaced with existing IPC.
- **Profile-state UX pass**: active profile cards now show `Applied ✓`, disable
  repeat apply clicks, and emphasize outcome-first expected results.
- **GPU-first UI Phase 3 visual system pass**: Forge Home now uses shared forged
  silicon tokens, reusable status badges, stronger Forge State hierarchy, profile
  identity variants, and a clearer Advanced Diagnostics disclosure. Frontend only;
  no tuning logic, IPC names, or backend contracts changed.
- **Phase 3 visual cleanup**: reduced background texture noise, compacted the GPU
  Hero into a focused control-panel summary, made the forge progression rail
  subtle, and set the desktop window to 1180x820 with a 1100x720 minimum.
- **Forge action cleanup (Codex UI pass, 2026-06-06)**: Forge Home now exposes a
  single primary Forge GPU / Refine Profiles action on the canonical Power Sweep
  path, applies only `ApplyPower*` profiles, moves curve/validation/benchmark/
  applied-profile verification/memory diagnostics into Advanced Diagnostics, and
  labels memory sweep as experimental/future pipeline work. Frontend only; no
  backend, IPC, tuning, or Safe Loop logic changed.
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
Post-audit sequencing (both foundation reviews now done; F1b stays on hold until 1–3):
1. **Split voltage fields + stop keying apply on measured voltage** (must-fix):
   `vf_table_voltage_mv` (apply/frontier key) vs `measured_voltage_mv` (telemetry) vs
   `effective_rail_voltage_mv` (future). Fix voltage acquisition (dense, validated,
   mean/min/max not just max).
2. **Richer dwell stats**: min/p5 clock, voltage avg/min/max, full NVML `ThrottleReasons`
   limiter, sample_count, timestamps, workload-context tag.
3. **Finalize Applied Curve Verification**: post-apply readback comparing the VF-table
   plateau via modern GetStatus (table-to-table, NOT against measured voltage); add the
   read-only verify IPC + `GpuApplyStatus.verification`.
4. **F1b** (only after 1–3): extend the safe flatten sweep to multiple target clocks →
   real game-power clock×power frontier; key by (clock + VF-table point), NOT measured
   voltage; wire `synthesize_forge_profiles` into the live sweep. Needs a supervised HW run.
5. Then F2–F7 (see `product.md` / `roadmap.md`).
6. In-game apply test (user present); optional one more supervised sweep → +240.
- Contract additions to draft on approval (`docs/contracts/ui-backend.md`, no `apps/ui`
  edits): populate `GpuSensors.voltage_mv`, add `dwell_quality`, `GpuApplyStatus.verification`,
  `workload_context`.
