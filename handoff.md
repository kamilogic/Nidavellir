# Nidavellir — Session Handoff

How to pick this up cold. State as of 2026-06-04, `master` (clean, latest commit
`2f785cb`). Deep NvAPI struct details live in `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Latest backend checkpoint (2026-06-06) — Applied curve verifier, Patch A (IMPLEMENTED, not pushed)
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
