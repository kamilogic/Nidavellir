\# UI ↔ Backend Contract



\## Purpose



This document defines the contract between the frontend (UI/UX) and backend (GPU tuning and service layer).



The goal is to:



\- document IPC methods

\- document payloads

\- track requested cross-team changes

\- avoid breaking integrations



\---



\# Current IPC Methods



To be documented as the frontend/backend contract stabilizes.



\---



\# Change Requests



\## Frontend request (2026-06-06): Forge action consolidation (backend → Codex)



The UI currently exposes too many tuning/test buttons, several of which are LEGACY

voltage-lock paths (TDR risk) that should not be normal user actions. Backend audit

result (see `decisions.md`):



\- \*\*Primary action\*\*: a single \*\*Forge GPU\*\* (→ \*\*Refine Profiles\*\* once profiles exist).

&#x20; Canonical backend path = `StartPowerSweep` (+ progress `GetPowerSweepProgress`) and apply

&#x20; via \*\*`ApplyPowerGodforge` / `ApplyPowerBrokkrs` / `ApplyPowerDeepCalm`\*\* only.



\- \*\*Move to Advanced Diagnostics\*\* (secondary, collapsed — safe, non-primary):

&#x20; `GetGpuCurve` (Read curve), `StartGpuValidation` (Validate stability),

&#x20; `StartBenchmark` (Benchmark), `VerifyAppliedProfile`, `StartMemSweep`

&#x20; (label it "Memory sweep (experimental)").



\- \*\*Hide as legacy / developer-only\*\* (do NOT surface as normal actions; do NOT call):

&#x20; `StartForgeAll` (Forge Everything), `StartRealSweep` + `StartRealSweepFast` (Real Sweep),

&#x20; and the legacy `ApplyGodforge` / `ApplyBrokkrs` / `ApplyDeepCalm` trio (these read the

&#x20; legacy voltage-lock `real_sweep` profiles). Backend keeps these IPC methods wired for now

&#x20; (removal scheduled after F1b); the UI should simply stop exposing them.



\- \*\*VRAM optimization\*\*: represent as a \*\*future pipeline step INSIDE Forge GPU\*\*, not a

&#x20; separate primary button. VRAM tuning must run AFTER the core VF curve is forged + validated

&#x20; and adapt to it. Until the VRAM redesign, memory sweep stays under Advanced Diagnostics only.



\- \*\*Labels\*\*: `Forge GPU`, `Refine Profiles`, `Advanced Diagnostics`. Avoid exposing

&#x20; "Real sweep" / "Forge everything" as user actions.



\- \*\*Migration / compatibility\*\*: no IPC fields change; this is a visibility/labelling request.

&#x20; All listed methods remain available. Backend does not edit `apps/ui/**`.



\## Frontend request (2026-06-06): Voltage semantics wording — no hard voltage cap (backend → Codex)



