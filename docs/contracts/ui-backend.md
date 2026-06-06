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

