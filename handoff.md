# Nidavellir — Session Handoff

How to pick this up cold. State as of 2026-06-16, `master` (clean, latest commit
`8667bf0`). Deep NvAPI struct details live in `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Latest backend checkpoint (2026-06-16) — F1c follow-up: Phase B captures a bounded below-knee TAIL (commit 8667bf0) — pure, no hardware
- **Why**: the FIRST confirmed knee-seeking run (2026-06-16, PASS-PARTIAL) found the real knee at **~1025 mV**
  (Phase B started 1056 mV below the 1062 Phase-A floor, descended to 1025 where **pcf dropped 1.000→0.437 in
  one 6 mV bin** — a steep knee). But Phase B stopped at that FIRST off-cap point → only **1** useful point →
  synthesis correctly still reported `POWER-BOUND COLLAPSE`. Stop policy, not budget, was the limiter.
- **What landed**: `descend_phase_b` now captures a BOUNDED below-knee tail. After the knee crossing (first
  `pcf < POWER_BOUND_FRAC` point) it keeps descending until `PHASE_B_MIN_USEFUL_POINTS` (=2) useful off-cap
  points OR `PHASE_B_POST_KNEE_TAIL_BINS` (=3) post-knee bins, then stops cleanly as new
  `BracketStop::KneeTailComplete`. ≥ 2 useful → existing synthesis differentiates; 1 → honest collapse.
- **Safety precedence preserved**: crash / abort / global drain / verifier failure / instability are checked
  BEFORE the tail and stop immediately; floor / `--phase-b-probes` / global `--max-probes` still bound it.
- **Confirmed-run safety (PASS)**: exit 0, no TDR/crash/reboot, `reset_to_stock` ran, no persist/apply
  (`gpu_applied.json`/`boot_flag.json` absent; state files byte-identical), monotone writer `positive_offsets=0`.
- **Unchanged**: Phase A, synthesis, bind-seeking, safety chain (writer/verifier/Safe Loop/reset_to_stock/
  floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; no new CLI flag.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only (BracketStop variant + 2 consts + `descend_phase_b`
  tail loop + dry-run plan line + tests).
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **203 / 0** (8 new).
- **Hardware STILL BLOCKED**. Next: NEW dry-run-only review of the bounded-tail plan output, before any
  further confirmed run. Non-goals unchanged. Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-16) — F1c follow-up: Phase B continues BELOW Phase-A floor (commit 9f35ec0) — pure, no hardware
- **What landed**: a budget-efficiency fix for F1c Phase B, acting on the dry-run-only review finding.
  Phase B now CONTINUES below the deepest bin Phase A already explored for the focused target instead of
  re-probing the inert top bins. Files: `crates/service/src/gpu_power_sweep.rs` only (no `main.rs` / no new
  flag). Pure: no hardware, no `--confirm`.
- **Why**: on this card's fine VF curve (~6–7 mV/bin), the `0ef4e68` Phase B re-started from the cap, so
  `--phase-b-probes 12` reached only ~1006 mV — re-covering Phase A's 1075/1068/1062 and stopping ~75 mV
  above the ~930 mV knee. Now each Phase-B probe lands on a new, deeper bin.
- **How**: two pure helpers in the orchestrator — `phase_a_deepest_bin` (focus target's deepest retained
  Phase-A bin) + `phase_b_start_below` (highest real bin strictly below it). Fallbacks: no Phase-A history
  for the target → safe-start cap; Phase A already at the floor → Phase B skipped cleanly. Dry-run plan adds
  a `knee start` line.
- **Unchanged**: Phase A, `descend_phase_b`, synthesis, safety chain (writer/verifier/Safe Loop/
  `reset_to_stock`/floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; global
  `--max-probes` master cap.
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **195 passed / 0
  failed** (5 new). No hardware.
- **Hardware STILL BLOCKED**. Next: a NEW dry-run-only review confirming the improved plan (`knee start`
  line, deeper reach). Budget sizing remains the operator's call (~20+ Phase-B probes to cross a ~930 mV
  knee from a ~1062 mV Phase-A floor); this patch makes each probe count, default budget unchanged (12).
  Non-goals unchanged. Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — F1c power-bound knee-seeking two-phase prototype IMPLEMENTED (commit 0ef4e68) — pure, no hardware
- **What landed**: an OPT-IN (default OFF) two-phase knee-seeking mode for `build-frontier`, the
  design-audit direction `NEED DEEPER POWER-BOUND DESCENT`. Phase A = the existing single-pass descent
  (byte-for-byte unchanged when OFF; extracted into `run_target_descents`). Phase B (only after a Phase-A
  power-bound collapse) detects the plateau (median power-bound clock), picks the lowest candidate target
  ≥ plateau, and descends THAT target deeper to cross the knee, then merges + re-synthesizes via the
  existing `synthesize_forge_profiles`. New CLI flags `--power-bound-knee-seeking` + `--phase-b-probes N`
  (default None → 12). Global `--max-probes` stays the master cap.
- **Why**: the validated `0996769` collapse only walked the top ~13 mV (bins `1075/1068/1062`), ~130 mV
  above the card's operating voltage, so the VF ceiling was inert and pcf stayed 1.000 — an honest
  diagnostic for a SHALLOW descent, not proof no frontier exists. Detail in `decisions.md` (top entry).
- **Knee model**: above the knee `pcf >= 0.95` (ceiling inert — keep descending); knee = first pcf drop
  below 0.95; clean deep stop at `pcf <= 0.50`; below-knee tail feeds Brokkr's / Deep Calm; Godforge =
  highest sustained off-cap clock (the knee region), NOT the highest requested clock. No knee ⇒ the honest
  `PowerBoundCollapse` is preserved.
- **Files**: `crates/service/src/gpu_power_sweep.rs` (helpers + `descend_phase_b` +
  `build_frontier_two_phase` + `FrontierLimits` fields + `validate_limits` + dry-run plan lines + wiring +
  tests); `crates/service/src/main.rs` (2 CLI flags + parse test).
- **Safety surfaces UNCHANGED** (diff audited): monotone writer, verifier gates, Safe Loop, `reset_to_stock`
  (runs after every build, both paths), floor/cluster derivation, per-target cap, warm-start default OFF,
  persistence/knowledge writes, power-limit / TDP / clock-lock.
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **190 passed / 0
  failed** (17 new). No dry-run / `--confirm` / hardware.
- **Hardware STILL BLOCKED**. Next: a SEPARATE dry-run-only review of the Phase-B plan output (no
  `--confirm`). A later confirmed run must be a bounded knee-seeking shape (one focused target descended
  deep past ~930 mV), NOT a same-config rerun. Non-goals unchanged: no power-limit/TDP, no clock-lock, no
  persistence/apply, no safety-chain change.

## Backend checkpoint (2026-06-15) — F1b power-bound collapse classification FIRST CONFIRMED HARDWARE VALIDATION (commit 0996769) — PASS
- **One supervised confirmed run** (operator present) validating `0996769`; HEAD = `origin/master` = `4880153`,
  tree clean; fresh worktree-local binary (built after `0996769`, not the stale main-repo target). Confirming
  dry-run gate passed first. Command: `build-frontier --confirm --max-targets 7 --max-probes 21
  --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`. **Exit 0; ~5.7 min.**
- **Safety PASS**: no TDR / crash / driver reset / black-screen / reboot; `reset_to_stock` ran; GPU back at
  stock/idle. After: `gpu_applied.json` / `boot_flag.json` absent; `safe_loop.json` idle/disarmed
  (`safe_mode:false`, no new crash/blacklist entry, mtime touched by startup recovery only); `forge_state.json`
  / `gpu_knowledge.json` / `heartbeat.txt` byte-unchanged; tree clean. Every probe `write_mode=monotone_static`,
  `positive_offsets=0`; no overshoot veto.
- **Mechanics**: 19 probes / 17 dwells; `--max-probes 21` not exhausted; **6 of 7 targets characterized**. 1920
  dropped on a benign verifier `LiveMismatch` at the start bin (verifier worked, no crash, run-variance); 1890
  hit a later `LiveMismatch`, kept its deepest verified bin. Descended to 1062/1068 mV. All dwells PowerLimited,
  `power_capped_frac=1.000`, ~199 W, ~1784–1825 MHz.
- **Reporting honesty PASS**: no `BoundBinding`, no `reason=Clock`. **Clock arm retirement validated** — probes
  with avg clock within 30 MHz of target (which would FALSE-bind under v2) correctly did NOT bind, descended to
  `PerTargetCap`. **`LeftPowerRegime` validated negatively** — evaluated each eligible probe, `bound=false
  reason=None`, no target stopped by it (none had pcf ≤ 0.50). **`PowerBound`/`PowerBoundCollapse` validated
  positively** — 6 points `[power-bound]`; reported `6 power-bound / 0 useful`; explicit *"power-bound collapse
  — cannot build a differentiated VF frontier under this workload/regime"*; frontier classes = `POWER-BOUND
  COLLAPSE (best-effort, NOT a differentiated VF frontier)`. Godforge/Brokkr's/Deep Calm collapsed to one
  best-effort point (1815 MHz / 199 W, R=0.00), confidence 0.21, all flagged not-differentiated — no fake
  frontier.
- **Verdict PASS** (safety + reporting honesty). Physical frontier still not useful here: the card is pinned at
  the ~199 W cap, now reported honestly. **Caveats**: `LeftPowerRegime` validated negatively only (a positive
  stop needs pcf ≤ 0.50, which this regime never produces); 1920 LiveMismatch is benign run-variance.
- **Direction**: accept the patch; **keep hardware BLOCKED for this same config**; do NOT repeat the run, do NOT
  bump the per-target cap, do NOT tune power-limit/TDP/clock-lock yet. Next is a design decision: a workload that
  doesn't saturate the ~199 W cap, candidate targets below the power-bound plateau, or a design pass for
  presenting "cannot differentiate under this workload/regime." Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — F1b power-bound collapse classification IMPLEMENTED (commit 0996769) — pure, no hardware
- **Commit `0996769 fix(service): classify power-bound frontier collapse`** (pushed to `origin/master` with
  the docs entry). Scope: `crates/service/src/gpu_power_sweep.rs` ONLY. Implements the SIMPLIFY patch from the
  audit below. No hardware, no `--confirm`, no dry-run.
- **Retired bind-seeking's Clock arm** — `classify_binding` is regime-only: a target binds (stops early) ONLY
  when it LEFT the power-limited regime (`power_capped_frac <= 0.50`). Removed `BIND_OVERSHOOT_MHZ`,
  `BindThresholds.overshoot_mhz`, `BindReason::Clock`; start-bin eligibility guard kept. Renamed
  `BracketStop::BoundBinding → LeftPowerRegime`.
- **Power-bound classification** (`POWER_BOUND_FRAC = 0.95`): pure `is_power_bound_frac` / `is_power_bound_point`
  / `useful_frontier_points` / `frontier_power_bound_collapse`. A pcf-saturated stable dwell = VALID raw
  bracket, NOT useful clock-frontier diversity. Invalid/missing pcf → not power-bound (fail open for
  classification), still fail-CLOSED for regime binding.
- **Collapse-aware synthesis**: `synthesize_forge_profiles` excludes power-bound points; < 2 useful → FLAGGED
  best-effort + diagnostic *"power-bound collapse — cannot build a differentiated VF frontier under this
  workload/regime"* (new `ForgeProfiles.power_bound_excluded` / `power_bound_collapse`). Catches the jittery
  ~1798–1819 MHz @ pcf 1.0 plateau the exact-distinct-clock check missed. NO power-bound points → legacy path
  byte-for-byte unchanged. RESULT output now prints per-point `pcf` + a `frontier classes` summary.
- **Safety surfaces UNCHANGED** (diff audited — no protected symbol added/removed): monotone writer, verifier
  gates, Safe Loop, `reset_to_stock`, floor/cluster derivation, per-target cap, warm-start default OFF,
  persistence/knowledge writes, power-limit/clock-lock.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **173 passed / 0 failed** (was 169: +5 power-bound tests, +2 regime tests, −3 retired clock tests).
- **Hardware STILL BLOCKED**: pure code/test patch. Next confirmed run only AFTER reviewing the new
  classification/reporting in a fresh dry-run; the same-config rerun remains not recommended. Full detail in
  `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — build-frontier / F1b algorithm audit — verdict SIMPLIFY (read-only, pre-implementation)
