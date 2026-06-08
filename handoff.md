# Nidavellir — Session Handoff

How to pick this up cold. State as of 2026-06-04, `master` (clean, latest commit
`2f785cb`). Deep NvAPI struct details live in `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Latest backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.4: stock core VF cluster seeding (IMPLEMENTED, not pushed)
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
