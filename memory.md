# Nidavellir — Project Memory

Honest, safety-first GPU/CPU/RAM auto-tuner for Windows. Tauri v2 + Svelte 5 UI,
Rust core service, NVIDIA-focused undervolting. ~14 K LOC source across 5 Rust
crates + a Svelte UI. Tested on RTX 3060 Ti (driver 595.97), i7-13700K, DDR4-4000.

This file is the continuity index. See also: `AGENTS.md` (canonical product/agent
governance), `architecture.md`, `decisions.md`, `roadmap.md`, `handoff.md`,
`product.md`, and the methodology doc `docs/gpu-forge.md`.

## Latest (2026-06-13) — build-frontier floor is hardware-derived / bin-based (commit f90981d, pushed; NOT hw-validated)
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
  runs may now go **below 875**. **NOT hardware-validated yet** — first runs must be bounded
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