- **Read-only audit only.** Inspected `crates/service/src/gpu_power_sweep.rs` + continuity docs. **No code edit,
  no tests, no `build-frontier`, no `--confirm`, no hardware, no VF write, no stress, no power sweep** were run.
  This entry records the audit conclusion BEFORE implementation so the next patch has a clear north star.
- **Verdict: SIMPLIFY CURRENT DIRECTION.** Not a redesign, not a full rollback. Do **not** run more hardware
  before the next pure/pure-ish patch; do **not** keep adding bind-seeking complexity. The discovery → descent
  → synthesis skeleton is still valid; the drift is concentrated in **bind-seeking / `BoundBinding`** semantics.
- **North star (unchanged)**: 1) find max core clock / top sustainable target; 2) start at max safe voltage /
  safe VF ceiling; 3) dwell holding the target; 4) descend real VF voltage bins while sustainable; 5) stop each
  target on an explicit reason — unstable, verifier failure, crash/abort/budget drain, voltage floor, per-target
  cap, **or power-bound regime/collapse**; 6) next lower target; 7) build a real stable frontier; 8) synthesize
  Godforge / Brokkr's / Deep Calm **only from meaningful (non-power-bound) data**.
- **Load-bearing — KEEP**: hardware-derived floor; cluster selection / sane-core VF filtering; real-bin descent;
  per-target probe cap; the typed hard/soft stops; confidence gate + best-effort fallback; monotone static-base
  writer; verifier gates; Safe Loop; `reset_to_stock`; no profile persistence during build-frontier.
- **Bind-seeking conclusion**: `BoundBinding` is the wrong **combined** abstraction — it mixes a **bad Clock
  arm** (false-binds under power cap: the cap, not the descent, sets the clock) with a **useful Regime arm**
  (`power_capped_frac <= 0.5`, card left power-limited behavior). The v2 start-bin guard was useful + validated
  but did NOT solve physical frontier collapse. The confirmed v2 run (`bf02971`) stayed power-limited
  throughout: `power_capped_frac=1.000`, ~199 W, ~1798–1819 MHz, confidence 0.21, profiles collapsed. ⇒ the
  remaining issue is **regime / power-bound collapse, not scheduler depth or per-target probe count**.
- **Decision**: stop treating a `Clock` bind as sufficient evidence of useful VF-bound behavior when
  `power_capped_frac` is saturated; **retire/neutralize the Clock arm**; keep the regime signal reclassified as
  **`LeftPowerRegime`**; add a first-class **`PowerBound` / `PowerLimitedPlateau` / `PowerBoundCollapse`**
  classification; strengthen synthesis so power-bound samples are valid raw brackets but **not** useful
  clock-frontier diversity. NOTE: the current collapse detector keys on exact-distinct clocks, so a jittery
  ~1798–1819 MHz plateau reads as ~6 "distinct" clocks and the warning never fires — even with bind-seeking OFF
  synthesis silently emits a falsely-differentiated frontier. Re-key it on pcf saturation.
- **Power-limited sample treatment**: valid bracket = yes; useful clock-frontier point = no (when pcf
  saturated); synthesis = raw input yes but excluded from differentiated selection; collapse diagnostic = yes
  (primary). Mark, don't discard (still a valid warm-start seed).
- **Next safest patch (pure / mostly pure, `gpu_power_sweep.rs`)**: add/rename stop classifications
  (`VoltageFloor`, `DepthCap`, `LeftPowerRegime`, `Unstable`, `VerifyFailed`, `Crashed`, `Aborted`,
  `BudgetDrained`, `PowerBound`/`PowerBoundCollapse`); strengthen `synthesize_forge_profiles` (detect the
  pcf-saturated plateau; don't treat jittery ~1800 MHz clocks as a real differentiated frontier; emit
  *"power-bound collapse — cannot build a differentiated VF frontier under this workload/regime"*); add tests
  over synthetic samples; **do not touch the hardware-writing path.** Optionally add read-only power-headroom
  telemetry.
- **Explicit non-goals**: no confirmed hardware run; no power-limit/TDP changes; no clock-lock changes; no
  target-generation redesign yet; no warm-start default change; no per-target cap change; no Safe Loop / reset /
  writer / verifier changes; no profile persistence / knowledge write change; no version bump.
- **Hardware: BLOCKED** until the power-bound classification + collapse report land and a fresh dry-run shows
  the new diagnostics. Re-running the proven-uninformative config is not justified. Full audit rationale in
  `decisions.md` (top entry); index line in `memory.md`.

## Backend checkpoint (2026-06-15) — bind-seeking F1b v2 strictness FIRST CONFIRMED HARDWARE VALIDATION — mechanism PASS / frontier PARTIAL (commit bf02971)
- **Supervised confirmed run** (operator present; this session). Validates `bf02971 fix(service): tighten
  bind-seeking stop criteria`; docs already at `3b8774c docs: record bind-seeking v2 strictness`
  (HEAD = origin/master = `3b8774c`, working tree clean).
- **Fresh worktree binary built first.** The worktree-local `target/debug/nidavellir-service.exe` was ABSENT
  and the only existing binary was STALE (main-repo `target/debug`, built 2026-06-07 — predates the entire
  bind-seeking feature). Built `cargo build -p nidavellir-service` → worktree binary created AFTER the build
  (mtime after the build marker, size differs from stale); the **stale main-repo binary was NOT used**;
  working tree stayed clean (target/ is gitignored).
- **Dry-run gate passed** (no `--confirm`): bind-seeking ENABLED; v2 strict "start bin is NOT bind-eligible"
  note; thresholds `avg_clock_overshoot <= 30 MHz` + `power_capped_frac <= 0.50`; coverage-bounded scheduler;
  `max_probes=Some(21)`; `max_probes_per_target=Some(3)`; targets `[1935,1905,1875,1845,1815,1785,1755]`;
  first-pass bins `[1075,1068,1062]`; warm-start OFF; no applied-profile warning; no Safe Loop conflict
  warning; dry-run no-op line (no Safe Loop arm / apply / dwell / VF write).
- **Confirmed command**: `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`.
- **Safety: PASS.** Exit 0; **no TDR / driver reset / black-screen / reboot / crash**. Startup recovery clean
  ("clean boot, nothing to restore"); `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). After: `boot_flag.json`/`gpu_applied.json` **absent**; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` **unchanged** (no persistence, no knowledge write); `safe_loop.json` stayed **idle**
  (`safe_mode:false`), size unchanged — only mtime bumped by startup recovery, no new blacklist/crash entry.
  GPU back at stock idle.
- **Probe result**: **15 probes/dwells** total; **all 7 targets physically characterized**. 6 stopped via
  **`BoundBinding`** (1935, 1905, 1875, 1845, 1815, 1785, each `probes_used=2`); 1 stopped via
  **`PerTargetCap`** (1755, `probes_used=3`). No target dropped. Global `--max-probes 21` **not exhausted**
  (15/21). No `overshoot_veto`. Every probe `write_mode=monotone_static`, `positive_offsets=0`.
- **v2 mechanism — PASS (start-bin guard validated).** **Every** 1075 mV start bin reported
  `eligible=false / bound=false` — v2 definitively prevents start-bin binding. All 7 targets descended to
  **1068 mV**; **1755** descended further to **1062 mV**. Earliest binding occurred only AFTER a real bin
  descent (the 6 bound targets bound at the 2nd bin, 1068, `reason=Clock`). Bind telemetry present on every
  probe: `eligible / bound / reason / avg_clock_mhz / p5_clock_mhz / power_capped_frac`. Regime arm
  (`power_capped_frac <= 0.5`) never fired (pcf saturated at 1.000) — binding came solely via the avg-clock
  path.
- **Physical frontier — PARTIAL (did NOT de-collapse).** All dwells were **PowerLimited** with
  **`power_capped_frac=1.000` throughout** (199 W flat). Achieved clocks clustered **~1798–1819 MHz**; all
  targets converged to the same power-bound operating point (vf_bin 1068, 1755 at 1062). Synthesis confidence
  stayed **0.21** (R=0.00); Godforge/Brokkr's/Deep Calm collapsed to **~1800 MHz / 199 W**.
- **Interpretation**: v2 fixed the **procedural** start-bin binding bug (the v1 collapse). The **remaining
  collapse is power/regime-related, not scheduler depth and not the per-target cap** — the card is pinned at
  the 199 W limit, so every target above the power-bound clock degenerates to one point. **Do NOT repeat the
  same hardware run; do NOT increase the per-target cap as the immediate next action (1755 went deeper to 1062
  and still produced ~1811 MHz / 199 W); do NOT jump directly to risky power-limit / clock-lock changes.**
- **Next design work (analysis first, no further confirmed run yet)**: add/adjust **regime-aware binding
  semantics**; distinguish a true `Clock` bind from a `PowerLimitedPlateau` / `PowerBoundCollapse`; consider
  **vetoing `Clock` binding when `power_capped_frac` is saturated near 1.0**; add explicit collapse
  diagnostics and power-headroom / power-drop telemetry. **Stop for analysis before any further confirmed
  hardware run.**
- **Scope of this entry**: docs/continuity only (`handoff.md`, `decisions.md`, `memory.md`). The only commands
  run this validation were one debug build, one dry-run, and one confirmed run — **no code edits, no tests, no
  further hardware**.

## Backend checkpoint (2026-06-15) — bind-seeking F1b v2 strictness IMPLEMENTED + PUSHED (commit bf02971) — now hardware-validated (see entry above)
- **Commit `bf02971 fix(service): tighten bind-seeking stop criteria`** — pushed to `origin/master`
  (HEAD = origin/master = `bf02971`). Scope: `crates/service/src/gpu_power_sweep.rs` ONLY (no other file).
- **Why**: the v1 supervised hardware run (`--bind-seeking`, this session) was **safety/mechanics PASS but
  semantic PARTIAL** — v1 allowed `BoundBinding` on the **first/start bin at 1075 mV**, so every viable target
  stopped immediately with no descent; the frontier stayed degenerate/single-bin (all ~1075 mV / ~199 W) and
  Forge synthesis confidence stayed low (~0.21).
