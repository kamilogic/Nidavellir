# Nidavellir — Decision Log

Durable technical decisions and their rationale. Newest first.

## bind-seeking F1b v2 strictness — IMPLEMENTED + pushed (commit bf02971), NOT yet hardware-validated
- **Decision (2026-06-15)**: tighten bind-seeking after its first supervised hardware run. Commit
  `bf02971 fix(service): tighten bind-seeking stop criteria`, pushed to `origin/master`. Scope:
  `crates/service/src/gpu_power_sweep.rs` only.
- **Why**: the v1 run was **safety/mechanics PASS but semantic PARTIAL** — v1 allowed `BoundBinding` on the
  first/start bin (1075 mV), so every viable target stopped immediately, no descent happened, and the frontier
  stayed degenerate (single-bin, ~1075 mV / ~199 W; Forge confidence ~0.21). The binding test was too
  permissive, and the p5/sustained clock signal could call a target "binding" while the average/achieved clock
  was still materially above target.
- **What changed**:
  - **Eligibility**: the start bin is never bind-eligible; a target must descend ≥1 real VF bin first
    (earliest bind = 2nd probed bin). `classify_binding` now takes an `eligible` flag and returns a
    `BindDecision`; eligibility computed by `bind_eligible(probes_before, cur_bin, start_bin)`.
  - **Clock metric**: clock binding keys off the AVERAGE/achieved clock (`avg - target <= 30`), not
    p5/sustained; p5 stays telemetry/reporting; absent/zero avg fails closed.
  - **Regime arm**: `power_capped_frac <= 0.5` kept; invalid/missing cap_frac fails closed (`valid_cap_frac`:
    NaN / <0 / >1 → no regime binding).
  - **Telemetry**: new `BindReason` + `BindDecision`; per-probe live log of eligible / bound / reason /
    avg_clock_mhz / p5_clock_mhz / power_capped_frac; dry-run reports the start-bin-not-eligible caveat.
- **Rejected/deferred (unchanged from v1)**: power-drop stop-condition still NOT a stopping rule; no
  power-limit / clock-lock changes; per-target cap semantics untouched.
- **Precedence preserved**: crash → abort → budget drain → verifier failure → dwell instability → binding →
  per-target cap → floor (only the binding arm is now gated by eligibility).
- **Safety unchanged**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  persistence/profile apply, hardware-floor derivation, warm-start default OFF.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  169 passed; dry-run only passed (`--max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`); no hardware boundary crossed.
- **Status**: hardware validation NOT yet run for `bf02971`. Next: separate dry-run + supervised confirmed run
  (proposed `--confirm` shape recorded in `handoff.md`); do not auto-run.

## bind-seeking F1b v1 — IMPLEMENTED + pushed (commit 08f745e), hardware-validated PARTIAL → superseded by v2
- **Decision (2026-06-14)**: implement the bind-seeking direction from the `5248758` run as an **opt-in,
  default-OFF** scheduler mode. Commit `08f745e feat(service): add opt-in bind-seeking to build-frontier`,
  pushed to `origin/master`. Scope: `crates/service/src/gpu_power_sweep.rs` + `crates/service/src/main.rs`.
- **Why**: `--max-probes-per-target` (`5248758`) fixed budget *distribution* but not *binding/differentiation*
  — shallow near-stock bins (1075/1068 mV) are non-binding on the power-capped 3060 Ti, so profiles collapsed
  to one point. Walking a fixed bin count cannot tell a useful (binding) point from a non-binding one. The
  fix: per target, **keep descending while stable but non-binding; stop at the first BINDING point**.
- **Binding signal v1 — Clock + regime** (`classify_binding`, pure): a verified + dwell-stable probe binds iff
  EITHER `sustained - target <= BIND_OVERSHOOT_MHZ (30)` (sustained = p5 else avg) OR `power_capped_frac <=
  BIND_CAP_FRAC (0.5)`. Constants `BIND_OVERSHOOT_MHZ=30`, `BIND_CAP_FRAC=0.5`.
- **Rejected for v1 — power-drop stop-condition**: deliberately excluded; it needs top-power reference
  tracking and adds state/risk. Clock + regime is sufficient and self-contained. Power-drop may be logged as
  telemetry later, but is NOT a stop rule in v1. (Alternatives considered earlier: log-only classifier — too
  weak; redefining the per-target cap — overloads validated semantics; power-limit/clock-lock — out of scope.)
- **Scheduler**: new `BracketStop::BoundBinding` — CLEAN (`is_hard_failed()==false`), carry-forward eligible
  when it has a `lowest_verified_mv`. Binding is evaluated ONLY on a verified+stable sample, AFTER every
  failure arm, so precedence is unchanged: crash/hard-failure → aborted → global budget drained →
  verifier-failure/unverified → dwell-unstable/silent-error → **binding** → per-target cap / floor.
- **Interactions / invariants**: `--max-probes` remains the hard global cap; `--max-probes-per-target` remains
  the per-target attempt/depth cap (bind-seeking can stop earlier); **warm-start remains default OFF**. Safety
  boundaries unchanged: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  hardware-derived floor, persistence/profile apply — none touched.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **165 passed / 0 failed** (classifier boundaries, failure precedence, BoundBinding carry-forward, dry-run
  reporting). Dry-run only (no `--confirm`): `--max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking` → exit 0, `bind-seeking: ENABLED`, thresholds + caveat printed,
  warm-start OFF, no Safe Loop arm / apply / dwell / VF write.
- **Hardware validation: NOT yet done for `08f745e`.** Next step (separate, operator-present): clean
  confirming dry-run, then `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`. **No hardware commands were run in this implementation or docs pass.**
- **Scope of this entry**: docs/continuity only (`handoff.md`, `memory.md`, this file).

