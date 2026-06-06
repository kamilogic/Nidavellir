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