- **v2 changes** (`classify_binding` now returns `BindDecision` and takes an `eligible` flag):
  - **Start bin is NOT bind-eligible** — a target must descend ≥1 real VF bin before `BoundBinding` can fire;
    earliest bind = the **2nd probed real VF bin** (`bind_eligible(probes_before, cur_bin, start_bin)`).
  - **Clock binding uses the AVERAGE/achieved clock** (`avg_clock_mhz - target <= 30`), not p5/sustained;
    p5 remains telemetry/reporting only; zero/absent avg fails closed (no clock binding).
  - **Regime arm unchanged** (`power_capped_frac <= 0.5`) but **invalid/missing cap_frac fails closed**
    (NaN / <0 / >1 → no regime binding, via `valid_cap_frac`).
  - **Bind telemetry** added (live run, per verified+stable probe): `eligible`, `bound`, `reason`
    (`BindReason::None/Clock/Regime`), `avg_clock_mhz`, `p5_clock_mhz`, `power_capped_frac`.
  - **Dry-run** prints the new `binding eligibility: start bin is NOT bind-eligible …` caveat + v2 threshold
    wording (`avg_clock_overshoot <= 30 MHz`).
- **Stop precedence PRESERVED**: crash → abort → budget drain → verifier failure → dwell instability →
  **binding** → per-target cap → floor (only the binding arm is now gated by eligibility).