## F1b `--max-probes-per-target` — FIRST confirmed hardware validation (commit 5248758) — coverage PASS, profile PARTIAL
- **Validation** (2026-06-13): the per-target probe cap (`5248758 feat(service): add per-target probe cap to
  build-frontier`) was confirmed on real hardware by a supervised run — operator present, after a clean
  confirming dry-run with no plan drift (HEAD/origin/master `5248758`; `47f39be`/`f90981d`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 14 --max-probes-per-target 2 --safe-start-cap 1075`
  — **warm-start OFF**.
- **Safety: PASS.** Exit 0; ~4 min; no TDR / driver reset / black-screen / reboot / crash. Startup recovery
  clean; Safe Loop armed→cleared **per probe** (idle); `reset_to_stock` ran. No persistence:
  `boot_flag.json`/`gpu_applied.json` absent before AND after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` content/size unchanged (idle, `safe_mode` false), mtime touched
  only, **no new blacklist/crash entry**. GPU back at stock idle.
- **Coverage: PASS (the fix works).** 13 dwells across **all 7 targets** (vs the prior 34-on-1935 depth-first
  run). 6 targets stopped via **`PerTargetCap`** (`probes_used=2`, bins 1075 + 1068 mV); global
  `--max-probes 14` **not exhausted** (13 used). The cap prevented one target from draining the budget — the
  precise goal of Option B.
- **1905 dropped** at probe 1: `LiveMismatch`, `overshoot_veto=true`, `eff_cov=0.963` — conservative verifier
  rejection (neighbors 1935 `NoDownCapNeeded` and 1875 `VerifiedCurve` passed), not a fault. Every probe
  `write_mode=monotone_static`, `positive_offsets=0`; verdicts `NoDownCapNeededCeiling` (1935) +
  `VerifiedCurve` (1875–1755). No writer/verifier/Safe-Loop/reset/persistence regression. Shallow band only
  (1075/1068 mV); did not touch 875/868/862/856/850.
- **Profile goal: PARTIAL.** Achieved clocks clustered **1832–1867 MHz**, power **194–199 W**; lower targets
  did not yield distinct clock/power. Live plateau ~1890 MHz, `overshoot` grew 1875:+30 → 1755:+135 → the
  near-stock flatten does not govern the achieved clock. Godforge/Brokkr's/Deep Calm collapsed to
  **1860 MHz / 194 W** (target 1755); FORGE confidence 0.21 (single-trial → best-effort).
- **Key conclusion**: **shallow near-stock coverage at 1075/1068 mV is non-binding on this hard power-capped
  RTX 3060 Ti.** `--max-probes-per-target` solved budget *distribution*, not *binding/differentiation*. The
  collapse cause is the power-limited / non-binding-ceiling regime (consistent with the prior bin-based-floor
  run), not the scheduler.
- **Direction — bind-seeking F1b (next design):** do NOT repeat these flags, do NOT bump the per-target cap to
  3, do NOT enable warm-start, do NOT jump to power-limit/clock-lock changes as the immediate next step.
  Instead, per target: **continue descending while the point is stable but non-binding; stop when it actually
  BINDS, fails the verifier/dwell, or hits the global/per-target cap.** Goal = first useful (binding) point
  per target, not deepest voltage. **No further hardware commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware change in
  this pass.

## Build-frontier bin-based floor — FIRST confirmed hardware validation (commit f90981d) — PASS (safe), partial characterization
- **Validation** (2026-06-13): the hardware-derived / bin-based descent floor (`f90981d`) was confirmed on
  real hardware by a supervised, bounded run — operator present, after a clean dry-run on a fresh debug
  build (HEAD/origin/master include `c99dbf1`+`f90981d`; `23b70c4`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 34 --safe-start-cap 1075 --warm-start-brackets`.
  `--max-probes 34` was chosen so the descent reaches **868 mV** (one real bin below the old 875 mV floor)
  but stops BEFORE **862 mV** — a historical reboot-zone / blacklisted bin.
- **Safety: PASS.** Exit 0; no TDR / driver reset / black-screen / reboot / crash. Startup recovery clean;
  Safe Loop armed → cleared (idle); `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). No persistence: `boot_flag.json`/`gpu_applied.json` absent before AND after;
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; `safe_loop.json` byte-identical
  (idle, `safe_mode` false, size unchanged) — mtime touched at run start only, **no new blacklist/crash
  entry**. GPU back at stock idle.
- **Coverage**: 34 hardware dwells, **all spent on target 1935** (ceilings 1075→868 mV). Reached 875 and
  868 mV; **did not reach 862 mV** (no `ceiling_mv=862` line — the 35th scheduler step hit
  `BudgetExhausted` before any write/dwell). Targets 1905/1875/1845/1815/1785/1755 were NOT physically
  characterized (budget exhausted on the hardest target). Warm-start carry-forward observed (B1: 1935 from
  cap, `warm_started=false`; B2: 1905 inherited `start_mv=893` = 868 + 25 mV). Every probe
  `write_mode=monotone_static`, `positive_offsets=0`, `down_caps=0`, no `overshoot_veto`, all
  `NoDownCapNeededCeiling`, `eff_cov=1.000`.
- **Key interpretation / limit of the result**: this proves it is **safe to WRITE the static VF ceiling
  down to 868 mV and descend the bin sequence** — it does **NOT** prove core stability when the core is
  forced to RUN at 868 mV. The card stayed **power-limited (~198 W)** the whole descent, so the ceiling
  was **non-binding** (NoDownCapNeededCeiling everywhere; the power-governed operating point sat at/below
  each ceiling). Frontier point 1935 → **1839 MHz @ 868 mV vf_bin / 198 W**. **PASS for the first
  bin-based floor validation; partial/insufficient for profile synthesis** (single sustainable clock
  1800 MHz → FORGE confidence 0.21, profiles collapse identical). The historical 862/855 mV reboot-zone
  blacklist is offset-keyed (offsets 255/300/330; freq 1755 @ 862) — a DIFFERENT regime from this
  pure-ceiling, zero-offset, power-limited descent.
- **Direction**: do NOT jump to `--max-probes 40`. `--max-probes 35` could deliberately touch 862 mV for
  pure reboot-zone boundary mapping (operator present; the 862 blacklist entry keyed `freq=1755` would not
  match a 1935-target ceiling → Safe Loop is the backstop, not prevention), but it does not produce useful
  profiles. **Primary next step: pivot to F1b / multi-clock characterization and/or make the ceiling
  actually BIND (e.g. raise the power limit) before descending deeper** — deeper ceilings on a
  power-limited card add reboot-zone exposure for ~zero gain. **No further hardware commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware change
  in this pass.

## Build-frontier floor is hardware-derived / bin-based — shipped (commit f90981d)
- **Decision** (2026-06-13): remove the hardcoded active **875 mV** descent floor from `build-frontier`.
  The lower bound is now **discovered from the GPU's real VF / core-cluster voltage bins** — the lowest
  real graphics-core bin (`seed.cluster_v_min_mv`), not a fixed constant. `FRONTIER_LOWEST_SAFE_MV` is
  deleted from active code; no replacement fixed floor (no 825/800).
- **Bin-based descent**: `FrontierDescent` carries `bins_desc` (real descending VF bins);
  `derive_descent` builds it from `CoreSeed.cluster_bins_mv`; `descend_target` walks **real bins only**
  and never invents 25 mV requested voltages outside the curve. Warm-start maps its margin to the
  **conservative real bin at or above** the requested margin target, and **never starts below the
  previous `lowest_verified_mv`** (B1). `--max-probes` remains the global exposure cap;
  `--warm-start-brackets` stays default OFF; no new CLI flag.
- **Fail-closed**: an empty/underivable bin domain aborts **before any hardware write** (no fallback to
  a fixed floor on a confirmed run). Dry-run now prints the hardware-derived floor, the exact descent
  bin sequence, the real bin count, and the worst-case dwell count.
- **Scope**: only `crates/service/src/gpu_power_sweep.rs`. **Unchanged**: monotone static-base VF
  writer, verifier gates, Safe Loop, `reset_to_stock`, persistence, profile apply. `cargo check` clean;
  `cargo test -p nidavellir-service` 142 passed. Pushed to `origin/master`.
- **Historical validity**: prior `1755 @ 875 mV` validations (NoDownCapNeededCeiling) remain valid for
  that point; they are **not** an active floor. Future runs may descend **below 875** where real bins
  exist and `--max-probes` allows.
- **Operational warning**: **no hardware run of `f90981d` yet.** The descent may now go **below the
  historical ~855 mV reboot zone**. First real runs MUST be bounded (`--safe-start-cap` / `--max-probes`),
  the operator present and able to reboot, and the dry-run hardware floor + bin sequence reviewed before
  any `--confirm`.

## F1b warm-start voltage-bracket carry-forward — shipped + hardware-validated (commits 23b70c4, 6f2f061)
- **Decision** (2026-06-13): ship a **generic** warm-start voltage-bracket carry-forward scheduler
  primitive for ordered hardest→easiest core-clock voltage descents, behind an opt-in CLI flag
  **`--warm-start-brackets` (default OFF)**. An easier target reuses the previous harder target's
  verified + dwell-stable bracket as its descent start (`lowest_verified_mv + 1 step`), skipping
  dominated high-voltage probes. NOT Godforge-specific; first adopter is build-frontier/F1b. Commit
  `23b70c4 feat(service): add warm-start bracket carry-forward` on `origin/master`.
- **Safety constraints (verified by unit tests AND live logs):**
  - **B1** — seed only from a verified (`VerifiedCurve`/`StockEquivalentCeiling`/
    `NoDownCapNeededCeiling`) AND dwell-stable bin; never from verify-only-unstable, unverified,
    crash/abort, or budget drain; never start below the previous `lowest_verified_mv`.
  - **B2** — the verifier/ceiling axis is NOT monotone in clock: a warm-started first probe that
    fails apply/verify falls back ONCE to `safe_start_cap` and descends normally; never on
    drain/crash/abort; never recurses; the target is not dropped.
  - **B3** — every target gets ≥1 verified-or-fallback-exhausted probe; never dropped solely because
    the inherited warm-start probe failed verify/apply.
- **Hardware validation (2026-06-13) — PASS.**
  `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075 --warm-start-brackets`:
  exit 0; no TDR/reboot; Safe Loop armed/cleared; `reset_to_stock` ran; no persistence
  (`boot_flag.json`/`gpu_applied.json` absent after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged); GPU back at stock idle. **33 probes**, all 7 targets produced points.
  **B2 exercised**: 1905 inherited an optimistic 900 mV start from 1935's boost-top NoDownCapNeeded
  bracket, failed verify (`LiveMismatch`, `overshoot_veto=true`), fell back once to cap 1075, target
  preserved. Frontier preserved — `1755 @ 900` (`NoDownCapNeededCeiling`, plateau 1665..1755,
  overshoot 0) and `1755 @ 875` (`NoDownCapNeededCeiling`, plateau 1620..1755, overshoot 0,
  ≈1755 MHz @ 875 mV ≈176 W) re-validated; every probe `write_mode=monotone_static`,
  `positive_offsets=0`. Residual non-1755 single-bin 15 MHz overshoot persists (safe verify-axis
  early stop); FORGE low confidence (0.21) unrelated.
- **Efficiency**: 33 probes vs 32 baseline (≈ flat) but **−5 vs the equivalent from-cap descent (38)**
  for an identical frontier (net of one B2-fallback probe). Modest on this RTX 3060 Ti because mid
  targets stop early on verify-axis residual overshoot regardless of start voltage. **Keep default
  OFF** until more runs justify flipping it.
- **Observability follow-up** (`6f2f061 feat(service): surface build-frontier scheduler logs`):
  log-only — `run_build_frontier` now emits the scheduler `result.log` (bracket carry / warm-start /
  fallback / probes_used) before the synthesis `result.profiles.log`, deduping shared lines (pure
  `ordered_frontier_logs` helper + 2 unit tests). No tuning behavior changed. Closes the validation
  finding that bracket telemetry existed but was invisible in CLI output.
- **Next (later, optional)**: 1–2 more warm-start runs; benign-zero-only (NoDownCapNeeded)
  bracket-seeding refinement (so a boost-top bracket doesn't mis-seed the next sub-boost target — the
  1905 case); broader frontier/profile confidence work. **Do NOT mix with profile persistence yet.**
- **Scope**: docs/continuity in this pass (`handoff.md`, `memory.md`, this file). Code already
  shipped/pushed in `23b70c4` + `6f2f061`.

## F1b Phase 2B.2-c: monotone static-base VF ceiling writer — hardware-validated (commit 8503182)
- **Validation** (2026-06-12): the monotone static-base VF ceiling writer (`8503182 feat(service):
  add monotone static-base VF ceiling writer`, on `origin/master`) was confirmed on real hardware by
  a supervised run `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075`
  (fresh debug build at `8503182`; clean bounded dry-run first; user present). This validates the
  writer behind the b.1 → c.0 → 8b6e105 (legit-zero diagnostics) → 91119e1 (NoDownCapNeeded rescue)
  → 8503182 chain.
- **Result**: exit 0; no TDR/reboot; Safe Loop armed+cleared; `reset_to_stock` ran; no persistence
  (`boot_flag.json`/`gpu_applied.json` absent after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged); GPU back at stock idle. All 32 probes used `write_mode=monotone_static`
  with `positive_offsets=0` (static-base-anchored monotone-down offsets only).
- **The writer fixes the boost-top overshoot.** Primary case `1755 @ 900 mV`: previously plateau
  1755..1845 with `overshoot_veto=true` → `LiveMismatch`; now plateau 1665..1755, overshoot=0, veto
  not triggered → `NoDownCapNeededCeiling` (pass). The run then reached `1755 @ 875 mV` and verified
  (`NoDownCapNeededCeiling`, overshoot=0, plateau 1620..1755, ~19 s dwell, ≈1755 MHz @ 875 mV
  ≈179 W). Matches the monotone-writer safety-audit expectation.
- **Caveats (not blockers)**: a few non-1755 probes at low ceilings still carry a single-bin 15 MHz
  overshoot (`overshoot_veto=true`); FORGE synthesis reported low confidence (best 0.21, single-trial
  Wilson) — unrelated to the writer fix. **Next**: design warm-started voltage-bracket reuse for
  F1b/Godforge; keep build-frontier non-persisting (do not mix with persistence / profile apply yet).
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware
  change in this pass.

## F1b Phase 2B.2-c.1: stock-equivalent ceiling verification for boost-top targets
- **Trigger** (2026-06-11): the FIRST bounded supervised `build-frontier --confirm` run
  (`--max-targets 1 --max-probes 6 --safe-start-cap 1075`, Fable-5-audited, user present) completed
  SAFELY (no TDR/reboot; Safe Loop armed+cleared per probe; reset-to-stock fired; no persistence;
  GPU back at stock) but produced **0 frontier points**: the only probe (target=1935 = the stock
  cluster boost top, ceiling 1075 mV) was rejected `LiveMismatch` at `offsets=20/27,
  plateau=1935..1935, overshoot=0`. Root cause: a flatten whose target equals the stock boost top
  legitimately needs ZERO offset on bins already at target, so the ≥90% offset-presence gate
  under-counts — the ceiling effect was present but unprovable by presence alone.
- **Decision**: add a NARROW stock-equivalent acceptance path, not a weaker global gate. New pure
  `is_stock_equivalent_ceiling` (`gpu_verify.rs`) consulted ONLY when the unchanged
  `classify_curve` gate says `LiveMismatch`. Accept only when ALL hold: (1) caller-supplied stock
  boost top present and target AT or BELOW it by ≤ tol — DIRECTIONAL `target ≤ top && top − target
  ≤ tol_mhz` (15 MHz); a target ABOVE the stock top is an overclock, rejected even within tol
  (replaces the original symmetric `abs_diff`); (2) every expected bin's offset readable;
  (3) NO bin reads above target (overshoot rejected even within tol); (4) every bin within tol
  below target; (5) every ZERO-offset bin reads EXACTLY at target — offset 0 means GetStatus shows
  the unmodified stock base and a correct flatten writes `target − base`, so `base == target` is
  the only valid explanation for a missing offset. Result carried as service-internal
  `LiveCeilingEval.stock_equivalent` (+`stock_equivalent_bins` diagnostic); **`CurveVerification`
  (IPC) is untouched** and `state` still reports the normal verdict.
- **Why it does not weaken safety**: the path demands MORE evidence than the normal gate (per-bin
  frequency agreement + zero-overshoot + full accounting of every bin), not less; a silently-failed
  apply leaves below-top bins at their stock base → violates (4)/(5) → rejected; GetStatus idle
  noise can only cause a false REJECT (fail closed). `VerificationFailed` (unreadable/empty) is
  never rescued. Below-boost targets with weak offsets stay `LiveMismatch`.
- **Data flow**: `eval_ceiling_evidence`/`classify_live_ceiling` gain `stock_top_mhz: Option<u32>`.
  `verify_applied_curve` (VerifyAppliedProfile IPC) passes `None` → classification byte-identical.
  `real_probe_step` passes `Some(seed.stock_boost_max_mhz)` and accepts `VerifiedCurve ||
  stock_equivalent`, logging the accepted branch distinctly as `verify=StockEquivalentCeiling`.
- **Scope**: `gpu_verify.rs` + `gpu_power_sweep.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop/
  reset/abort/`gpu_apply`/planning change, no hardware run in this patch. `cargo check` clean;
  service **109/109** (+11 stock-equivalent tests: the exact first-run reproduction, plateau-miss /
  any-overshoot / below-boost / zero-offset-not-exact / no-stock-top / unreadable-offset rejects,
  the fully-degenerate all-zero-at-target accept, tol-boundary accept/reject on both the bin-freq
  and target-vs-top gates, and the directional above-top reject), core 46/46. **`--confirm` remains
  gated on explicit user approval; re-run the bounded dry-run first.**

## F1b Phase 2B.2-c.0: first-run limiter flags for build-frontier
- **Decision** (2026-06-08): bound the first supervised hardware run so it validates the pipeline
  without the full 84-dwell plan. Added `build-frontier` flags: `--max-targets N` (truncate to the
  top N targets), `--max-probes N` (hard-stop total probe executions), `--safe-start-cap MV` (lower
  the descent start to the cap when below the derived cluster top).
- **Semantics**: dry-run + confirmed both honor the flags; defaults (no flags) preserve the full
  plan. `--safe-start-cap` never raises above the derived top and never goes below the crash floor.
  FAIL CLOSED on absurd values (max-targets/max-probes = 0; cap ≤ crash floor; non-numeric/missing).
  `--max-probes` short-circuits remaining probes (no hardware), then the run resets to stock + clears
  the Safe Loop flag (no auto-apply, no `forge_state`/`gpu_knowledge` writes).
- **Pure helpers**: `FrontierLimits` / `validate_limits` / `apply_frontier_limits` (gpu_power_sweep);
  `parse_frontier_limits` (main.rs). Dry-run prints a `limits` line + the capped dwell budget.
- **Scope**: `gpu_power_sweep.rs` + `main.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop/
  `gpu_apply`/`nvml_gpu`/Phase-3/11D change, no hardware. `cargo check` clean; service 95/95 (+7),
  core 46/46. **Dry-run QA** (`--max-targets 1 --max-probes 6 --safe-start-cap 1075`, stock, no
  --confirm, no state writes): targets=[1935], descent 1075→875 mV (9 bins), 6 dwells (~120 s,
  capped). `--confirm` remains forbidden.

## F1b Phase 2B.2-b.4: derive safe_start from the stock core VF cluster (refines b.3)
- **Decision** (2026-06-07): b.3's generic guard rejected absurd values (7001 MHz, 1237 mV) but
  still let `safe_start` come from the global max of *all* sane points — which on the 3060 Ti was
  1150 mV (the hard-cap boundary, likely a non-core point inside the generous range). b.4 adds a
  stage-2 derivation: select the actual contiguous core VF cluster and derive boost/sustained/
  safe_start from the CLUSTER TOP, never the global sane max.
- **`select_core_cluster`** (pure): sort sane points by voltage; split into contiguous runs where
  the voltage gap ≤ `CORE_CLUSTER_GAP_MV` (60 mV); pick the LARGEST run (ties → lowest voltage = the
  dense core); FAIL CLOSED if it has < `MIN_CORE_CLUSTER_POINTS` (8). Isolated high-voltage points
  above the cluster top are reported as rejected outliers.
- **Diagnostics**: the dry-run now prints raw/retained/rejected counts, rejected extremes, the
  selected core-cluster mV+MHz range, outliers-above count, the stock reference (from cluster top),
  the safe_start source, and a WARNING when a profile appears applied (`gpu_apply::load_applied()`).
- **b.3 generic hard guards retained** (freq [500,3500] MHz, voltage [600,1150] mV); b.4 only changes
  WHICH sane point becomes safe_start.
- **Scope**: `gpu_power_sweep.rs` + docs only. No IPC/contract/core/`apps/ui`/Safe-Loop/`gpu_apply`/
  `nvml_gpu`/Phase-3/11D change, no auto-reset, no hardware. `cargo check` clean; service 88/88,
  core 46/46. **Stock dry-run QA pending the user's manual reset; `--confirm` remains forbidden.**
  Future: NVML `max_clock_info(Graphics)` could corroborate boost (frozen `nvml_gpu.rs` this patch).

## F1b Phase 2B.2-b.3: graphics-core sanity-domain seeding guard (safety fix)
- **Decision** (2026-06-07): the first `build-frontier` dry-run revealed the seeding derived
  candidate clocks + safe_start from the UNFILTERED global max over `read_vf_curve_modern()`, which
  includes non-core / memory-domain points → a bogus plan (targets 7001..6311 MHz, safe_start
  1237 mV). The dry-run gate correctly blocked it (zero hardware). Fix: never seed from the global
  max; seed ONLY from sane graphics-core points and FAIL CLOSED otherwise.
- **Guard** (pure, in `gpu_power_sweep.rs`): `sane_core_points` keeps points with freq ∈ [500, 3500]
  MHz and voltage ∈ [600, 1150] mV; `derive_core_seed` derives boost/sustained/safe_start from those
  only, records rejected-point diagnostics (rejected max freq/voltage), emits soft-limit warnings
  (freq > 3200, voltage > 1125), and returns `Err` (fail-closed) when no sane points remain or a
  derived value exceeds a hard guard. `run_build_frontier` aborts (no Safe Loop arm / apply / dwell /
  VF write) on `Err` or if any candidate target > 3500 MHz. Constants are SANITY guards, NOT tuning
  targets; a future GPU outside them fails closed and prompts a code update.
- **Result** (re-run dry-run, same card): 132 raw points → 88 sane-core retained, 44 rejected (incl.
  the 7001 MHz / 1237 mV memory points); boost~1935 MHz; targets 1755..1935; 84 worst-case dwells
  (~1680 s). safe_start landed at 1150 mV (the hard-max boundary) → flagged by the soft-max warning;
  the live curve is currently in an applied state, so a stock read would be cleaner.
- **Scope**: `gpu_power_sweep.rs` + docs only. No IPC/contract/core/`apps/ui`/Safe-Loop-behavior/
  `gpu_apply`/`nvml_gpu`/Phase-3/11D change, no auto-reset, no hardware. `cargo check` clean; service
  86/86 (+5 guard tests), core 46/46. Dry-run QA: sane plan, no state writes. **`--confirm` remains
  forbidden until the fixed dry-run is reviewed.**

## F1b Phase 2B.2-b.2: real probe closure + supervised `build-frontier` (code only, not run)
- **Decision** (2026-06-07): implement the real Windows-only probe + supervised console entry, but
  DO NOT execute the hardware path in this patch (validated by `cargo check`/tests only).
- **`real_probe_step`** (the `build_frontier` seam under `--confirm`): abort/boundary guard → snap
  `vbin` to a real VF bin (`nearest_vf_bin_at_or_above`) → arm Safe Loop → `apply_vf_ceiling` →
  read-only verify via the shared `classify_live_ceiling` (+ 11C diag log) → on not-VerifiedCurve
  reset+clear+return → `load_and_measure` dwell → clear flag → `measured_to_probe` + set `vf_bin_mv`.
  A dwell **Crash** resets to stock and sets an `abort` flag so the remaining probes short-circuit
  (run drains safely); a normal Unstable/unverified only stops THAT clock's descent.
- **`run_build_frontier(store, confirm)`**: always prints the `plan_frontier` plan. Dry-run (no
  `--confirm`) is read-only — no arm/apply/dwell/VF-write, no startup recovery. Confirmed runs
  `build_frontier` with the real probe, then ALWAYS `reset_to_stock` + clears the flag. **No
  auto-apply, no `forge_state` write, no `gpu_knowledge` write.** Console subcommand `build-frontier`
  in `main.rs`; `--confirm` runs startup recovery (parachute) first.
- **First-version conservative seeding** (operator-tunable consts; review the printed plan before a
  run): `lowest_safe_mv=875` (above the ~855 mV known reboot), 25 mV step, 30 MHz clock step, 0.90
  floor; an idle `Unconstrained` regime is clamped to `PowerLimited` (no OC on a first run);
  sustained ≈ curve top freq; per-probe confidence = 0.21 (single-trial Wilson; matures via V3).
- **Scope**: `gpu_power_sweep.rs` + `main.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop-behavior
  /`gpu_apply`/`nvml_gpu`/Phase-3/11D change. `cargo check` clean; service 81/81 (+1 arg-parse test),
  core 46/46. **Hardware path NOT executed** — supervised dry-run + `--confirm` QA is 2B.2-c
  (separately gated).

## F1b Phase 2B.2-b.1: seeding + dry-run plan + vf_bin propagation (pure prep)
- **Decision** (2026-06-07): land the pure half of 2B.2-b. Exposes the verifier's
  `classify_live_ceiling` / `LiveCeilingEval` / `CurveDiag` as `pub(crate)` (intra-crate visibility
  only — NO IPC/contract change) so the future transient-ceiling probe (2B.2-b.2) reuses ONE
  classification path. Adds pure seeding: `derive_descent(curve_bins, lowest_safe, step) ->
  FrontierDescent` (safe_start = top live bin, clamped ≥ the operator crash floor) and a read-only
  dry-run `plan_frontier(targets, &descent, dwell_ms) -> FrontierPlan` (worst-case dwell count +
  wall-time estimate + safety notice). `candidate_clocks` / `classify_regime` (Phase 1) supply targets.
- **Internal `ProbeSample.vf_bin_mv: Option<u32>`** (NOT IPC): the actually-applied snapped bin.
  `probe_to_point` now records `vf_table_voltage_mv = vf_bin_mv.or(descent vbin)`; the pure
  `measured_to_probe` leaves it `None` (the real probe fills it after the apply in 2B.2-b.2).
- **Scope**: pure prep only — NO real probe, NO `apply_vf_ceiling` / `load_and_measure`, NO
  `build-frontier` subcommand / `--confirm`, NO Safe-Loop arm/clear, NO startup-recovery wiring, NO
  forge_state / gpu_knowledge writes, NO Phase-3 / 11D / `apps/ui` / core / contract change, NO
  hardware. `cargo check` clean; service 80/80 (+7), core 46/46 (untouched). 2B.2-b.2 (real probe +
  supervised `--confirm` entry) and the hardware QA run remain separately gated.

## F1b Phase 2B.2-a: shared live-ceiling classification helper (pure refactor)
- **Decision** (2026-06-07): factor live-curve classification out of `verify_applied_curve`
  into a reusable path so the persisted-profile verifier (today) and the future transient-ceiling
  probe (2B.2-b) share ONE classifier. `classify_live_ceiling(live, ceiling_idx, ceiling_mv,
  target, tol)` (read-only; builds the offset-readback evidence at/above the bin) →
  `eval_ceiling_evidence(target, anchor_idx, &expected, tol)` (pure; runs the UNCHANGED
  offset-presence `classify_curve` gate + the 11C `compute_curve_diag`) → `LiveCeilingEval`
  bundle.
- **Behavior unchanged**: `VerifyAppliedProfile` output is byte-identical (same classifier, same
  diagnostic, same inputs) — the refactor only removes inline duplication. Offset-presence remains
  the gate; GetStatus plateau spread stays diagnostic-only; voltage never affects classification.
- **Scope**: service-internal refactor + 5 pure tests in `gpu_verify.rs`. NO real probe, NO
  `build-frontier` subcommand, NO `apply_vf_ceiling` / `load_and_measure`, NO Safe-Loop / synthesis /
  `apps/ui` / core / contract / Phase-3 / 11D change, NO hardware. `cargo check` clean; service
  73/73 (+5), core 46/46. Pure seeding helpers were NOT added (dead code until 2B.2-b). 2B.2-b
  (real probe + supervised `--confirm` entry) and the hardware QA run remain separately gated.

## F1b Phase 2B.1: pure probe-mapping prep + target_clock_mhz (no hardware)
- **Decision** (2026-06-07): land the pure, hardware-free half of Phase 2B first. Adds
  `measured_to_probe(&Measured, curve_verified, confidence) -> ProbeSample` (in
  `gpu_power_sweep.rs`) — the seam the real probe closure (2B.2) will use to feed `build_frontier`
  — performing NO hardware I/O, only a conservative interpretation of already-collected dwell data.
- **Conservative mapping rules**: a `Stable` verdict becomes `ProbeOutcome::Stable` ONLY when
  clock/power telemetry quality ≥ Medium AND a sustained-clock `p5` is present; `SilentError` /
  `Crash` (incl. a TDR / device-lost dwell → `Measured::degenerate(Crash, …)`) or weak telemetry →
  `Unstable`. `p5_clock` is preserved as the sustained-clock signal (0 / no samples → `None`);
  measured voltage uses the ramp-filtered avg and stays `None` when missing — never a fake 0.
- **Additive schema**: `PowerSweepPoint.target_clock_mhz: Option<u32>` (`#[serde(default)]`,
  backward-compatible, no schema bump) records the asked-for clock vs `clock_mhz` (measured
  achieved). Phase 2A `probe_to_point` now stamps it; the single-clock live sweep sets `None`.
