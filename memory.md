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
- **Applied curve verifier — Patch A (this session) — IMPLEMENTED, not pushed**: read-only
  `VerifyAppliedProfile` IPC + `crate::gpu_verify`. Classifies the live modern VF curve vs
  the applied profile into `CurveVerification` = NotApplicable / MetadataOnly /
  VerifiedCurve / LiveMismatch / VerificationFailed. **Table-to-table only**: re-derives the
  deterministic ceiling bin via `nearest_vf_bin_at_or_above(core.voltage_mv)` (same as apply),
  reads `read_vf_curve_modern` (GetStatus) + `vf_get_point_khz` (offset corroboration, logged);
  expected = points ≥ ceiling read target ±15 MHz, ≥90% match → VerifiedCurve. **Read-only**:
  no apply/reapply/write/stress. No telemetry/load/context/stock-fingerprint yet (Patches B/C).
  Additive IPC (`ApplyVerificationStatus`), contract noted. Tests: check clean · service 26/26
  (+7 verifier). **Runtime QA BLOCKED**: `gpu_applied.json` exists → console startup would
  reapply (VF write, prohibited) → live IPC test deferred. Patch B (load classification from
  existing dwell stats) is now unblocked.
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
