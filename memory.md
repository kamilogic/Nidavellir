# Nidavellir — Project Memory

Honest, safety-first GPU/CPU/RAM auto-tuner for Windows. Tauri v2 + Svelte 5 UI,
Rust core service, NVIDIA-focused undervolting. ~14 K LOC source across 5 Rust
crates + a Svelte UI. Tested on RTX 3060 Ti (driver 595.97), i7-13700K, DDR4-4000.

This file is the continuity index. See also: `AGENTS.md` (canonical product/agent
governance), `architecture.md`, `decisions.md`, `roadmap.md`, `handoff.md`,
`product.md`, and the methodology doc `docs/gpu-forge.md`.

## Current status (2026-06-05)
- `master`, tag **v0.3.1** (forge-state persistence pushed). Worktree branch
  `claude/vibrant-almeida-dfb6c7`.
- Active work: **foundation reviews before F1b** (F1b on hold, direction not final).
  Review 1 (persistence/startup) **done** → forge-state persistence shipped (below).
  Applied-Curve-Verification review **done** (investigation; see handoff).
  Review 2 (Sensor Quality Audit) **done** (investigation; key decision below).
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