- **Scope**: pure / backend-safe only. NO real probe closure, NO `apply_vf_ceiling`, NO
  `load_and_measure` loop, NO supervised console command, NO Safe-Loop / synthesis / `apps/ui` /
  Phase-3 / 11D change, NO hardware. `cargo check` clean; service 68/68 (+7), core 46/46 (+2).
  Phase 2B.2 (real probe + supervised entry point) and the hardware run remain separately gated.

## Read-only live diagnostic for the elastic VF ceiling (Patch 11C)
- **Decision** (2026-06-06): extend the existing read-only verifier (`gpu_verify::verify_applied_curve`
  / `verify-applied`) with structured diagnostic evidence + a single live telemetry snapshot, so the
  applied-curve↔telemetry relationship is provable **without mutating GPU state**. Classifier
  semantics are **unchanged**: `VerifiedCurve` still gates on flatten-offset *presence*; measured/live
  voltage above the VF anchor never downgrades; GetStatus freq stays diagnostic only.
- **Added (pure, testable)**: `compute_curve_diag` over the same per-point evidence → first modified
  bin index/voltage, modified vs expected bin count, GetStatus freq-match count, GetStatus plateau
  min/max MHz, max target overshoot/undershoot, and 3 representative offset samples (first-modified,
  anchor, highest-voltage). Plus one read-only `LiveSnapshot` (NVAPI measured voltage + first NVML
  reading: clock/power/util/temp/limit/cap). Surfaced via additive `Option`/`serde(default)` fields on
  `ApplyVerificationStatus` and one compact `apply_verify_diag:` log line.
