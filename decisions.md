# Nidavellir — Decision Log

Durable technical decisions and their rationale. Newest first.

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