The VF ceiling caps FREQUENCY, not voltage (see `decisions.md`: "Elastic VF ceiling caps

frequency, not effective voltage"). The current "MHz @ mV" wording implies a hard voltage cap

that the backend does NOT provide. Backend audit result:



\- \*\*Replace "X MHz @ Y mV"\*\* wherever it implies a voltage cap. The `Y mV` is a VF-table

&#x20; CURVE BIN (the deterministic `vf_table_voltage_mv` apply key), NOT a guaranteed rail-voltage

&#x20; ceiling. Measured / HWiNFO "GPU Core Voltage" is a different domain and may read ABOVE it.



\- \*\*Prefer\*\* wording such as `1785 MHz target · 843 mV VF bin` (or "curve bin"): "target" for

&#x20; the clock, "VF bin" / "curve bin" for the voltage.



\- \*\*Keep measured voltage separate\*\* from the deterministic VF bin. When available, show the

&#x20; measured-under-load voltage (avg/min/max from the applied point's dwell stats) as a SEPARATE

&#x20; value — never merge it into one "@ mV" figure.



\- \*\*Do NOT imply a hard effective-voltage cap\*\* anywhere in copy. Nidavellir guarantees a

&#x20; frequency plateau and preserved power-management elasticity, not a voltage ceiling.



\- \*\*Migration / compatibility\*\*: wording/labelling only. No backend methods, IPC names, or

&#x20; payload fields change. Backend does not edit `apps/ui/**`.



\## Additive (2026-06-05): PowerSweepPoint voltage fields



`PowerSweepPoint` (in `GetPowerSweepProgress` / `ApplyPower*` payloads) gains two

OPTIONAL fields (`#[serde(default)]`, backend-only, backward-compatible):



\- `measured_voltage_mv: Option<u32>` — measured effective dwell voltage (telemetry

&#x20; only; descriptive). Same source/value as the legacy `voltage_mv`.

\- `vf_table_voltage_mv: Option<u32>` — deterministic VF-table bin voltage (the apply

&#x20; key). `None` for legacy points produced before the split.



The legacy `voltage_mv` is retained for display/back-compat and still means the

measured max. UI must keep treating voltage as MEASURED telemetry, NOT as a

guaranteed cap; the deterministic key is `vf_table_voltage_mv` when present. No UI

change is required (missing optional fields tolerated). Rationale: `decisions.md`

→ "Voltage is three concepts, not one number".



\## Additive (2026-06-05): PowerSweepPoint richer dwell stats



`PowerSweepPoint` gains further OPTIONAL `#[serde(default)]` fields (backend-only,

backward-compatible; `None` on points measured before this change):



\- Clock sustainability: `min_clock_mhz`, `p5_clock_mhz` (Option<u32>).

\- Measured-voltage distribution (telemetry only, ramp-filtered + sanity-checked):

&#x20; `avg_measured_voltage_mv`, `min_measured_voltage_mv`, `max_measured_voltage_mv`,

&#x20; `voltage_sample_count` (Option<u32>).

\- Dwell meta: `dwell_sample_count` (Option<u32>), `dwell_duration_ms` (Option<u64>).

\- Temperature: `start_temp_c`, `end_temp_c`, `avg_temp_c` (Option<f32>).

\- Confidence: `voltage_quality`, `telemetry_quality` — new enum `DwellQuality`

&#x20; serializing as `"high"`/`"medium"`/`"low"`/`"unavailable"`.



These are descriptive telemetry for UI explanation/confidence. `voltage_quality`

is typically `medium` (voltage is sampled sparsely). The legacy `voltage_mv` /

`measured_voltage_mv` (max) and the deterministic `vf_table_voltage_mv` apply key

are UNCHANGED. No UI change required. Rationale: `decisions.md` Sensor Quality Audit.



\## Additive (2026-06-06): VerifyAppliedProfile (read-only curve verifier)



New OPTIONAL read-only IPC method `VerifyAppliedProfile` (Patch A — curve-only). It

reads the live modern VF curve and classifies it against the applied profile; it

NEVER applies, reapplies, or mutates GPU state. `GetAppliedProfile` stays the cheap

metadata path — verification is explicit/opt-in.



Response: `ResponseData::ApplyVerification(ApplyVerificationStatus)`:



\- `status`: enum `CurveVerification` → `"not_applicable"` / `"metadata_only"` /

&#x20; `"verified_curve"` / `"live_mismatch"` / `"verification_failed"`.

\- `live_curve_match: bool` (structured; UI must not parse `message`).

\- `label`, `target_mhz`, `vf_table_voltage_mv` (deterministic ceiling bin used for

&#x20; comparison), `legacy_voltage_mv` (diagnostic), `matched_points`,

&#x20; `expected_points`, `message`.



Comparison is table-to-table against the deterministic VF-table bin (re-derived like

apply), NOT measured voltage. `stock_detected`/`external_unknown` and live real-game

workload context are NOT included yet (later patches). Rationale: `decisions.md`

Applied Curve Verification.



\### Additive (2026-06-06): load axis (Patch B)



`ApplyVerificationStatus` gains a second, orthogonal LOAD axis derived from the applied

point's EXISTING synthetic-dwell stats (no new stress run). All additive optional fields:



\- `load_state`: enum `LoadVerification` → `"not_evaluated"` / `"verified_under_load"` /

&#x20; `"telemetry_insufficient"` / `"load_mismatch"` / `"workload_state_mismatch"` (reserved,

&#x20; not produced yet) / `"load_verification_failed"`.

\- `load_reason: Option<String>`, `telemetry_match: Option<bool>`.

\- Diagnostic dwell stats of the matched point: `p5_clock_mhz`, `min_clock_mhz`,

&#x20; `avg/min/max_measured_voltage_mv`, `voltage_sample_count`, `voltage_quality`,

&#x20; `telemetry_quality`.



`status` remains the CURVE axis. Effective headline derivation: `verified_under_load`

only when curve is `verified_curve` AND `load_state == verified_under_load`; absent/weak

load data NEVER downgrades a verified curve. `verified_under_load` here means verified

from stored synthetic-dwell stats, NOT live real-game telemetry. UI must use the

structured `status` + `load_state` fields, not parse `message`. No UI change required.



\## Additive (2026-06-06): Voltage semantics clarification (frequency-only VF ceiling)



Documentation-only clarification of fields already in the contract (no schema change):



\- The applied core profile flattens the modern VF curve to a frequency PLATEAU at/above the

&#x20; deterministic `vf_table_voltage_mv` bin via per-point FREQUENCY offsets. It writes no voltage

&#x20; and does not hard-cap measured / rail voltage in any P-state.

\- `vf_table_voltage_mv` (the VF / curve bin) is the deterministic apply / verify / frontier key.

\- `measured_voltage_mv` / `avg|min|max_measured_voltage_mv` and HWiNFO "GPU Core Voltage" are a

&#x20; DIFFERENT domain (measured rail incl. load-line / droop) — telemetry + cross-check only, and

&#x20; may legitimately read ABOVE the VF bin. Measured ≠ the bin is EXPECTED, not a mismatch.

\- `VerifyAppliedProfile` proves the frequency-flatten OFFSETS are resident (plus a load axis from

&#x20; stored dwell stats); it proves nothing about effective / measured voltage. A verified curve is

&#x20; NOT a verified voltage cap.



No UI change is required by this note (it documents existing fields); the wording request above is

the actionable UI item. Rationale: `decisions.md` → "Elastic VF ceiling caps frequency, not

effective voltage".



\## Additive (2026-06-06): VerifyAppliedProfile read-only live diagnostic (Patch 11C)



`ApplyVerificationStatus` gains OPTIONAL `#[serde(default)]` diagnostic fields (backend-only,

backward-compatible; `None` on older payloads). They are populated by the read-only verifier

(`VerifyAppliedProfile` / `verify-applied`). \*\*None of them affect `status` / classification\*\*,

and the `live_*` snapshot is telemetry only — a single read at verification time, NOT load

verification, and it does NOT imply a hard voltage cap.



Curve / offset evidence:



\- `first_modified_bin: Option<u32>`, `first_modified_mv: Option<u32>` — first plateau bin carrying

&#x20; a non-zero flatten offset, and its VF-table voltage.

\- `modified_bin_count: Option<u32>`, `expected_bin_count: Option<u32>` — modified vs expected

&#x20; (points at/above the anchor).

\- `getstatus_freq_match_count: Option<u32>` — GetStatus plateau points within tolerance of target

&#x20; (diagnostic only; GetStatus is unreliable at idle).

\- `getstatus_plateau_min_mhz` / `getstatus_plateau_max_mhz: Option<u32>` — observed plateau spread.

\- `max_target_overshoot_mhz` / `max_target_undershoot_mhz: Option<i32>` — plateau vs target

&#x20; (`Some(0)` when flat; `None` only when no plateau points).

\- `first_modified_offset_khz`, `anchor_offset_khz`, `highest_bin_offset_khz: Option<i32>` —

&#x20; representative offset samples (kHz).



Live telemetry snapshot (telemetry only; unavailable → `None`, never a fake zero):



\- `live_voltage_mv: Option<u32>` (NVAPI measured core voltage), `live_clock_mhz: Option<u32>`,

&#x20; `live_power_w: Option<f32>`, `live_utilization_pct: Option<f32>`, `live_temperature_c:

&#x20; Option<f32>`, `live_power_limit_w: Option<f32>`, `live_power_capped: Option<bool>`.

\- `diagnostic_message: Option<String>` — compact human-readable note (UI must NOT parse it for

&#x20; logic; use the structured fields).



UI is NOT required to use these now. `live_voltage_mv` may legitimately read ABOVE

`vf_table_voltage_mv` — it is measured rail telemetry, not a cap. Rationale: `decisions.md`

→ "Read-only live diagnostic for the elastic VF ceiling (Patch 11C)".



(No other active requests)



\---



\# Rules



Backend may:

\- add new optional fields

\- add new IPC methods



Backend must not:

\- rename payload fields without updating this document

\- remove fields without migration notes



Frontend must:

\- tolerate missing optional fields

\- avoid relying on display strings for logic

\- use structured payload fields whenever possible



Frontend must not:

\- infer safety state from logs

\- infer profile state from text messages