- **What it proves**: the flatten offsets are resident and *curve-flatten-shaped* (big `+` at the
  low-voltage anchor, `−` at the top), and how far the GetStatus plateau spreads vs target. **What it
  does NOT prove**: effective/measured voltage behavior, exact per-point offset correctness, or live
  in-game stability. The live snapshot is telemetry, not load verification.
- **Exact-offset verification deferred**: expected offset is `target − stock_base_mhz`, but per-point
  stock base is not persisted and GetStatus freq is idle-unreliable, so exact-offset classification is
  NOT implemented (would need a persisted pre-apply stock curve, or validating the GetStatus `base`
  tuple — a future patch). 11C reports the *symptom* (plateau spread / offset distribution) instead.
- **Runtime QA finding (read-only, 3060 Ti)**: `VerifiedCurve` (62/64 offsets present), but the
  diagnostic revealed `anchor_offset_khz=+255000`, `highest_bin_offset_khz=−120000`, GetStatus plateau
  **1770–1830 MHz** (overshoot 45, undershoot 15) and live snapshot `voltage=1068 mV, clock=1815 MHz,
  util=6%`. This is consistent with both (a) a genuinely curve-flatten-shaped offset set and (b) the
  open **overshoot suspect** (plateau not landing exactly on target) — but GetStatus idle noise
  (freq_match 18/64) means it is **not yet conclusive**. Confirms the diagnostic does its job: surface
  the evidence, defer the verdict to exact-offset work. No state mutated (`gpu_applied.json` mtime
  unchanged).