- **Safety boundaries UNCHANGED**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  persistence/profile apply, hardware-floor derivation; **warm-start default remains OFF**.
- **Validation before commit (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p
  nidavellir-service` **169 passed / 0 failed** (new: start-bin-not-eligible, avg-not-p5, invalid-cap-frac
  fail-closed, skips-start→binds-at-second). **Dry-run only** (no `--confirm`) passed:
  `build-frontier --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`.
  No hardware boundary crossed.
- **Hardware validation: DONE 2026-06-15** (see the FIRST CONFIRMED HARDWARE VALIDATION entry above) via
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking` — mechanism PASS (start-bin guard), frontier PARTIAL (still power-limited / collapsed).

## Backend checkpoint (2026-06-14) — bind-seeking F1b v1 IMPLEMENTED + PUSHED, hardware-validated PARTIAL → superseded by v2 (commit 08f745e)
- **Commit `08f745e feat(service): add opt-in bind-seeking to build-frontier`** — pushed to `origin/master`
  (HEAD = origin/master = `08f745e`). Scope: `crates/service/src/gpu_power_sweep.rs` +
  `crates/service/src/main.rs` ONLY. This builds the bind-seeking direction set after the `5248758` run.
- **Feature**: new opt-in CLI flag **`--bind-seeking`** + `FrontierLimits.bind_seeking` (default **OFF** —
  absence reproduces current behavior byte-for-byte). Per target, the descent stops at the first verified +
  dwell-stable **binding** point instead of walking a fixed number of bins, so each target can contribute a
  distinguishable point (vs the 1832–1867 MHz / 194–199 W collapse in the `5248758` run).
- **Binding signal v1 (Clock + regime)** — pure `classify_binding`: a verified + stable probe binds iff EITHER
  `sustained - target <= BIND_OVERSHOOT_MHZ (30)` (sustained = p5 if present, else avg) OR
  `power_capped_frac <= BIND_CAP_FRAC (0.5)` (card no longer power-pinned).
- **Power-drop deliberately NOT a v1 stop-condition**: no top-power reference tracking. May become
  telemetry/log later — never a binding/stop rule in v1.
- **Scheduler**: adds `BracketStop::BoundBinding` — a CLEAN stop (`is_hard_failed()==false`), carry-forward
  eligible when it recorded a `lowest_verified_mv`. Binding is checked ONLY on a verified+stable sample, AFTER
  the failure arms. **Precedence preserved**: crash/hard-failure → aborted → global budget drained →
  verifier-failure/unverified → dwell-unstable/silent-error → **then binding** → then per-target cap / floor.
- **Interactions**: `--max-probes` stays the hard global cap; `--max-probes-per-target` stays the per-target
  attempt/depth cap; bind-seeking may stop EARLIER than the per-target cap; **warm-start stays default OFF**.
- **Safety boundaries UNCHANGED**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  hardware-derived floor, persistence/profile apply — none modified. No power-limit / clock-lock changes.
- **Validation before commit (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p
  nidavellir-service` **165 passed / 0 failed** (incl. classifier 30/0.50 boundaries, the full failure
  precedence, BoundBinding carry-forward, and dry-run reporting). **Dry-run only** (no `--confirm`):
  `build-frontier --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`
  → exit 0; printed `bind-seeking: ENABLED`, thresholds (`clock_overshoot <= 30 MHz OR power_capped_frac
  <= 0.50`), the live-metrics caveat, warm-start OFF; **no Safe Loop arm / apply / dwell / VF write**.
- **Hardware validation: NOT yet run for `08f745e`.** Next step (separate, operator-present, NOT this task): a
  clean confirming dry-run, then the supervised confirmed run
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`.
- **Scope of this entry**: docs/continuity only. No code/test/hardware commands run in the docs pass.

## Latest backend checkpoint (2026-06-13) — F1b `--max-probes-per-target` FIRST CONFIRMED HARDWARE VALIDATION — coverage PASS / profile PARTIAL (commit 5248758)
- **Supervised confirmed run** (operator present, after a clean confirming dry-run that showed no plan drift;
  HEAD/origin/master at `5248758`; required commits `47f39be`/`f90981d`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 14 --max-probes-per-target 2 --safe-start-cap 1075`
  — **warm-start OFF**. **Exit 0; ~4 min; no TDR, no driver reset, no black-screen, no reboot, no crash markers.**
- **Safety state**: startup recovery clean; Safe Loop armed/cleared **per probe**, ended **idle/disarmed**;
  `reset_to_stock` ran ("GPU restored to stock; no profile applied or persisted"). After:
  `boot_flag.json`/`gpu_applied.json` **absent**; `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt`
  unchanged (no forge-state persistence, no knowledge write); `safe_loop.json` content/size **unchanged**
  (idle, `safe_mode` false) — mtime touched only, **no new blacklist/crash entry**. GPU back at stock idle.
- **Coverage — PASS (the fix works)**: **13 hardware dwells spread across all 7 targets** (not depth-first on
  one). 6 targets stopped cleanly via **`PerTargetCap`** (`probes_used=2` each, bins **1075 + 1068 mV**);
  global `--max-probes 14` was **not exhausted** (13 used). The per-target cap successfully prevented one
  target from consuming the whole budget — the exact fix vs the prior 34-on-1935 depth-first run.
- **Target 1905 dropped** after its 1st probe: ceiling 1075 mV → **`LiveMismatch`**, **`overshoot_veto=true`**,
  `eff_cov=0.963`, 1 unexplained zero — a conservative verifier rejection (neighbors 1935 `NoDownCapNeeded`
  and 1875 `VerifiedCurve` passed), not a hardware fault. **6/7 produced frontier points.**
- **Writer/verifier**: every probe `write_mode=monotone_static`, `positive_offsets=0`; verdicts
  `NoDownCapNeededCeiling` (1935) + `VerifiedCurve` (1875–1755). No VF persistence; no Safe Loop / reset /
  verifier / writer regression. Voltage band shallow only — **1075 and 1068 mV**; did **not** touch
  875/868/862/856/850 mV (the cap-2 stop holds at the top two bins).
- **Profile goal — PARTIAL**: lower targets did **not** produce distinct clock/power. Achieved clocks
  clustered **1832–1867 MHz**, power **194–199 W**; the live plateau stayed ~1890 MHz with `overshoot`
  growing 1875:+30 → 1755:+135 → the near-stock flatten does not govern the achieved clock.
  Godforge/Brokkr's/Deep Calm collapsed to one point (**1860 MHz / 194 W**, target 1755); FORGE confidence
  stayed low (best 0.21, single-trial → best-effort synthesis).
- **Key conclusion**: **shallow near-stock coverage at 1075/1068 mV is non-binding on this hard power-capped
  RTX 3060 Ti** — the ceiling does not materially govern the achieved clock at that high-voltage band.
  `--max-probes-per-target` **solved budget distribution, not binding/differentiation**.
- **Direction (next design = bind-seeking F1b)**: do NOT repeat the same flags; do NOT use per-target cap 3
  next; do NOT enable warm-start next; do NOT jump straight to power-limit/clock-lock changes. Instead, per
  target: **keep descending while the point is stable but non-binding, and stop when it actually BINDS** (the
  ceiling materially governs the clock), fails the verifier/dwell, or hits the global/per-target cap — the
  goal is the **first useful (binding) point per target**, not the deepest voltage. **No further hardware
  commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `decisions.md`, `memory.md`). No code/test/IPC/hardware
  change in this pass.

## Backend checkpoint (2026-06-13) — Hardware-derived / bin-based floor FIRST CONFIRMED HARDWARE VALIDATION — PASS (commit f90981d)
- **Supervised confirmed run** (operator present, after a clean bounded dry-run on a fresh debug build;
  HEAD/origin/master at `c99dbf1`+`f90981d`; required commits `23b70c4`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 34 --safe-start-cap 1075 --warm-start-brackets`.
  `--max-probes 34` was chosen so the descent reaches **868 mV** (one real bin below the old 875 mV floor)
  but stops BEFORE **862 mV** (a historical reboot-zone / blacklisted bin). **Exit 0; no TDR, no driver
  reset, no black-screen, no reboot, no crash markers.**
- **Safety state**: startup recovery clean ("clean boot, nothing to restore"); Safe Loop armed at startup
  and cleared back to **idle**; `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). After: `boot_flag.json`/`gpu_applied.json` **absent** (as before); `forge_state.json`/
  `gpu_knowledge.json`/`heartbeat.txt` unchanged; `safe_loop.json` **byte-identical** (idle, `safe_mode`
  false, size unchanged) — mtime touched at run start only, **no new blacklist/crash entry**
  (`consecutive_crashes` still 1, `crash_log` still `["unrelated"]`). GPU back at stock idle.
- **Probe budget — 34 hardware dwells, ALL on target 1935** (ceilings 1075→868 mV; benign_zeros 27→60).
  The per-target `bracket_carry` line logs `probes_used=35` with `stop_reason=BudgetExhausted` — the 35th
  increment is the scheduler ATTEMPTING the next step (→862 mV) and finding the budget spent; **no
  `ceiling_mv=862` line exists; 862 mV was never set or dwelled.** Targets 1905/1875/1845/1815/1785/1755
  were NOT physically characterized this run (budget exhausted on the hardest target; each logged a
  `probes_used=1` bookkeeping entry, no dwell, "no stable point in safe range — dropped").
- **Voltage coverage**: reached **875 mV** (probe 33) and **868 mV** (probe 34), both
  `NoDownCapNeededCeiling`, `eff_cov=1.000`, `overshoot=0`; **did NOT reach 862 mV**. Safety goal met —
  validated **one real bin below the old 875 mV floor** while **avoiding the historical 862/855 mV
  reboot-zone bins** on this first bounded run. Warm-start observed: 1935 started from cap 1075
  (`warm_started=false`); 1905 carried 1935's bracket (`warm_started=true`, `start_mv=893` = 868 + 25 mV).
- **Writer/verifier**: every probe `write_mode=monotone_static`, `positive_offsets=0`, `down_caps=0`, no
  `overshoot_veto`, all verified `NoDownCapNeededCeiling`, `eff_cov=1.000`.
- **IMPORTANT interpretation**: this validated **safe WRITING/descent of the static VF ceiling down to
  868 mV** — it did NOT prove core stability when actually RUN at 868 mV. The GPU stayed **power-limited
  (~198 W)** the whole descent, so the ceiling was **non-binding** (the core's power-governed operating
  point was already at/below each ceiling → no down-cap needed). Frontier point: 1935 target →
  **1839 MHz @ 868 mV vf_bin, 198 W** (p5 1800). **PASS for the first bin-based floor validation;
  partial/insufficient for profile synthesis** — FORGE confidence stayed low (best 0.21), single
  sustainable clock (1800 MHz), so Godforge/Brokkr's/Deep Calm collapsed identical.
- **Direction**: do NOT jump straight to `--max-probes 40`. `--max-probes 35` would deliberately touch
  **862 mV** if the goal is pure reboot-zone boundary mapping (operator present; NB the 862 blacklist
  entry is keyed `freq=1755`, so a 1935-target ceiling at 862 would not match it — Safe Loop is the
  backstop, not prevention), but that is NOT the best path for useful profiles. **Primary next step: pivot
  to F1b / multi-clock characterization, and/or a regime that makes the ceiling actually BIND (e.g. raise
  the power limit) before descending deeper** — since the descent was power-limited, deeper ceilings add
  reboot-zone exposure for ~zero characterization gain. **No further hardware commands were run.**

## Backend checkpoint (2026-06-13) — Hardware-derived / bin-based build-frontier floor SHIPPED (commit f90981d)
- **Change** (`f90981d feat(service): derive build-frontier floor from real VF bins`, on `origin/master`):
  the hardcoded active **875 mV** descent floor is GONE. `build-frontier` now derives the floor from the
  GPU's real VF / core-cluster voltage bins — `hw_floor_mv = seed.cluster_v_min_mv` (lowest real
  graphics-core bin). No replacement fixed floor (no 825/800); `FRONTIER_LOWEST_SAFE_MV` deleted.
- **Bin-based descent**: `FrontierDescent` carries `bins_desc` (real descending bins) built by
  `derive_descent` from `CoreSeed.cluster_bins_mv`; `descend_target` walks **real bins only** — it does
  NOT invent 25 mV requested voltages off the curve. Warm-start snaps its margin to the **conservative
  real bin ≥ the requested margin target** and **never starts below the previous `lowest_verified_mv`**
  (B1). `--max-probes` remains the exposure cap; `--warm-start-brackets` stays default OFF; no new flag.
  Empty/underivable bin domain → **fail closed before any hardware write**. Dry-run prints the
  hardware-derived floor, the exact descent bin sequence, the real bin count, and worst-case dwells.
- **Scope**: only `crates/service/src/gpu_power_sweep.rs`. **Unchanged**: monotone static-base writer,
  verifier gates, Safe Loop, `reset_to_stock`, persistence, profile apply. `cargo check` clean;
  `cargo test -p nidavellir-service` **142 passed**.
- **Historical note**: the older `1755 @ 875` validations below remain valid for that point but are NO
  LONGER the active floor; future runs may descend **below 875** where real bins exist + budget allows.
- **NOT hardware-validated yet.** The descent may now reach **below the historical ~855 mV reboot zone**.
  **Suggested next operational step (DRY-RUN ONLY, no `--confirm`):**
  `build-frontier --max-targets 7 --max-probes 70 --safe-start-cap 1075 --warm-start-brackets` → review
  the hardware-derived floor, exact bin sequence, worst-case dwell count, and whether `--max-probes` is
  enough. ONLY THEN consider a separate supervised `--confirm` run (operator present + able to reboot).

## Backend checkpoint (2026-06-13) — Warm-start voltage-bracket carry-forward SHIPPED + HARDWARE-VALIDATED (commits 23b70c4, 6f2f061)
- **Feature** (`23b70c4 feat(service): add warm-start bracket carry-forward`, on `origin/master`):
  a **generic** scheduler primitive (NOT Godforge-specific) for ordered hardest→easiest core-clock
  voltage descents. An easier target reuses the previous harder target's verified + dwell-stable
  bracket as its descent start (`lowest_verified_mv + 1 step`), skipping dominated high-voltage
  probes. Opt-in CLI flag **`--warm-start-brackets`, default OFF** — no runtime behavior changes
  unless the flag is passed. Preserves the monotone static-base VF writer, verifier gates,
  `overshoot_veto`, Safe Loop, `reset_to_stock`, persistence/profile apply, and the 875 mV floor.
  Safety constraints B1/B2/B3 (see `decisions.md`).
- **Hardware validation (2026-06-13) — PASS.** Supervised
  `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075 --warm-start-brackets`
  on a fresh debug build, after a clean bounded dry-run (warm-start shown ENABLED;
  `gpu_applied.json`/`boot_flag.json` absent; state mtimes unchanged). **Exit 0; no TDR/reboot**;
  startup recovery clean; Safe Loop armed/cleared per probe; `reset_to_stock` ran ("GPU restored to
  stock"). After: `boot_flag.json`/`gpu_applied.json` absent; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` mtime touched at run start (idle/disarmed, size
  unchanged); GPU back at stock idle.
- **Scheduler behavior (33 probes, all 7 targets produced points):**
  - 1935 (first) started at cap 1075, descended to floor 875 (boost-top NoDownCapNeeded everywhere),
    `lowest_verified=875`.
  - **B2 exercised — 1905**: inherited an optimistic 900 mV warm start from 1935's boost-top bracket,
    failed verify at 900 (`LiveMismatch`, `overshoot_veto=true`) → **fell back ONCE to cap 1075**,
    re-descended, target preserved (point at 1075). Fallback did not loop and did not fire on a drain.
  - 1875/1845 collapsed to cap (`lv+step ≥ cap`); 1815/1785/1755 warm-started below cap and skipped
    1 / 2 / 3 dominated high-V probes respectively.
  - **Probes: 33** vs **32** baseline (≈ flat raw) but **−5 vs the equivalent from-cap descent (38)**
    for an identical frontier — net of one B2-fallback probe. Benefit is modest on this RTX 3060 Ti
    because mid targets stop early on verify-axis residual overshoot regardless of start voltage.
- **Frontier preserved.** Critical low-V region re-validated exactly: **`1755 @ 900`**
  `NoDownCapNeededCeiling`, plateau **1665..1755**, overshoot 0, dwell stable; **`1755 @ 875`**
  `NoDownCapNeededCeiling`, plateau **1620..1755**, overshoot 0, ≈**1755 MHz @ 875 mV, ≈176 W**.
  Every probe `write_mode=monotone_static`, `positive_offsets=0`. Residual single-bin 15 MHz
  overshoot on non-1755 targets persists (safe verify-axis early stop, as before). FORGE synthesis
  low confidence (best 0.21) — unrelated Wilson metric.
- **B1/B2/B3 held in live logs** (B2 actually exercised). Feature is hardware-validated as **safe
  behind the opt-in flag**.
- **Observability follow-up** (`6f2f061 feat(service): surface build-frontier scheduler logs`):
  log-only. `run_build_frontier` previously emitted only `result.profiles.log`; now emits the
  scheduler/frontier `result.log` (bracket carry / warm-start / fallback / probes_used) FIRST, then
  the synthesis log, deduping shared lines (pure `ordered_frontier_logs` helper + 2 unit tests). No
  tuning behavior changed. Closes the validation finding that bracket telemetry existed but was
  invisible in CLI output.
- **Keep `--warm-start-brackets` default OFF** until more runs justify flipping it. **Next** (later,
  optional): 1–2 more warm-start runs; a benign-zero-only (NoDownCapNeeded) bracket-seeding refinement
  so a boost-top bracket doesn't mis-seed the next sub-boost target (the 1905 B2 case); broader
  frontier/profile confidence work. **Do NOT mix with profile persistence yet.**

## Backend checkpoint (2026-06-12) — Phase 2B.2-c: monotone static-base VF writer HARDWARE-VALIDATED (commit 8503182)
- **Milestone — supervised hardware revalidation of the monotone static-base VF ceiling writer.**
  After a clean bounded dry-run on a fresh `origin/master` debug build at `8503182`
  (`gpu_applied.json`/`boot_flag.json` absent; state mtimes unchanged), the user approved one
  confirmed run: `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075`.
- **Safety: exit 0, no TDR, no reboot.** Startup recovery clean ("clean boot, nothing to restore");
  Safe Loop armed for the VF writes and cleared; `reset_to_stock` ran at the end ("GPU restored to
  stock; no profile applied or persisted"). After the run: `boot_flag.json` absent,
  `gpu_applied.json` absent; `forge_state.json` / `gpu_knowledge.json` / `heartbeat.txt` unchanged
  (build-frontier never persists); `safe_loop.json` mtime touched at run start, size unchanged
  (startup-recovery bookkeeping only). GPU back at stock idle (nvidia-smi ~66 W / 7% util / 200 W
  limit). The audited safety contract held end-to-end on real hardware.
- **Functional: monotone writer confirmed working.** Every one of the **32 probes** logged
  `write_mode=monotone_static` with **`positive_offsets=0`** (static-base-anchored monotone-down
  offsets only) over `static_base_points=132`.
- **Primary fixed case — `1755 @ 900 mV` no longer overshoots.**
  - OLD: raw_cov 0.891, eff_cov 1.000, **`overshoot_veto=true`**, plateau **1755..1845**, result
    **`LiveMismatch`** (blocked).
  - NEW: `positive_offsets=0`, eff_cov 1.000, **overshoot=0**, plateau **1665..1755** (max 1755),
    veto not triggered, result **`NoDownCapNeededCeiling`** (pass). Plateau max dropped 1845 → 1755
    exactly; overshoot collapsed to 0.
- **Run continued to `1755 @ 875 mV` and it verified**: `NoDownCapNeededCeiling`, overshoot=0,
  plateau **1620..1755**, dwelled (~19 s); achieved ≈ **1755 MHz @ 875 mV, ≈179 W**. The `1755`
  ceiling descent shows overshoot decaying cleanly to 0 from 950 mV down (950/925/900/875 all
  overshoot=0).
- **Minor residual (not a blocker for the writer fix)**: a few **non-1755** probes at low ceilings
  still show a single-bin **15 MHz** overshoot with `overshoot_veto=true` (e.g. 1905@1050, 1875@950,
  1845@975, 1815@950, 1785@950). All `1755` probes are overshoot=0.
- **Unrelated note**: FORGE synthesis reported low confidence (best 0.21 < 0.85) → best-effort
  profiles. This is the single-trial Wilson confidence metric, not a writer/overshoot issue.
- **Next technical phase**: design **warm-started voltage-bracket reuse** for F1b / Godforge (carry
  a verified bracket forward across targets to cut probes). **Do NOT mix this with persistence /
  profile apply yet** — keep build-frontier non-persisting until the bracket-reuse design lands.

## Backend checkpoint (2026-06-11) — Phase 2B.2-c: FIRST confirmed run (SAFE, 0 points) + c.1 stock-equivalent verifier fix (IMPLEMENTED, not committed)
- **Milestone — first supervised hardware run executed.** After a Fable 5 blocker audit (GO) and a
  clean bounded dry-run (fresh worktree debug build of 6881cd7; `gpu_applied.json` absent; mtimes
  unchanged), the user approved and we ran
  `build-frontier --confirm --max-targets 1 --max-probes 6 --safe-start-cap 1075`.
  **Safety: exit 0, no TDR, no reboot (~10 s)**; startup recovery clean; Safe Loop armed before the
  VF write and cleared after; `reset_to_stock` fired on the verify reject and again at run end; no
  profile applied/persisted (`gpu_applied.json` absent; `forge_state`/`gpu_knowledge` mtimes
  unchanged; `safe_loop.json` re-saved by startup recovery = bookkeeping only); GPU back at stock
  idle (1% util / 44 °C / 64 W). The full audited safety contract held on real hardware.
- **Functional: 0 frontier points.** The only probe — target=1935 (the stock cluster boost top),
  ceiling 1075 mV — was rejected by the verify gate: `verify=LiveMismatch offsets=20/27
  plateau=1935..1935 overshoot=0`, so the descent stopped before any dwell. Root cause: flatten-to-
  boost-top needs ZERO offset on the 7 top bins already at 1935 in stock → the ≥90% offset-presence
  gate under-counts. The frequency evidence (plateau exactly at target, no overshoot) showed the
  ceiling WAS in effect.
- **c.1 fix (this checkpoint, code+tests only, NO hardware run yet)**: narrow stock-equivalent
  acceptance — pure `is_stock_equivalent_ceiling` in `gpu_verify.rs`, consulted ONLY on a
  `LiveMismatch`, accepting only when: target within tol of the caller-supplied stock boost top;
  ALL offsets readable; NO bin above target (overshoot rejected even within tol); all bins within
  tol below target; every zero-offset bin EXACTLY at target (offset 0 ⇒ GetStatus shows stock base;
  correct flatten writes `target−base`, so only `base==target` explains a missing offset). Carried
  as service-internal `LiveCeilingEval.stock_equivalent` + `stock_equivalent_bins`;
  **`CurveVerification` IPC untouched**. `eval_ceiling_evidence`/`classify_live_ceiling` gain
  `stock_top_mhz: Option<u32>`; `verify_applied_curve` passes `None` (byte-identical);
  `real_probe_step` passes `Some(seed.stock_boost_max_mhz)`, accepts `VerifiedCurve ||
  stock_equivalent`, logs the branch as `verify=StockEquivalentCeiling stock_equiv_bins=N`.
  Safe-Loop/reset/abort flow in the probe unchanged. Condition 1 is DIRECTIONAL (`target ≤ top &&
  top − target ≤ tol`) — a target above the stock top is an overclock, never stock-equivalent.
- **Files**: `crates/service/src/gpu_verify.rs`, `crates/service/src/gpu_power_sweep.rs`.
- **Tests**: `cargo check` clean · service **109/109** (+11: first-run reproduction accepted;
  plateau-miss / any-overshoot / below-boost-top / zero-offset-not-exact / no-stock-top /
  unreadable-offset all rejected; normal VerifiedCurve never consults the path; fully-degenerate
  all-zero-at-target accepted; tol-boundary accept@15 / reject@16 on bin-freq AND target-vs-top;
  directional above-top reject) · core 46/46.
- **Next**: re-run the bounded DRY-RUN on the rebuilt binary, then (user approval required) the
  SAME bounded `--confirm` — expect the 1935-target probe to verify stock-equivalent and dwell
  (~6 probes, ~120 s). 11D (persisted stock base / exact-offset proof) still deferred.

## Backend checkpoint (2026-06-08) — F1b Phase 2B.2-c.0: first-run limiter flags (pushed, 6881cd7)
- **Bounded first run.** `build-frontier` gains `--max-targets N`, `--max-probes N`,
  `--safe-start-cap MV` so the first supervised QA validates the pipeline without the full 84-dwell
  plan. Dry-run + confirmed both honor them; defaults preserve the full plan.
- **Behavior**: `--max-targets` truncates to the top N; `--safe-start-cap` lowers the descent start
  to the cap when below the derived cluster top (never raises above it, never below the crash floor);
  `--max-probes` hard-stops total probe executions (short-circuits remaining, then resets to stock +
  clears the flag). FAIL CLOSED on absurd values (0; cap ≤ crash floor; non-numeric/missing).
- **Pure helpers**: `FrontierLimits` / `validate_limits` / `apply_frontier_limits` (gpu_power_sweep);
  `parse_frontier_limits` (main.rs). Dry-run prints a `limits` line + the capped dwell budget.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/main.rs`. **No IPC/contract/
  core/apps-ui/Safe-Loop/gpu_apply/nvml_gpu/Phase-3/11D change; no auto-apply; no persistence; no
  hardware.**
- **Tests**: `cargo check` clean · service **95/95** (+7: validate/apply/max-probes-cap + 3 parse
  tests) · core 46/46.
- **Dry-run QA** (stock, no --confirm, no state-file writes — mtimes unchanged):
  `build-frontier --max-targets 1 --max-probes 6 --safe-start-cap 1075` → targets=[1935],
  descent 1075→875 mV (9 bins), 6 dwells (~120 s, capped by --max-probes). NB: the soft-max warning
  still cites the derived cluster top (1150 mV) even when --safe-start-cap lowers the effective start
  (1075) — accurate about the curve, cosmetic next to the capped descent.
- **Next — Phase 2B.2-c (supervised hardware QA, separately gated)**: a bounded `--confirm` run
  (e.g. the flags above) with the user present and able to reboot. 11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.4: stock core VF cluster seeding (IMPLEMENTED, not pushed)
- **Refines b.3.** b.3's generic guard rejected absurd values but still let `safe_start` = global max
  of all sane points (1150 mV on the 3060 Ti — the hard-cap boundary / a non-core point). b.4 derives
  safe_start/boost from the actual contiguous core VF cluster instead.
- **`select_core_cluster`** (pure, `gpu_power_sweep.rs`): sort sane points by voltage; split into
  contiguous runs where voltage gap ≤ 60 mV; pick the LARGEST (ties → lowest voltage = dense core);
  FAIL CLOSED if < 8 points. `derive_core_seed` seeds boost/safe_start from the cluster top and
  reports isolated high-V outliers above it. b.3 generic hard guards (500..3500 MHz, 600..1150 mV)
  retained.
- **Dry-run diagnostics** now print raw/retained/rejected counts, rejected extremes, selected
  core-cluster mV+MHz range, outliers-above count, stock reference (cluster top), safe_start source,
  and a WARNING when a profile appears applied (`gpu_apply::load_applied()`).
- **Files**: `crates/service/src/gpu_power_sweep.rs` + docs. **No IPC/contract/core/apps-ui/Safe-Loop/
  gpu_apply/nvml_gpu/Phase-3/11D change; no auto-reset; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **88/88** (cluster tests: isolated
  1150 rejected; ends-at-1075→1075; legit-1150→1150; empty/ambiguous fail-closed; targets seed from
  cluster not outlier; diagnostics report cluster range) · core 46/46.
- **Stock dry-run QA PENDING the user's manual reset to stock** (this patch does NOT auto-reset).
  Then run `nidavellir-service.exe build-frontier` (no --confirm) and confirm: no arm/apply/dwell/
  VF-write, no state-file mtime change, plausible targets, safe_start = stock core cluster top,
  applied-profile warning if not reset. **`--confirm` remains forbidden until reviewed.**
- NB: b.3 + b.4 are both UNCOMMITTED — the eventual commit bundles them unless split. Future: NVML
  `max_clock_info(Graphics)` could corroborate boost (nvml_gpu.rs frozen here).

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.3: core-domain seeding guard (IMPLEMENTED, not pushed)
- **Safety fix.** The first `build-frontier` dry-run (read-only) caught a seeding bug:
  `run_build_frontier` derived candidate clocks + safe_start from the UNFILTERED global max of
  `read_vf_curve_modern()` (includes non-core / memory-domain points) → bogus plan (targets
  7001..6311 MHz, safe_start 1237 mV). The dry-run gate blocked it with zero hardware risk.
- **Guard** (pure, `gpu_power_sweep.rs`): `sane_core_points` keeps freq ∈ [500,3500] MHz & voltage ∈
  [600,1150] mV; `derive_core_seed` seeds boost/sustained/safe_start from sane points only, records
  rejected max freq/voltage, soft-warns (>3200 MHz / >1125 mV), FAILS CLOSED (Err) if no sane points
  or a derived value exceeds a hard guard. `run_build_frontier` aborts (no arm/apply/dwell/VF-write)
  on Err or any candidate target > 3500 MHz. Consts are sanity guards, NOT tuning targets.
- **Files**: `crates/service/src/gpu_power_sweep.rs` + docs. **No IPC/contract/core/apps-ui/
  Safe-Loop-behavior/gpu_apply/nvml_gpu/Phase-3/11D change; no auto-reset; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **86/86** (+5 guard tests:
  sane_core rejects 7001/1237 & keeps plausible; seed uses sane max not global max; fail-closed on no
  sane points; targets never > hard max; soft-limit warnings) · core 46/46.
- **Dry-run QA (read-only, no --confirm, no state writes — all 4 mtimes unchanged)**: 132 raw VF
  points → 88 sane-core retained, 44 rejected (incl. 7001 MHz / 1237 mV); boost~1935 MHz; targets
  [1935,1905,1875,1845,1815,1785,1755]; 1150→875 mV step 25 (12 bins); 84 worst-case dwells
  (~1680 s); WARNING safe_start 1150 mV > soft max 1125 mV. **NB**: the live curve is in an APPLIED
  state, so the numbers reflect the applied curve, not stock; a stock read (reset first) would be
  cleaner, and safe_start 1150 mV is high for a 3060 Ti core (~1075) → review before --confirm.
- **`--confirm` remains forbidden** until this fixed plan is reviewed. **Next — Phase 2B.2-c**
  (supervised hardware QA, separately gated): optionally reset to stock first for a clean plan,
  re-review the dry-run, then `--confirm` with the user present and able to reboot. 11D deferred.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.2: real probe + supervised build-frontier (CODE ONLY, not run, not pushed)
- **Real Windows probe `real_probe_step`** (the `build_frontier` seam under `--confirm`):
  abort/boundary guard → snap vbin to a real VF bin → arm Safe Loop → `apply_vf_ceiling(bin,target)`
  → read-only verify via shared `classify_live_ceiling` (+ 11C diag log) → on not-VerifiedCurve
  reset+clear+return → `load_and_measure` dwell → clear flag → `measured_to_probe` + set `vf_bin_mv`.
  Dwell CRASH → reset to stock + set `abort` so remaining probes short-circuit (run drains safely);
  a normal Unstable/unverified only stops that clock's descent.
- **`run_build_frontier(store, confirm)`** + console `build-frontier` (main.rs): always prints the
  `plan_frontier` plan. Dry-run (no `--confirm`) = read-only (no arm/apply/dwell/VF-write, no startup
  recovery). `--confirm` = startup recovery (parachute) first, then `build_frontier` with the real
  probe, then ALWAYS `reset_to_stock` + clears the flag. **No auto-apply; no forge_state; no
  gpu_knowledge writes.**
- **Conservative first-run consts** (review the printed dry-run plan before any run): lowest_safe=875
  mV (above the ~855 mV known reboot), 25 mV step, 30 MHz clock step, 0.90 floor; idle Unconstrained
  regime clamped → PowerLimited (no OC on a first run); sustained ≈ curve top freq; confidence 0.21.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/main.rs`. **No IPC/contract
  /core/apps-ui/Safe-Loop-behavior/gpu_apply/nvml_gpu/Phase-3/11D change. Hardware path NOT executed.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **81/81** (+1 `--confirm` arg parse)
  · core 46/46. `real_probe_step`/`run_build_frontier` are hardware → not unit-tested; the abort
  short-circuit PATTERN is covered by the 2B.2-b.1 fake-probe test.
- **Commands**: dry-run `nidavellir-service.exe build-frontier`; confirmed (DO NOT RUN until QA)
  `nidavellir-service.exe build-frontier --confirm`.
- **Next — Phase 2B.2-c (supervised hardware QA, separately gated)**: run the dry-run, review the
  plan, then `--confirm` with the user present and able to reboot; verify gpu_applied.json /
  forge_state.json unchanged, boot flag armed/cleared per probe, abort on TDR. 11D deferred to after
  Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.1: seeding + dry-run plan + vf_bin (IMPLEMENTED, not pushed)
- **Pure prep for 2B.2-b.** Exposed `classify_live_ceiling` / `LiveCeilingEval` / `CurveDiag` as
  `pub(crate)` in `gpu_verify.rs` (intra-crate visibility only — NO IPC/contract change) so the
  future transient-ceiling probe reuses one classification path.
- **Pure seeding** in `gpu_power_sweep.rs`: `derive_descent(curve_bins, lowest_safe, step) ->
  FrontierDescent` (safe_start = top live bin, clamped ≥ operator crash floor) + read-only
  `plan_frontier(targets, &descent, dwell_ms) -> FrontierPlan` (worst-case dwell count + wall-time +
  safety notice). Targets via existing `classify_regime` / `candidate_clocks`.
- **Internal `ProbeSample.vf_bin_mv: Option<u32>`** (NOT IPC): the actually-applied snapped bin.
  `probe_to_point` records `vf_table_voltage_mv = vf_bin_mv.or(descent vbin)`; `measured_to_probe`
  leaves it None (the real probe fills it after the apply in 2B.2-b.2).
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/gpu_verify.rs`. **No real
  probe; no `apply_vf_ceiling`/`load_and_measure`; no `build-frontier` subcommand / `--confirm`; no
  Safe-Loop arm/clear; no startup-recovery wiring; no forge_state / gpu_knowledge writes; no
  Phase-3/11D/apps-ui/core/contract change; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **80/80** (+7 pure: regime→targets,
  derive_descent, plan_frontier estimates, vf_bin propagation + fallback, mapper-leaves-None,
  build_frontier abort short-circuit via fake probe) · core 46/46 (untouched).
- **Next — Phase 2B.2-b.2 (NOT started, separately gated)**: real `#[cfg(windows)]` probe closure
  (arm Safe Loop → `apply_vf_ceiling(vbin,target)` → `classify_live_ceiling` verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe` + set `vf_bin_mv`) + supervised
  `build-frontier --confirm` console subcommand (dry-run default via `plan_frontier`; runs startup
  recovery; print/log-only, no auto-apply, no persistence). Then supervised hardware QA (2B.2-c).
  11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-a: shared live-ceiling classifier (IMPLEMENTED, not pushed)
- **Pure refactor** in `gpu_verify.rs`: extracted `classify_live_ceiling(live, ceiling_idx,
  ceiling_mv, target, tol)` (read-only; offset-readback evidence build) + pure
  `eval_ceiling_evidence(target, anchor_idx, &expected, tol)` (runs the UNCHANGED offset-presence
  `classify_curve` gate + 11C `compute_curve_diag`) → `LiveCeilingEval`. `verify_applied_curve` now
  routes through it.
- **Behavior identical**: `VerifyAppliedProfile` output is byte-for-byte unchanged (same classifier,
  diagnostic, inputs); only inline duplication removed. Offset-presence stays the gate; plateau spread
  stays diagnostic; voltage never affects classification. This is the shared path the 2B.2-b
  transient-ceiling probe will reuse to verify a JUST-applied ceiling (not the persisted profile).
- **Files**: `crates/service/src/gpu_verify.rs` only. **No core/contract/`apps/ui`/Safe-Loop/synthesis
  /Phase-3/11D change; no real probe; no `apply_vf_ceiling`/`load_and_measure`; no `build-frontier`
  subcommand; no hardware.** Pure seeding helpers deferred to 2B.2-b (would be dead code now).
- **Tests**: `cargo check -p nidavellir-service` clean · service **73/73** (+5 `eval_ceiling_*` pure
  tests; all pre-existing verify tests green) · core 46/46.
- **Next — Phase 2B.2-b (NOT started, separately gated)**: real `#[cfg(windows)]` probe closure (arm
  Safe Loop → `apply_vf_ceiling(vbin,target)` → `classify_live_ceiling` verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe`) + supervised `build-frontier --confirm`
  console subcommand (print/log-only; runs startup recovery; no auto-apply). Then supervised hardware
  QA. 11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.1: pure probe-mapping prep (IMPLEMENTED, not pushed)
- **Pure, hardware-free half of Phase 2B.** `measured_to_probe(&Measured, curve_verified, confidence)
  -> ProbeSample` in `gpu_power_sweep.rs` — the seam the real probe closure (2B.2) will use to feed
  `build_frontier`. No hardware I/O; conservative interpretation of already-collected dwell data only.
- **Conservative rules**: Stable→`ProbeOutcome::Stable` ONLY if clock/power quality ≥ Medium AND p5
  present; else (SilentError / Crash / TDR-degenerate, or weak telemetry) → Unstable. p5 preserved
  (0 → None); measured voltage = ramp-filtered avg, None when missing (never 0).
- **Additive schema**: `PowerSweepPoint.target_clock_mhz: Option<u32>` (serde default, no schema
  bump). Phase 2A `probe_to_point` now stamps the target; the single-clock live sweep sets None.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/core/src/ipc.rs`,
  `docs/contracts/ui-backend.md`, decisions/memory/handoff. **No real probe, no `apply_vf_ceiling`,
  no `load_and_measure` loop, no supervised console cmd, no Safe-Loop/synthesis/`apps/ui`/Phase-3/11D,
  no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **68/68** (+7 mapping/target tests) ·
  core **46/46** (+2 serde roundtrip + legacy-load). No hardware run.
- **Next — Phase 2B.2 (NOT started, separately gated)**: the real `#[cfg(windows)]` probe closure
  (arm Safe Loop → `apply_vf_ceiling(vbin,target)` → read-only offset-readback verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe`) + a supervised console subcommand that calls
  `build_frontier` with it behind explicit confirm. Then a supervised hardware QA run. 11D
  (exact-offset stock-base persistence) deferred to AFTER Phase 2B unless QA shows need.

## Backend checkpoint (2026-06-06) — Patch 11C: read-only live VF-ceiling diagnostic (IMPLEMENTED, not pushed)
- **Read-only diagnostic** added to `gpu_verify::verify_applied_curve` (and the `verify-applied`
  console subcommand): pure `compute_curve_diag` over the existing per-point evidence + one
  `LiveSnapshot`. No mutation, no stress, no apply. Classifier semantics UNCHANGED (offset-presence
  gate; live voltage above the VF anchor never downgrades; GetStatus freq stays diagnostic).
- **New evidence**: first modified bin idx/mv, modified vs expected bin count, GetStatus freq-match,
  GetStatus plateau min/max MHz, max target overshoot/undershoot, 3 offset samples (first/anchor/
  highest), and a live snapshot (NVAPI voltage + first NVML clock/power/util/temp/limit/cap). Surfaced
  via additive `Option`/`serde(default)` fields on `ApplyVerificationStatus` + one `apply_verify_diag:`
  log line. Additive IPC documented in `docs/contracts/ui-backend.md`.
- **Files**: `crates/service/src/gpu_verify.rs`, `crates/core/src/ipc.rs`,
  `docs/contracts/ui-backend.md`, `decisions.md`, `memory.md`, this file. **No apply/Safe-Loop/
  synthesis/F1b/`apps/ui`/`nvml_gpu.rs` change. P-state + full ThrottleReasons deferred.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **61/61** (+9 pure diag tests) ·
  core **44/44** (additive serde fields, nothing broken).
- **Runtime QA** (`verify-applied`, read-only — confirmed non-mutating: all four
  `%ProgramData%\Nidavellir\*.json` mtimes unchanged across the run): curve=`VerifiedCurve` (62/64
  offsets present), load=`VerifiedUnderLoad`. Diagnostic revealed `anchor_offset_khz=+255000`,
  `highest_bin_offset_khz=−120000`, GetStatus plateau **1770–1830 MHz** (overshoot 45, undershoot 15),
  live `voltage_mv=1068 clock_mhz=1815 util_pct=6 temp_c=47 power_w=66 cap=200W capped=false`.
  Interpretation: offsets are resident and *curve-flatten-shaped* (big `+` at the 843 mV anchor, `−`
  at the top) → curve IS applied; the plateau spread + overshoot is consistent with BOTH normal
  GPU-Boost behavior AND the open overshoot suspect, but GetStatus idle noise (freq_match 18/64) keeps
  it **non-conclusive** — exactly what 11C was meant to surface. Live voltage 1068 mV ≫ 843 mV anchor
  confirms (again) measured voltage is NOT capped (telemetry only).
- **Exact-offset verification still deferred**: expected offset = `target − stock_base_mhz`, but
  per-point stock base is not persisted and GetStatus freq is idle-unreliable. Future "11D" options:
  persist the pre-apply stock curve, or validate the GetStatus `base` tuple (`StatusEntry.base`,
  currently decoded but discarded in `vfcurve::get_status`). Only then can the overshoot suspect be
  proven/refuted.
- **F1b Phase 2B**: still NOT started; UNBLOCKED by this diagnostic — Phase 2B's `curve_verified`
  gate (offset-readback) is the same axis 11C reports, so the supervised HW run can now log the
  plateau/offset evidence per dwell. Sequence the Codex copy fix + (optionally) 11D before relying on
  exact-offset proof.

## Backend checkpoint (2026-06-06) — Applied voltage semantics (Patch 11A, DOCS ONLY, not pushed)
- **Read-only investigation** confirmed the elastic VF ceiling caps **frequency, not voltage**:
  `apply_vf_ceiling` (`crates/gpu-nvapi/src/lib.rs`) writes per-point FREQUENCY offsets to every
  modern VF point whose table voltage ≥ the selected bin (flatten to `target_mhz`); points below
  are untouched. It writes **no voltage** and does **not** hard-cap measured/rail voltage in any
  P-state. The apply key is the deterministic `vf_table_voltage_mv` (VF/curve bin), re-derived by
  snapping measured voltage UP to the lowest table bin ≥ it (`nearest_vf_bin_at_or_above`).
- **Semantics resolved**: `measured_voltage_mv` / HWiNFO "GPU Core Voltage" are a DIFFERENT (rail,
  load-line/droop) domain and may legitimately read ABOVE the VF bin — idle ~1.075 V and in-game
  ~0.887–0.956 V for an ~850 mV bin are EXPECTED, not a mismatch. `VerifyAppliedProfile` proves
  offset PRESENCE (+ a stored-dwell load axis), nothing about effective voltage. Nidavellir must
  NOT imply a hard voltage cap; a true cap = the legacy voltage-lock (TDR) path → rejected.
- **Patch 11A (this change) = DOCS/CONTRACT ONLY**: updated `decisions.md` (new doctrine entry),
  `docs/contracts/ui-backend.md` (semantics clarification + Codex wording request: drop "MHz @ mV",
  use "target" + "VF bin", keep measured voltage separate), `memory.md`, this file. **No backend
  code, no `apps/ui`, no apply/verify change, no F1b Phase 2B, no hardware.**
- **Open suspect (deferred, read-only-testable — Patch 11C, not started)**: offsets are computed as
  `target − GetStatus_base` and GetStatus under-reports freq at idle → a plateau applied at idle may
  land above `target` (consistent with observed ~1815–1830 MHz vs ~1785, on top of normal 15 MHz
  boost-bin quantization). To be confirmed by a future read-only live diagnostic — NOT changed here.
- **Does NOT block F1b Phase 2B** (it already keys on the VF bin + offset-readback VerifiedCurve
  gate); sequence the Codex copy fix + (optional) 11C live diagnostic before the supervised HW run.

## Backend checkpoint (2026-06-06) — F1b Phase 2A: simulated multi-clock loop (DONE, not pushed)
- **`build_frontier(candidate_clocks, &FrontierDescent, &ForgePolicy, probe: impl Fn(u32,u32)->
  ProbeSample)`** in `gpu_power_sweep.rs` proves the multi-clock outer loop, per-target voltage-bin
  descent, stopping rules, known-unsafe boundary, frontier assembly, and synthesis wiring **without
  hardware**. The injected probe closure is the only seam to (future) hardware.
- **Loop rules**: descend from `safe_start_mv` by `voltage_step_mv`, never below `lowest_safe_mv`
  (known-crash floor as config); keep deepest stable; stop on first `Unstable`; stop/drop on
  simulated `curve_verified=false` (Phase-2B Patch-A gate); drop a clock with no stable point.
  Partial frontier allowed; empty → synthesis all-`None` (safe). Points record `vf_table_voltage_mv`
  (deterministic bin); measured voltage stays telemetry.
- **No hardware wired**: no `load_and_measure`, no `apply_vf_ceiling`, no VF write, no stress, no
  Safe Loop interaction, no real power sweep. New types/fn `#[cfg(windows)] #[allow(dead_code)]`.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only. No IPC/persistence/`apps/ui` change.
  `cargo check` clean · service **52/52** (+8 sim; 3060 Ti 1830/1815/1740 + 4090 2880/2860/2700
  proven through the loop).
- **F1b Phase 2B (next, NOT started)**: fill the real probe closure — apply ceiling at the bin →
  Safe-Loop-armed `load_and_measure` dwell → offset-readback `VerifiedCurve` gate → map to
  `ProbeSample`; wire `build_frontier` into a **supervised/approval-gated** entry point; feed
  `candidate_clocks(...)` from a live `classify_regime`; add `target_clock_mhz` to points if needed.
- **Phase 3 (future)**: knowledge re-key to `(target_clock, vf_table_voltage_bin)` + global
  voltage-floor crash boundary; backward-compatible `gpu_knowledge.json` migration.

## Backend checkpoint (2026-06-06) — F1b Phase 1: policy-driven multi-clock synthesis (DONE, pushed)
- Pure, service-internal in `gpu_power_sweep.rs`. **`ForgePolicy`** centralizes thresholds —
  Balanced `brokkrs_min_clock_frac=0.98` / `deep_calm_min_clock_frac=0.90` / `confidence_threshold=
  0.85`; Conservative (0.99/0.92/0.95) and Aggressive (0.97/0.85/0.70) presets.
- **`synthesize_forge_profiles(frontier, &ForgePolicy)`** now applies clock floors:
  Godforge = highest **sustainable** clock (prefers `p5_clock_mhz`, falls back to `clock_mhz`;
  ties→lowest power); **Brokkr's = max R within the Brokkr's clock floor** (real trade: clock<gc,
  power<gp); Deep Calm = max MHz/W within the Deep Calm floor. **Selection never uses measured
  voltage** — `vf_table_voltage_mv` stays the deterministic apply axis. **Single-clock collapse**
  detected + logged (still returns all three). **4090 doc ambiguity resolved: Brokkr's = 2860**
  (max-R-within-floor).
- Added Phase-2 helpers (pure, `#[allow(dead_code)]` until wired): `Regime` enum,
  `classify_regime(...)`, `candidate_clocks(...)`.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only. No IPC, no `apps/ui`, no Safe Loop,
  no hardware path. `cargo check` clean · service **44/44** (3 F1a tests unchanged + 9 F1b).
- **F1b Phase 2 (next, NOT started)**: real multi-clock measurement loop over the safe flatten
  sweep — build a **simulated/inject outer-loop scaffold first** (test loop/knowledge/stopping
  without a GPU), then a **supervised, approval-gated** hardware run; verify the ceiling per dwell
  (Patch A offset readback); SyntheticDwell context only; add `target_clock_mhz` to points then.
- **Phase 3**: re-key knowledge by (target_clock, vf_table_voltage_bin) + global voltage-floor
  crash boundary; backward-compatible `gpu_knowledge.json` migration.

## Backend checkpoint (2026-06-06) — Forge action consolidation audit (recorded, no code change)
- Backend has **two engine generations**. **Canonical Forge GPU core path = `gpu_power_sweep.rs`
  (Power Sweep)**: `set_core_offset_mhz` + `apply_vf_ceiling` (elastic VF ceiling), game-power
  render dwell, Safe-Loop-guarded, **no voltage lock**. Apply via `ApplyPowerGodforge/Brokkrs/
  DeepCalm`. **F1b must extend ONLY this engine.**
- **Legacy (voltage-lock, TDR risk)**: `gpu_sweep_real.rs` (Real Sweep — `lock_core_voltage_mv`
  L239/L370, ALU load) and `gpu_forge_all.rs` (Forge Everything — fixed `CORE_VOLTAGE_MV=900`
  lock L193, VRAM around a fixed-voltage core) + the legacy `ApplyGodforge/Brokkrs/DeepCalm` trio.
  → hide from normal UI, schedule removal AFTER F1b. Keep IPC methods wired for now (no mid-stream
  break).
- **Memory/VRAM** (`gpu_mem_sweep.rs`): no core voltage lock, but runs independent of the forged
  core. **VRAM tuning remains future work and must adapt to the forged core curve** (run after
  core VF forge + validation, never define/destabilize it). Advanced Diagnostic until redesigned.
- **Action audit table + answers**: see this session's audit; frontend request in
  `docs/contracts/ui-backend.md`; rationale in `decisions.md`. No code removed, no `apps/ui` change.

## Backend checkpoint (2026-06-06) — Patch B load-state classification (IMPLEMENTED, pushed)
- Adds an orthogonal **LOAD axis** to `ApplyVerificationStatus`: `load_state: LoadVerification`
  (`NotEvaluated/VerifiedUnderLoad/TelemetryInsufficient/LoadMismatch/WorkloadStateMismatch
  (reserved)/LoadVerificationFailed`) + `load_reason`, `telemetry_match`, and diagnostic dwell
  fields (`p5_clock_mhz`, `min_clock_mhz`, `avg/min/max_measured_voltage_mv`,
  `voltage_sample_count`, `voltage_quality`, `telemetry_quality`). `status` stays the curve axis.
- **Source**: existing synthetic-dwell stats only — NO new stress run. `gpu_power_sweep::
  load_restored_progress()` (read-only, reads `forge_state.json`) → `find_applied_point` matches
  by label→named slot (Godforge/Brokkr's Best/Deep Calm) with a clock check, fallback = unique
  `points` entry; ambiguous→None. `classify_load`: curve must be VerifiedCurve; `p5_clock ≥
  target−30 MHz` (two bins) AND `telemetry_quality ≥ Medium` → VerifiedUnderLoad; voltage is
  telemetry-only (implausible→TelemetryInsufficient); `stable=false`→LoadMismatch; bad power→
  LoadVerificationFailed; missing p5/quality→TelemetryInsufficient. `effective_status` derivation:
  load upgrades VerifiedCurve→VerifiedUnderLoad, never downgrades; LiveMismatch stays LiveMismatch.
- **Files**: `crates/core/src/ipc.rs` (LoadVerification + fields), `crates/service/src/gpu_verify.rs`
  (find_applied_point, classify_load, effective_status, fill_load_axis, tests),
  `crates/service/src/gpu_power_sweep.rs` (load_restored_progress), `docs/contracts/ui-backend.md`.
  Additive only; `verify-applied` stays read-only.
- **Tests**: check clean · service 35/35 (+10 load tests).
- **Runtime QA** (`verify-applied`, read-only): curve=VerifiedCurve(63/65), forge_state loaded
  (17 pts), matched Brokkr's slot, **load_state=TelemetryInsufficient** ("legacy point without
  dwell quality" — the persisted point predates the richer-dwell-stats patch), status=verified_curve.
  No writes (`gpu_applied.json` + `forge_state.json` mtimes unchanged). To get VerifiedUnderLoad a
  fresh sweep (HW, supervised) must produce a point carrying the new dwell stats.
- **Limitations**: WorkloadStateMismatch reserved (live real-game context = future); load axis only
  as good as the persisted dwell stats. **Next: Forge Action Consolidation.**

## Backend checkpoint (2026-06-06) — Applied curve verifier, Patch A (IMPLEMENTED, pushed)
- **Read-only `VerifyAppliedProfile` IPC** + new `crates/service/src/gpu_verify.rs`. Answers
  "does the live modern VF curve match the applied profile?" → `CurveVerification` =
  `NotApplicable | MetadataOnly | VerifiedCurve | LiveMismatch | VerificationFailed`.
- **Table-to-table only**: re-derives the deterministic ceiling bin the same way apply does
  (`nearest_vf_bin_at_or_above(core.voltage_mv)` — NOT measured voltage); reads
  `read_vf_curve_modern` (GetStatus) + `vf_get_point_khz` (offset corroboration, logged only).
  Rule: points with `mv ≥ ceiling` should read `target ±15 MHz`; ≥90% match (and ≥1) →
  VerifiedCurve, else LiveMismatch; empty/unmappable → VerificationFailed.
- **Read-only**: never applies/reapplies/writes/stresses. Patch B (telemetry/load),
  Patch C (workload context, stock fingerprint, ExternalUnknown) NOT implemented.
- **Files**: `crates/core/src/ipc.rs` (enum `CurveVerification`, `ApplyVerificationStatus`,
  `VerifyAppliedProfile` request, `ApplyVerification` response), `crates/service/src/gpu_verify.rs`,
  `main.rs` (mod), `ipc_server.rs` (handler), `docs/contracts/ui-backend.md`. Additive only.
- **Tests**: `cargo check -p nidavellir-service` clean · service 26/26 (+7 verifier pure tests).
- **Read-only runtime path (2026-06-06)**: added console subcommand
  `nidavellir-service.exe verify-applied` (`run_verify_only` in `main.rs`) — runs the verifier
  with NO `run_startup_recovery`/`spawn_heartbeat`/`reapply_on_boot`/pipe server, so **no apply,
  no VF write**. Prints `ApplyVerificationStatus` JSON + the `apply_verify:` log. Proven
  non-mutating (`gpu_applied.json` mtime unchanged across a run).
- **Patch A.1 — offset-based verification (2026-06-06) — DONE**: runtime QA proved GetStatus
  actual-freq is unreliable at idle (under-reported the plateau 31/65 while the flatten offsets
  were resident 63/65). `classify_curve` now gates on the **GET-control offset readback**
  (`vf_get_point_khz`): a point ≥ ceiling counts as flattened if it carries a **non-zero** offset
  (presence, not exact value — per-point stock base isn't persisted); ≥90% → VerifiedCurve;
  unreadable offsets → VerificationFailed (safer than mismatch). GetStatus freq match stays a
  logged diagnostic (`getstatus_freq_match=...`). Re-ran `verify-applied` → **VerifiedCurve**
  (offset_match 63/65, getstatus 31/65), no write (`gpu_applied.json` mtime unchanged). Service
  25/25, check clean. **Known caveat**: presence-only offset check can't yet distinguish a
  Nidavellir flatten from an external tool's offsets (ExternalUnknown = Patch C); and it can't
  detect an offset that's present but wrong-valued (would need persisted stock base).
- **Unblocks**: Patch B (load classification) can reuse the applied `PowerSweepPoint` dwell stats.

## Backend checkpoint (2026-06-05) — Richer dwell stats (IMPLEMENTED, pushed)
- Second patch off the Sensor Audit. **`PowerSweepPoint` gains optional dwell-quality
  fields**: `min_clock_mhz`/`p5_clock_mhz`, measured-voltage `avg/min/max` +
  `voltage_sample_count`, `dwell_sample_count`/`dwell_duration_ms`, `start/end/avg_temp_c`,
  and `voltage_quality`/`telemetry_quality` (new `DwellQuality` enum in `core/ipc.rs`:
  high/medium/low/unavailable).
- **Voltage stats are ramp-filtered + sanity-checked (500–1250 mV)**; the legacy unfiltered
  voltage max (`volt_mv` → `voltage_mv`/`measured_voltage_mv` + the apply-key snap) is
  **UNCHANGED** (restriction: don't touch the apply-key decision). min/p5 clock from the
  retained post-ramp clock samples; temp from NVML per-sample reads. Per-point
  `dwell_stats:` log line (not per-sample).
- **Files**: `crates/core/src/ipc.rs`, `crates/service/src/gpu_power_sweep.rs`,
  `docs/contracts/ui-backend.md`. No `apps/ui`, Safe Loop, synthesis, or F1b change.
  Additive serde-default fields; `PowerSweepPoint` stays `Copy`; old `forge_state.json` loads.
- **Tests**: `cargo check -p nidavellir-service` clean · core 44/44 · service 19/19.
- **Limitations (next work)**: full NVML limiter reasons deferred (needs `NvmlGpuReading`
  in core); voltage cadence still ~480 ms (≈Medium quality, now surfaced); no per-sample
  timestamps; no hotspot/fan; `arduous_validate` soak path doesn't yet use the richer stats.

## Backend checkpoint (2026-06-05) — Voltage field separation (IMPLEMENTED, pushed)
- First patch off the Sensor Audit decision. **`PowerSweepPoint` now separates
  `measured_voltage_mv` (telemetry) from `vf_table_voltage_mv` (deterministic apply/
  frontier key)**; legacy `voltage_mv` retained for compat/display.
- **Apply path snaps measured voltage → real VF-table bin** (`nearest_vf_bin_at_or_above`
  in gpu-nvapi; `choose_ceiling_mv` in `gpu_apply.rs`) **before `apply_vf_ceiling`** — no
  longer keys the ceiling on raw measured voltage. Logs `voltage_semantics: …`.
- **Backward-compatible**: no schema bump; old `forge_state.json`/`PowerSweepPoint` JSON
  loads new optional fields as `None`; `VfPoint`/`gpu_applied.json` unchanged → apply
  re-snaps at runtime (legacy warning only if the live curve is empty). Additive IPC
  fields documented in `docs/contracts/ui-backend.md`.
- **Files**: `crates/gpu-nvapi/src/lib.rs`, `crates/core/src/ipc.rs`,
  `crates/service/src/gpu_apply.rs`, `crates/service/src/gpu_power_sweep.rs`,
  `docs/contracts/ui-backend.md`. No `apps/ui`, Safe Loop, or synthesis change.
- **Tests**: `cargo check -p nidavellir-service` clean · gpu-nvapi 5/5 · service 15/15.
- **Limitations (next work)**: frequency-only flatten unchanged; the ~1062 mV unfocused/
  desktop state is NOT solved here; richer dwell stats + applied-curve verification pending.

## Backend checkpoint (2026-06-05) — Sensor Quality Audit (Review 2, investigation-only)
- **No code/IPC/UI change.** GPU telemetry sources are right (NVML clock/power/cap/temp/
  util; NVAPI curve). Three structural gaps found:
  1. **Two disconnected telemetry worlds**: "sensor world" (`SensorEngine`/`GpuSensors`,
     **30 s cache, `voltage_mv` hardcoded `None`** → UI never gets GPU voltage) vs
     "sweep world" (`load_and_measure`, NVML 30 ms + NVAPI voltage ~480 ms, stored as
     `fetch_max`). Nothing reconciles them.
  2. **Voltage is the weakest signal**: NVAPI `core_voltage()` **string-parsed**, sparse,
     **max-only**, ramp-unfiltered — then **reused as the deterministic `apply_vf_ceiling`
     threshold** (`PowerSweepPoint.voltage_mv` → `AppliedProfile.core.voltage_mv` →
     `ceiling_mv`). This is the root of 837-vs-869 and makes apply fidelity unprovable.
  3. **One name, three meanings**: `voltage_mv` on `PowerSweepPoint` (measured max),
     `VfCurvePoint`/GetStatus (VF-table), `AppliedProfile.core` (measured, consumed as
     curve threshold).
- **KEY DECISION (see `decisions.md`)**: split voltage into **`vf_table_voltage_mv`**
  (deterministic, the **apply/frontier key**) · **`measured_voltage_mv`** avg/min/max
  (telemetry + HWiNFO cross-check only, never an apply key) · **`effective_rail_voltage_mv`**
  (future). **F1b must NOT key on measured dwell voltage.**
- **Verdicts NOT finalized**: 837-vs-869 ≈ undersampling + bin quantization (expected,
  not apply failure); constant ~1062 mV unfocused/desktop ≈ workload-scoped (P0/3D)
  ceiling leaving other states on stock curve + frequency-only flatten leaving voltage
  uncapped. To be confirmed by the verification work.
- **Other gaps**: perf-limiter only reads `SW_POWER_CAP` (NVML exposes the full
  `ThrottleReasons` set — thermal/voltage/util discarded); no timestamps/p5-clock-dip
  stats; no workload-context tag; no cross-validation (0 mV / 0 W dropouts stored as real).
- **Post-audit sequencing** (F1b stays on hold until 1–3 land): (1) split voltage fields +
  stop keying apply on measured voltage [must-fix]; (2) richer dwell stats + full limiter +
  context tag; (3) finalize Applied Curve Verification (table-to-table GetStatus plateau,
  not vs measured voltage) + verify IPC + `GpuApplyStatus.verification`; (4) F1b on the
  cleaned axis.

## Backend checkpoint (2026-06-05) — Applied Curve Verification review (investigation-only)
- **No code change.** Apply is **write-and-forget** (`apply_core` logs flattened count,
  no readback). `GetAppliedProfile` is **metadata-only** (`gpu_applied.json`, no live
  driver check) — "Applied ✓" = file exists, not curve verified. Verification must use
  the **modern ClkVfPoints GetStatus** path (`read_vf_curve_modern`), not legacy
  `read_curve`/`GetGpuCurve`. The flatten caps **frequency, not voltage** — a high VF bin
  can still be selected. Primitive for verification already exists (`read_vf_curve_modern`),
  just unwired. Feeds directly into the sensor-audit sequencing above.

## Backend checkpoint (2026-06-05) — forge-state persistence
- **F1b is on hold** pending two foundation reviews; cheap lower-clock probe plan is
  NOT approved. Both reviews (persistence/startup + sensor quality) are now done.
- **Shipped**: `forge_state.json` (under `%ProgramData%\Nidavellir`) persists the final
  `PowerSweepProgress` on successful sweep completion (only when a profile exists, so a
  failed sweep can't wipe a good snapshot). Startup seeds `PowerSweepHandle` from it when
  the GPU key (`read_curve().name`) matches; else idle. Fixes a service restart losing
  forged profiles/points/apply buttons. Files: `crates/service/src/gpu_power_sweep.rs`
  (+ `main.rs`, `service_impl.rs` seed both startup paths). Backend-only — no UI, IPC,
  Safe Loop, synthesis, or `gpu_knowledge.json` change.
- **Validation**: `cargo test -p nidavellir-service` → 11/11 pass;
  `cargo check -p nidavellir-service` → no warnings. No GPU stress run.
- **Remaining foundation work (in order)**:
  a) manual restart verification (apply a profile → restart service → UI still shows it);
  b) **must-fix**: split voltage fields (`vf_table_voltage_mv` / `measured_voltage_mv` /
     `effective_rail_voltage_mv`) + stop keying `apply_vf_ceiling` on measured voltage;
  c) richer dwell stats (min/p5 clock, voltage avg/min/max, full `ThrottleReasons`,
     sample_count, timestamps, workload-context tag);
  d) finalize Applied Curve Verification (post-apply GetStatus plateau readback,
     table-to-table; verify IPC + `GpuApplyStatus.verification`);
  e) F1b redesign — only after b–d land and the direction is confirmed; key the frontier
     by (clock + VF-table point), NOT measured voltage.

## Where things stand
- **Brokkr's V1 (continuous per-GPU knowledge): implemented + HW-validated.**
- **V2 (confidence-gated selection): committed (5d72342).** `cargo test` 3/3. The
  gate is now reused as the confidence axis for all 3 product profiles.
- **Product reframe (this session, see `product.md`)**: 3 profiles forged from a
  clock×power frontier. **F1a done** — pure `synthesize_forge_profiles` + tests
  (6/6), not yet wired. **F1b** = produce the real multi-clock frontier.
- Architecture finding: two overlapping sweep engines (safe flatten vs unsafe
  lock-voltage frontier); F1b builds on the flatten one — tech debt to consolidate.
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
1. **Commit F1a** (synthesis + tests + `product.md`/decisions/roadmap) once reviewed.
2. **F1b**: extend the safe flatten sweep to several target clocks → real game-power
   clock×power frontier; knowledge keying by (clock, offset); wire
   `synthesize_forge_profiles` in (replaces the single-clock godforge/brokkrs picks).
3. Then F2–F7 (see `product.md`).
4. In-game apply test (`ApplyPowerBrokkrs`) — consistency in Overwatch; user present.

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