- **Scope**: additive IPC + log only. Files: `crates/service/src/gpu_verify.rs`,
  `crates/core/src/ipc.rs`, `docs/contracts/ui-backend.md`, docs. **No apply/classifier/Safe-Loop/
  synthesis/`apps/ui`/`nvml_gpu.rs` change; P-state + full ThrottleReasons deferred; no hardware
  writes.** `cargo check` clean; service 61/61 (+9 diag tests), core 44/44.

## Elastic VF ceiling caps frequency, not effective voltage (no hard voltage cap)
- **Decision** (Applied Voltage Behavior investigation, 2026-06-06): the canonical apply path
  (`apply_vf_ceiling`, `crates/gpu-nvapi/src/lib.rs`) writes **per-point frequency offsets** to
  every modern ClkVfPoints curve point whose VF-table voltage is **≥ the selected ceiling bin**,
  flattening them to `target_mhz`; points below the bin are left untouched (elastic). It writes
  **no voltage**, holds no rail lock, pins no clock. It therefore **caps frequency, not
  effective/rail voltage**, and **does not hard-cap measured voltage in any P-state**.
- **`vf_table_voltage_mv` (the VF/curve bin) is the deterministic apply/verify/frontier key** —
  re-derived by snapping the measured dwell voltage UP to the lowest table bin ≥ it
  (`nearest_vf_bin_at_or_above`). `measured_voltage_mv` (NVAPI `core_voltage`) and HWiNFO's
  "GPU Core Voltage" are a DIFFERENT domain (measured rail incl. load-line/droop) — telemetry +
  cross-check only. They may legitimately read **above** the VF bin (idle/2D especially, and under
  load by the VID→rail offset). **Measured ≠ the bin is EXPECTED, not a mismatch** (idle ~1.075 V
  and in-game ~0.887–0.956 V for an ~850 mV bin are normal).
- **Nidavellir must NOT imply a hard voltage cap.** "X MHz @ Y mV" reads as a rail-voltage ceiling
  the engine does not provide; prefer "1785 MHz target · 843 mV VF bin". Profile cards should
  eventually show the VF bin AND the measured-under-load voltage (avg/min/max — fields already on
  `PowerSweepPoint`) as SEPARATE values.
- **A true hard voltage cap would require the legacy voltage-lock path** (`lock_core_voltage_mv` /
  `set_vfp_locks`) — the documented TDR cause under game load. There is no "soft voltage ceiling"
  NvAPI mechanism. A hard cap is therefore **not aligned with safety-first**; Nidavellir stays on
  the elastic VF ceiling only.
- **What verification proves**: `VerifyAppliedProfile` confirms the frequency-flatten OFFSETS are
  resident (≥90% of plateau points carry a non-zero offset) plus a load axis from stored dwell
  stats. It proves **nothing about effective/measured voltage** and cannot (yet) detect a
  present-but-wrong-valued offset or a live-load plateau.
- **Open suspect (not confirmed; read-only-testable, deferred)**: offsets are computed as
  `target − base_mhz` with `base_mhz` from GetStatus at apply time, and GetStatus under-reports
  freq at idle — so a plateau applied at idle could land above `target` (consistent with observed
  ~1815–1830 MHz vs ~1785, alongside normal 15 MHz boost-bin quantization). To be confirmed by a
  future read-only live diagnostic, NOT changed here.
- **Scope**: documentation/contract only (Patch 11A) — `decisions.md`, `docs/contracts/ui-backend.md`
  (incl. a Codex wording request), `memory.md`, `handoff.md`. **No backend code, no `apps/ui`, no
  apply/verify change, no F1b Phase 2B, no hardware.** Live diagnostic (11C) and the UI copy
  implementation (Codex) are deferred.

## F1b Phase 2A: simulated multi-clock outer-loop scaffolding (no hardware)
- **Decision** (2026-06-06): prove the multi-clock loop in isolation BEFORE touching hardware.
  `build_frontier(candidate_clocks, &FrontierDescent, &ForgePolicy, probe: impl Fn(u32,u32) ->
  ProbeSample)` (in `gpu_power_sweep.rs`) drives the outer loop over candidate clocks and a
  per-target voltage-bin **descent** through an **injected probe closure** — the closure is the
  only seam to (future) hardware.
- **Loop rules**: inner descent starts at `safe_start_mv`, steps down by `voltage_step_mv`,
  **never below `lowest_safe_mv`** (the known-crash floor as a config input); keeps the deepest
  stable point; stops on first `Unstable`; stops/drops on simulated `curve_verified == false`
  (the Phase-2B Patch-A gate); drops a clock with no stable point. Outer loop allows a partial
  frontier; empty frontier → synthesis returns all-`None` (safe failure, no panic).
- **Frontier points** record `vf_table_voltage_mv` as the deterministic bin; measured voltage
  stays telemetry only. Synthesis = `synthesize_forge_profiles(&frontier, policy)`.
- **No hardware path wired**: no `load_and_measure`, no `apply_vf_ceiling`, no VF write, no GPU
  stress, no Safe Loop interaction, no real power sweep. All types `#[cfg(windows)]
  #[allow(dead_code)]` (wired in Phase 2B).
- **Validation**: `cargo check` clean; service **52/52** (8 sim tests; 3060 Ti → 1830/1815/1740
  and 4090 → 2880/2860/2700 proven *through the loop*; inner-stop, boundary, verify-fail,
  partial, collapse, no-valid all covered).
- **Phase 2B (future)**: real probe closure (apply ceiling at bin → Safe-Loop-armed dwell →
  offset-readback `VerifiedCurve` gate) behind a supervised/approval-gated entry point.
  **Phase 3** (knowledge re-keying by `(target_clock, vf_table_voltage_bin)` + global voltage-floor
  boundary) remains future work.

## F1b Phase 1: policy-driven multi-clock synthesis (pure, service-internal)
- **Decision** (2026-06-06): the three profiles are synthesized from ONE multi-clock frontier
  via centralized policy, not three independent sweeps. `ForgePolicy` (in `gpu_power_sweep.rs`)
  holds the thresholds — **Balanced default: Brokkr's ≥ 0.98 × Godforge clock, Deep Calm ≥ 0.90,
  confidence ≥ 0.85**; Conservative (0.99/0.92/0.95) and Aggressive (0.97/0.85/0.70) presets exist.
- **Selection rules** (`synthesize_forge_profiles(frontier, &ForgePolicy)`):
  - **Godforge** = highest **sustainable** clock — uses `p5_clock_mhz` when present (dip-aware),
    falls back to `clock_mhz`; ties → lowest power.
  - **Brokkr's Best** = **max R = %power_saved ÷ %clock_lost within the Brokkr's clock floor**
    (real trade: clock < Godforge, power < Godforge). Resolves the F1b-doc 4090 ambiguity:
    Brokkr's = **2860** (max R within floor), NOT 2840 — keeps Brokkr's nearest Godforge rather
    than drifting into the eco profile.
  - **Deep Calm** = max MHz/W within the Deep Calm clock floor (stays useful).
- **Measured voltage is NOT a selection axis** — selection uses clock/power/p5/confidence only;
  `vf_table_voltage_mv` stays the deterministic apply axis (per the voltage-split decision).
- **Single-clock collapse** (the old single-clock sweep's failure mode) is detected and logged;
  synthesis still returns all three profiles (no panic/empty).
- **Scope**: pure, service-internal, additive — no IPC change, no `apps/ui`, no Safe Loop, no
  hardware path. `cargo check` clean; service 44 tests (F1a assertions unchanged). **Phase 2 (real
  multi-clock measurement loop) NOT started** — needs simulated outer-loop scaffolding first, then
  a supervised/approval-gated hardware run.

## Forge action consolidation: Power Sweep is canonical; Real Sweep / Forge Everything are legacy
- **Decision** (Forge Action Consolidation audit, 2026-06-06): **`gpu_power_sweep.rs` (Power
  Sweep) is the canonical Forge GPU core path** — offset + elastic VF ceiling
  (`apply_vf_ceiling`), game-power render dwell, Safe-Loop-guarded, knowledge-bounded, **no
  voltage lock**. It is the only current safe core-optimization engine and the one F1a/F1b
  build on. Canonical apply path = `ApplyPowerGodforge/ApplyPowerBrokkrs/ApplyPowerDeepCalm`.
- **`gpu_sweep_real.rs` (Real Sweep) and `gpu_forge_all.rs` (Forge Everything) are LEGACY
  voltage-lock paths**: both call `lock_core_voltage_mv` (Real Sweep L239/L370; Forge Everything
  fixed `CORE_VOLTAGE_MV=900` L193) — the documented TDR cause under load. Real Sweep also uses
  compute (ALU) load, not game power. Forge Everything tunes VRAM around a *fixed-voltage* core,
  not a forged curve. The legacy `ApplyGodforge/ApplyBrokkrs/ApplyDeepCalm` trio (from
  `real_sweep.profiles`) belongs to this generation.
- **Legacy core paths should be hidden from normal UI** (developer/diagnostic only) and
  **scheduled for removal in a later patch** (after F1b makes the Power Sweep pipeline the whole
  story). Do NOT remove the IPC methods yet — keep them wired to avoid mid-stream build/IPC breaks.
- **VRAM tuning is a FUTURE Forge GPU pipeline step**, not an independent primary action. It MUST
  run **after** the core VF curve is forged + validated and **adapt to** that curve — it must
  never define or destabilize the core. `gpu_mem_sweep.rs` is safer than the legacy core paths
  (no core voltage lock) but today runs independently of the forged core, so it stays an Advanced
  Diagnostic until redesigned. The Gen-1 `Forge Everything` ordering (VRAM around fixed-voltage
  core) is exactly what this rule forbids.
- **Status**: audit only — no code removed, no `apps/ui` change. Frontend request recorded in
  `docs/contracts/ui-backend.md`.

## Voltage is three concepts, not one number; F1b keys on VF-table, not measured dwell voltage
- **Decision** (Sensor Quality Audit, 2026-06-05): GPU voltage must be split into
  explicitly-named, never-conflated fields:
  - `vf_table_voltage_mv` — deterministic VF-curve point voltage (NVAPI GetStatus).
    **This is the apply/ceiling key and the F1b frontier axis.**
  - `measured_voltage_mv` (avg/min/max) — what the rail actually did under dwell
    (NVAPI `core_voltage`, to be made dense + validated). **Descriptive telemetry and
    cross-check only — never an apply key.**
  - `effective_rail_voltage_mv` (future) — physical rail incl. droop, if separable;
    the only value meaningfully comparable to HWiNFO's "GPU Core Voltage".
- **F1b must NOT use measured dwell voltage as the deterministic apply/frontier key.**
  Key the frontier by **(target clock + VF-table point/index)**; attach measured dwell
  telemetry as a separate descriptive field.
- **Why**: the current sweep stores voltage as a sparsely-sampled (~480 ms) NVAPI
  string-parsed **max**, then reuses that exact number as the `apply_vf_ceiling`
  threshold (`PowerSweepPoint.voltage_mv` → `AppliedProfile.core.voltage_mv` →
  `ceiling_mv`). A noisy measured max steering a deterministic curve op is the root of
  the 837-vs-869 confusion and makes apply fidelity unprovable. Three structs already
  carry a field literally named `voltage_mv` with three incompatible meanings.
- **Corollary**: 837-vs-869 mV is consistent with undersampling + bin quantization
  (+ possible sensor-source difference) — expected semantics, NOT proof of apply
  failure. The constant ~1062 mV unfocused/desktop state is most likely a
  workload-scoped (P0/3D) ceiling leaving other states on the stock curve, compounded
  by the frequency-only flatten leaving voltage uncapped. Neither verdict is finalized.
- **Sequencing**: (1) split voltage fields + stop keying apply on measured voltage
  (must-fix); (2) richer dwell stats (min/p5 clock, voltage avg/min/max, full
  `ThrottleReasons` limiter, sample_count, timestamps, workload-context tag);
  (3) finalize Applied Curve Verification (compare VF-table plateau via GetStatus,
  table-to-table — not against measured voltage); (4) resume F1b on the cleaned axis.
- **Status**: audit recorded; no code/IPC/UI change yet. Contract additions
  (populate `GpuSensors.voltage_mv`, `dwell_quality`, `GpuApplyStatus.verification`)
  to be drafted in `docs/contracts/ui-backend.md` on approval.

## Product model: 3 profiles over a clock×power frontier (reverses "two profiles")
- **Decision**: ship **Godforge / Brokkr's Best / Deep Calm**, synthesized from a
  multi-clock power frontier:
  - Godforge = highest sustainable clock under the cap (not the advertised boost).
  - Brokkr's = max `R = %power_saved ÷ %clock_lost` vs Godforge (**NOT** max MHz/W).
  - Deep Calm = max MHz/W.
  Two orthogonal axes: the **product profile** (objective) and the **confidence
  profile** (Conservative/Balanced/Aggressive — the V2 Wilson gate, applied to all 3).
- **Reverses**: the earlier "Two profiles, not three / removed Deep Calm" and
  "Brokkr's = max MHz/W". Under the product vision (`product.md`), Brokkr's is a
  benefit/cost trade and the old Brokkr's metric (MHz/W) becomes Deep Calm.
- **Requires** a multi-clock frontier measured in real power. The validated *flatten*
  sweep is single-clock; the *lock-voltage* frontier sweep is unsafe under game load
  and uses a voltage proxy. Chosen path (A): **extend the safe flatten sweep to
  multiple target clocks** (F1b) — keeps safety + game-power + continuous knowledge.
- **Status**: F1a (pure `synthesize_forge_profiles`) implemented + unit-tested
  (reproduces Godforge 1830 / Brokkr's 1815 / Deep Calm 1740); not yet wired (F1b).

## V2 selection = Wilson-confidence gate (not score×confidence), with V1 fallback
- **Decision**: among off-cap points, pick the highest accumulated `score()` (MHz/W)
  whose stability confidence — Wilson lower bound (z=1.96) over accumulated
  trials/stable_trials — clears the active profile threshold (**Conservative .95 /
  Balanced .85 / Aggressive .70**; Balanced active, a const). If none clears it, fall
  back to the V1 strategy (best off-cap perf/watt) and log it. Never returns "no
  solution".
- **Why a gate, not score×confidence**: safety-first. "Trust ≥ X, then best
  efficiency" is predictable; a product silently trades confidence for efficiency and
  could ride a barely-tested point. Ranking among the trusted still uses `score()`.
- **Why a join, not selecting from knowledge directly**: the off-cap invariant lives
  on the per-run `PowerSweepPoint` (`power_capped_frac`); `PointStat` has no cap
  field. V2 gates the off-cap subset and joins to `know.points` by offset for
  confidence — so data collection and the `gpu_knowledge.json` schema stay untouched.
- **Reality today**: 1 trial/point → Wilson-LB ≈ 0.21 everywhere → V2 always falls
  back to V1 (the chosen point is unchanged, but the decision is now logged). The gate
  "wakes up" only as trials accumulate across runs → motivates V3.
- **Scope**: code-only inside `gpu_power_sweep.rs`; unit-tested (Wilson values +
  gate-accept + gate-fallback). Supersedes the earlier "score×confidence" phrasing.

## Brokkr's objective = max efficiency (MHz/W), not min voltage
- **Decision**: the undervolt profile maximizes performance-per-watt (the efficiency
  knee), using the stability frontier only to bound the search.
- **Why**: chasing the lowest stable mV walks toward the crash cliff; the best
  perf/watt sits before it. Consistency (off-cap) > squeezing the last millivolt.
- **Alternatives**: "deepest stable undervolt" (rejected — courts the cliff).

## No fixed safety margin — data-driven, per-GPU
- **Decision**: drop fixed MHz margins (the old `CRASH_BUFFER=60` was arbitrary).
  V1 = margin **relative to the discovered zone width** (Conservative 30%). V2 =
  **Wilson lower-bound confidence** (e.g. 0 fails/50 trials ≫ 0/1) as the gate.
- **Why**: GPU curves differ; a fixed MHz is blind to the curve. Confidence/zone-
  relative margins adapt automatically and emerge from observed data.

## Stability frontier stored by severity, accumulated per GPU
- **Decision**: `BoundaryKnowledge { highest_clean, lowest_silent_error, lowest_tdr,
  lowest_reboot }` + per-offset `PointStat { trials, failures, worst_severity, ... }`
  persisted per GPU and summed across runs.
- **Why**: a cheap SilentError ≠ an expensive HardReboot; a HardReboot is valuable
  experimental data to be remembered permanently and **never re-probed**. Brokkr's
  becomes a continuous learning system of each GPU's curve, not a one-shot tuner.

## Apply undervolt via the elastic VF ceiling (ClkVfPoints), not a rigid lock
- **Decision**: flatten the V/F curve above the chosen voltage to the target clock
  via the modern NvAPI **ClkVfPoints** family (per-point freq offsets). No hard
  voltage lock, no NVML clock pin (those are the fallback for unsupported GPUs).
- **Why**: under a ~power-cap game load, a rigid voltage lock / clock pin removes the
  card's power management and **TDRs**. The Afterburner-style flatten keeps
  elasticity. Verified: ceiling controls the live clock under load with no TDR.
- **Key facts**: the old `nvapi` crate (`SetClockBoostTable`) is rejected on 595.97;
  exact struct IDs/layouts + the read-modify-write + one-bit-mask gotchas are in
  `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Dwell load must be game-representative (and repeatable)
- **Decision**: stress with the FurMark-class textured render (`run_render_stress`),
  bounded to short frames (8 overdraw instances × 96 frag-iters), polled every 3
  frames, on a fresh GpuCtx per measurement.
- **Why**: a pure-ALU compute kernel drew ~159 W and never hit the cap → wrong
  regime. The render draws ~199 W and saturates the cap like real games. A single
  oversized frame (~2 s) tripped the TDR watchdog → bound per-frame work; many short
  frames = a real game and stay repeatable across a full sweep.
- **Rejected**: a correction factor on the compute reading (user wants real sensor
  values, not a fudge).

## Validate under game power; drop the rigid clock pin in validation
- **Decision**: the arduous 35 s soak uses the same render and only the offset (no
  NVML clock pin — the cap + curve limit the clock naturally).

## Two profiles, not three
- **Decision**: ship **Godforge** (max performance, ~99% cap, OC-oriented — refined
  later) and **Brokkr's Best** (best perf/watt, off-cap). Removed **Deep Calm**.
- **Why**: Deep Calm converged with Brokkr's once OC+UV were unified.

## Profiles must stay OFF the power cap (except Godforge)
- **Why**: a capped profile dips its clock in-game; off-cap = steady clocks =
  consistency, the project's differentiator.

## Safety: Safe Loop owns crash recovery; risky runs are supervised
- Hardware writes are boot-flag-guarded and reboot-survivable. Deep exploration is
  incremental and supervised (the user must be able to reboot). Verified end-to-end:
  a reboot left no bad profile persisted; the GPU came back at stock.
