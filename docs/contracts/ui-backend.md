\# UI ↔ Backend Contract

> NOTE (2026-07-08): the Claude/Codex backend-frontend split was retired — Claude now owns the whole
> stack. This file is no longer a cross-agent handoff; it is REFERENCE documentation of the IPC
> surface (methods + payload shapes). Keep it current when the IPC changes.



\## 2026-07-16 (additive): `StartPowerSweepClean` — experimental organic clean run

\- **New IPC method** (unit method, no params, same request/response shape as `StartPowerSweep`):
  `StartPowerSweepClean`. Wire: `{"method":"StartPowerSweepClean"}`. Response, progress
  (`GetPowerSweepProgress`), stop (`StopPowerSweep`) and apply methods are all UNCHANGED.
\- **Semantics**: Standard dwell policy, but a fully ORGANIC search for algorithm evaluation during
  development. At start the backend archives `f2_observations.jsonl` + `forge_state.json` under
  `forge-archive/<run_id>/`, snapshots `safe_loop.json` and strips its GPU V/F blacklist regions,
  and reads the durable condemnation ledger RUN-SCOPED (only this run's failures). Failures during
  the run still block and steer vertical repair; ledger writes keep flowing to the global file.
  Sentinel, startup recovery and Safe Mode are unaffected. At the end the run's observations are
  copied into the same archive folder; the next clean run starts organic again.
\- **UI**: the Forge mode selector gains a fourth option `clean` ("Clean run · Experimental") →
  `StartPowerSweepClean`. Existing modes and mappings unchanged.
\- **New additive field (2026-07-17)**: `PowerSweepProgress.learning: Option<String>` —
  `"clean_run"` or `"persistent"` (`None` on legacy payloads). Printed in the run-log export
  header (`learning :`); the clean-run pre-flight also writes
  `forge-archive/<run_id>/clean-run-manifest.txt` as log-independent proof of the mode. Added
  after the 2026-07-17 run proved the live-log tail cannot evidence which policy executed.

\## Current F2 reference (2026-07-15): contract v17, native DX11 gate and Candidate Transaction

This section is the normative current behavior and supersedes the dated v4/v6/v7 runtime descriptions
below where they conflict. Historical notes remain in place to explain payload evolution. No IPC method
or existing field was removed.

\- **Evidence contract v17.** Every current F2 dwell persists `evidence_provenance` with the service
  build version/revision, semantic workload fingerprint, actual selected render backend, adapter name,
  driver name/details, checksum method and stock-golden configuration/values. Pre-v17 positive evidence
  remains readable but cannot unlock Apply. Positive discovery, frontier qualification and exact-Apply
  qualification additionally require `reset_to_stock_ok == true` and `boot_flag_cleared == true`.

\- **Deterministic preheat and distinct clocks.** Before any candidate write, backend phase
  `"preheat"` runs up to six 10 s stock windows and requires two consecutive usable windows converged
  within 2 °C and 30 MHz p5, with no throttle or telemetry failure. It fails closed before tuning.
  Ctable is the sane physical-table ceiling/count, Cboost is the live maximum observed after preheat,
  and Cmax remains the first reset-clean sustainable clock proved by discovery. They are not aliases.

\- **Candidate Transaction for discovery.** Each candidate attempt arms Safe Loop and applies/verifies
  the curve once, then runs PowerRender plus any active qualification phases under that same curve and
  performs one checked stock reset/boot-flag clear. Same-curve Qualification observations are persisted
  before Discovery, preventing resume from seeing an unpaired positive discovery. A p99 retry closes the
  current transaction cleanly before the next attempt; reset/clear failure can never become positive.

\- **Power-cap hysteresis.** With a valid numeric board limit, p99 ≥99% is `NearCap`, p99 ≤98% is
  `OffCap`, and the interval between them is `Ambiguous`. The sampled cap flag is only a fallback when
  the numeric limit is unavailable. Ambiguous evidence receives bounded retries and remains
  inconclusive if it does not resolve.

\- **Interleaved MixedGame and sparse integrity.** Every MixedGame frame records BoostEdge,
  TextureRop and PowerRender as three passes in one encoder/frame/submit. BoostEdge and MixedGame run
  GPU reduction/compare every 16 frames; mismatch state accumulates across all sampled checks and
  `checksum_count` reports the checks actually executed. The UI must not describe this as 100% frame
  checksum coverage.

\- **Exact Apply adds native DX11.** Every unique selected `(target, Apply VF bin)` requires Texture
  5 min, native offscreen DX11 5 min, TransitionShock 8 min and Endurance 20 min. DX11 captures a stock
  golden on an explicitly selected NVIDIA adapter, records/matches its LUID, performs periodic readback
  integrity checks and fails inconclusive when coverage cannot be proved. No existing dwell was shortened.

\- **Additive/defaulted `PowerSweepProgress` fields:**
  - `observed_boost_clock_mhz: Option<u32>` — Cboost observed after deterministic stock preheat.
  - `clock_table_bin_count: Option<u32>` — number of sane static physical V/F bins (Ctable domain).
  - `clock_table_ceiling_mhz: Option<u32>` — highest sane static physical V/F clock (Ctable ceiling).
  - `preheat_converged: Option<bool>` — `false` while normalization is unresolved, `true` only after
    deterministic convergence.
  - `preheat_temperature_c: Option<f32>` — converged stock temperature.

  Existing `cmax_clock_mhz` retains the proved Cmax meaning. `ForgeProgress.svelte` renders the
  Ctable/Cboost/preheat facts from these structured fields and labels `phase == "preheat"` as stock
  normalization. Legacy/interrupted payloads deserialize the new fields as `None`; frontend fallback
  must remain display-only and must not infer safety or eligibility from logs.



\## Purpose



This document defines the IPC surface between the frontend (UI/UX) and backend (GPU tuning and service layer).



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



\## Additive (2026-06-07): PowerSweepPoint.target_clock_mhz (F1b Phase 2B.1)



`PowerSweepPoint` (in `GetPowerSweepProgress` / `ApplyPower*` payloads) gains one OPTIONAL

`#[serde(default)]` field (backend-only, backward-compatible):



\- `target_clock_mhz: Option<u32>` — the TARGET clock the point was probed at in the F1b

&#x20; multi-clock frontier. Distinct from `clock_mhz`, which is the MEASURED achieved clock (the

&#x20; two may differ by boost-bin behavior). `None` for single-clock / pre-2B.1 points.



No schema bump; old `forge_state.json` / `GetPowerSweepProgress` payloads load with the field as

`None`. No UI change required (UI may later show target vs measured clock). Rationale:

`decisions.md` → "F1b Phase 2B.1".



\## Frontend request (2026-06-23): Multi-clock profile discovery + confidence opt-in (backend → Codex)



Backend is building the v0.5 multi-clock frontier that finally differentiates the three

profiles. Three UI-relevant changes; all backend data is additive/optional.



\### 1. Profiles come from a MEASURED multi-clock frontier (not a single clock)



The official sweep now descends MULTIPLE clock targets (anchored at the max sustained clock,

stepping down toward the Deep Calm clock), producing a frontier of points

`(target_clock, measured_clock, p5_clock, voltage_mv, watts, confidence)`. The three profiles are

SELECTION POLICIES over that frontier (no new scoring):



\- \*\*Godforge\*\* = highest sustained clock (top of the frontier).

\- \*\*Brokkr's Best\*\* = best benefit/cost (`%power_saved ÷ %clock_lost`) keeping \*\*≥ 95%\*\* of

&#x20; Godforge's clock (relaxed from 98% → 95%, so the efficiency knee may sit up to 5% below

&#x20; Godforge for much larger watt savings).

\- \*\*Deep Calm\*\* = best MHz/W keeping \*\*≥ 90%\*\* of Godforge's clock (lowest power, still usable).



\*\*UI:\*\* present all three as distinct points (clock / mV / watts / MHz-per-watt). \*\*Honest

collapse:\*\* on a hard power-limited GPU the knee can coincide with the top — when the backend

flags `power_bound_collapse` (or Godforge and Brokkr's resolve to the same point), the UI should

say so plainly (e.g. "Brokkr's ≡ Godforge on this GPU — power-limited, no headroom above the

efficiency point") rather than imply a fake difference. Do NOT manufacture a distinction.



\### 2. Confidence is a STABILITY GATE, not a voltage margin — surface it



Why an applied point can sit ABOVE the deepest voltage the sweep reached (e.g. applied 906 mV

while the sweep validated down to 868 mV): selection is \*\*voltage-agnostic\*\* and gates each point

on accumulated \*\*Wilson stability confidence\*\* (default ≥ 0.85). A point validated only once has

low confidence (~0.21) and is NOT trusted yet; the deepest point that has earned enough repeat

confirmations wins. It is NOT a fixed safety margin.



\*\*UI:\*\* per profile point, show its \*\*confidence\*\* and \*\*validation count\*\* (e.g. "confidence 0.84

· 12 confirmations"), so the user understands a deeper point is "not yet confirmed enough" rather

than "blocked by a margin".



\### 3. Confidence opt-in: "Build confidence now" (longer run) — DEFAULT OFF



New backend option (`validation_passes`, default 1): an OPT-IN that spends a longer single session

doing extra validation passes on the deepest discovered point so it earns the confidence gate

WITHIN one session, instead of waiting across days/runs. Bounded (max 20 passes). The default

(`1`) is exactly today's behavior and is UNCHANGED.



\- \*\*Mode 1 (default, keep)\*\*: confidence accrues across normal runs over time.

\- \*\*Opt-in\*\*: user chooses a longer "build confidence now" run (more passes) to skip the wait.



\*\*UI:\*\* a clear optional control (toggle + passes/time selector) labelled as a LONGER run, with a

note that it re-validates the deepest point repeatedly (more GPU time/heat) and is optional.

Default OFF. \*\*Future (not in this delivery):\*\* automatic confidence-building while the PC is IDLE —

leave conceptual room for it but do not build it yet.



\*\*Compatibility:\*\* all backend additions are optional/additive (no payload renames/removals). The

`validation_passes` knob will need an IPC parameter when the Forge action is wired; until then it

is a service-level option. Rationale + algorithm details: `decisions.md`, `handoff.md`.



\## Frontend implementation checkpoint (2026-06-26): UI ready, IPC additions requested (Codex → backend)



Codex applied the frontend-only parts of the 2026-06-23 request:



\- Profile cards and Forge Progress use `target_clock_mhz` when available and keep measured

&#x20; `clock_mhz` / `p5_clock_mhz` separate.

\- All three profiles show clock, VF bin, watts and MHz/W.

\- Honest collapse copy uses structured `power_bound_collapse` when available, with equality of the

&#x20; Godforge/Brokkr's points as a backward-compatible fallback.

\- Per-profile stability evidence renders when the optional `confidence` and `validation_count`

&#x20; fields are present. Missing fields remain silent; the UI does not fabricate values.



The profile evidence remains optional and silent on legacy payloads. The additive backend fields
requested here were delivered in the 2026-06-27 Phase 2 contract closeout.

The delivered fields are:



\- `PowerSweepPoint.confidence: Option<f64>` — structured stability confidence (0–1) for

&#x20; the selected point. F1 uses its Wilson model; F2 uses its learned-frontier confidence model.

\- `PowerSweepPoint.validation_count: Option<u32>` — successful confirmations at that exact selected

&#x20; point. This is NOT the total observation count across other voltages or outcomes.

\- `PowerSweepProgress.power_bound_collapse: bool` (`#[serde(default)]`)

&#x20; — structured synthesis result; the UI must not infer it from logs or notes.

The start-control dependency is now delivered by the fixed Fast / Standard / Long modes documented

below. That bounded mode contract supersedes the earlier free-form `validation_passes` UI request;

Codex should wire the three explicit start methods rather than expose a numeric pass selector.



\## Frontend request (2026-06-26): Forge GPU button MODES — Fast / Standard / Long (backend → Codex)



The live multi-clock forge (`StartPowerSweep`) now supports three MODES. Two NEW additive IPC methods

select the non-default modes; the existing `StartPowerSweep` is UNCHANGED and means the proven

\*\*Standard\*\* mode. This realises the `validation_passes` "IPC parameter when the Forge action is

wired" noted in the 2026-06-23 entry — delivered as a bounded MODE, not a free-form integer.



\- \*\*New IPC methods\*\* (unit methods, no params, same shape as `StartPowerSweep`):



&#x20; `StartPowerSweepFast` and `StartPowerSweepLong`. Wire: `{"method":"StartPowerSweepFast"}`.

&#x20; Response + progress (`GetPowerSweepProgress`) + stop (`StopPowerSweep`) + apply

&#x20; (`ApplyPowerGodforge` / `ApplyPowerBrokkrs` / `ApplyPowerDeepCalm`) are all UNCHANGED.



\- \*\*Expected UI\*\*: a 3-way mode selector on the Forge GPU / Refine Profiles action. Default =

&#x20; \*\*Standard\*\* → keep sending the plain `StartPowerSweep` (no behavior change). \*\*Fast\*\* →

&#x20; `StartPowerSweepFast`. \*\*Long\*\* → `StartPowerSweepLong`. Only the START method changes per mode;

&#x20; stop / progress / apply are identical across modes.



\- \*\*Mode semantics\*\* (for toggle copy / tooltips):



&#x20; \*\*Fast\*\* — quicker discovery (fewer probes, shallower per-clock depth); ONE ceiling soak per

&#x20; profile. Cross-run confidence is left to IDLE / later manual runs. Shortest supervised run.



&#x20; \*\*Standard\*\* — today's proven, hardware-validated default. Unchanged.



&#x20; \*\*Long\*\* — broader + deeper discovery AND repeated ceiling soaks per profile, so a deep point

&#x20; earns its confidence in ONE session (no waiting for IDLE). Longest supervised run.



\- \*\*Progress payload UNCHANGED\*\*: `PowerSweepProgress` gains NO fields. The selected mode and the

&#x20; per-profile validation count appear in the `note` / `log` TEXT only (display — do NOT parse for

&#x20; logic). If a structured `mode` field is wanted, request it separately (additive).



\- \*\*Safety (reflect in copy)\*\*: all three modes run the SAME fail-closed supervised motor; every

&#x20; applied profile is validated at its discovered ceiling at least once; NOTHING is auto-applied —

&#x20; apply stays the separate `ApplyPower*` step ("confirme em jogo"). Fast only REDUCES exposure;

&#x20; Long's extra passes can only REJECT a marginal pick, never widen it.



\- \*\*Migration / compatibility\*\*: purely additive. `StartPowerSweep` keeps current behavior; no

&#x20; payload field renames/removals. Backend does not edit `apps/ui/**`. Rationale + knob values:

&#x20; `decisions.md`, `handoff.md`.



\## Backend → Frontend (2026-06-27): forge button is now F2 undervolt; Apply is REFUSED in Phase 1

The forge button's backend method PIVOTED from F1 flatten-down to \*\*F2 anchored undervolt\*\*. Reason:
the RTX 3060 Ti is power-bound (pinned at its 200 W limit), and F1 flatten-down cannot lower power on
a power-bound card. F2 holds the clock at a lower voltage and drops power directly (proven −43 W at the
same clock). F2 produces REAL differentiated Godforge / Brokkr's / Deep Calm profiles.

\- \*\*No IPC method changes.\*\* `StartPowerSweep` / `StartPowerSweepFast` / `StartPowerSweepLong`,
&#x20; `GetPowerSweepProgress`, and `ApplyPowerGodforge` / `ApplyPowerBrokkrs` / `ApplyPowerDeepCalm`
&#x20; are all unchanged in name/shape. The forge button keeps using exactly these.

\- \*\*New additive field\*\*: `PowerSweepProgress.is_undervolt: bool` (`#[serde(default)]`, false on legacy /
&#x20; pre-pivot payloads). `true` means the current forge result is an F2 undervolt profile.

\- \*\*Apply is GATED in Phase 1\*\*: when `is_undervolt == true`, the three `ApplyPower*` requests RETURN A
&#x20; FAILURE — `"F2 undervolt apply not yet wired (Phase 2) — profile discovered but not applicable"`.
&#x20; The profiles are DISCOVERED + persisted and safe to display, but cannot be applied yet (the F2 apply
&#x20; path lands in Phase 2). Until then the UI should, when `is_undervolt` is true:
&#x20; surface the 3 profiles as DISCOVERED, and either hide/disable the Apply action or show a clear
&#x20; "apply coming soon (Phase 2)" state instead of letting Apply fail silently.

\- \*\*Legacy F1 unchanged\*\*: with `is_undervolt == false` (old `real_sweep`/F1 payloads), Apply behaves
&#x20; exactly as before. This is additive + backward-compatible; no migration needed.

\- \*\*Migration / compatibility\*\*: additive field only; no renames/removals. Backend does not edit
&#x20; `apps/ui/**`. Rationale + phased plan: `decisions.md` top entry ("FORGE PIVOTS TO F2 UNDERVOLT").



\## Frontend implementation checkpoint (2026-06-27): F2 discovery state wired (Codex)

\- `PowerSweepProgress.is_undervolt` now drives a structured frontend state; the UI does not infer
&#x20; F2/apply availability from `note`, `log`, or error text.

\- When `is_undervolt == true`, all three profiles remain visible and are labelled \*\*Discovered\*\*.
&#x20; Apply controls are disabled with clear "Apply coming in Phase 2" copy, plus a defensive action
&#x20; guard prevents accidental `ApplyPower*` requests.

\- When `is_undervolt == false` or the field is missing, the existing F1 Apply behavior is unchanged.
&#x20; No Rust, IPC, persistence, profile synthesis, or hardware logic changed.

\- \*\*SUPERSEDED by the Phase 2 backend note below\*\* — Apply is now WIRED; the UI should un-gate.

\## Backend → Frontend (2026-06-27): F2 apply is WIRED (Phase 2) — Apply now applies; un-gate the UI

Phase 2 supersedes the Phase 1 "Apply is REFUSED" note above. The three apply methods now APPLY the F2
anchored undervolt when `is_undervolt == true` — they no longer return the
*"F2 undervolt apply not yet wired (Phase 2)"* failure.

\- \*\*No IPC method changes.\*\* `ApplyPowerGodforge` / `ApplyPowerBrokkrs` / `ApplyPowerDeepCalm`,
&#x20; `GetPowerSweepProgress`, `StopPowerSweep` and the Fast/Standard/Long start methods are all unchanged
&#x20; in name/shape. The response stays `ResponseData::GpuApply(GpuApplyStatus)` as before. The existing
&#x20; `core` status point carries the deterministic F2 target/anchor for UI compatibility.

\- \*\*UI action required\*\*: REMOVE the Phase-1 "apply coming soon / disabled" state for `is_undervolt`
&#x20; results. When `is_undervolt == true`, Apply Godforge/Brokkr's/Deep Calm is a normal, enabled action.
&#x20; On success the status message reads e.g. `Applied Godforge: 1800 MHz @ 875 mV VF bin (undervolt)`;
&#x20; on a fail-closed write it reads `Apply failed: …` (the GPU is reset to stock — nothing left applied).

\- \*\*Behavior\*\*: apply arms the Safe Loop, writes the anchored undervolt, VERIFIES it, persists it
&#x20; (`gpu_applied.json`, re-applied on every boot, fail-closed: a crash leaves it un-re-applied), and is
&#x20; reversible via the existing GPU reset. Still NO auto-apply — apply remains the explicit user step.

\- \*\*Legacy F1 unchanged\*\*: `is_undervolt == false` payloads still apply the F1 flatten ceiling exactly
&#x20; as before. The persisted-profile shape gains an internal `undervolt` descriptor (service-side only;
&#x20; NOT an IPC payload field). Additive + backward-compatible; no migration. Rationale: `decisions.md`
&#x20; top entry + `handoff.md`.

\## Frontend implementation checkpoint (2026-06-27): Phase 2 Apply un-gated (Codex)

\- Removed the Phase-1 disabled/"Apply coming in Phase 2" state and its defensive action guard.
\- F2 profile actions now call the unchanged `ApplyPower*` methods normally.
\- Applied-state matching uses the deterministic F2 target clock and anchor exposed through the existing
&#x20; `GpuApplyStatus.core` point; legacy F1 matching remains measured-clock based.
\- The Discovered badge remains until a profile is applied, then yields to the existing Active state.
\- Structured `confidence`, `validation_count`, and `power_bound_collapse` evidence is now delivered;
&#x20; legacy payloads continue to render without fabricated values.


\## Frontend implementation checkpoint (2026-06-26): Forge modes wired (Codex)



\- Added a compact, product-styled Fast / Standard / Long dropdown inside the Forge GPU /

&#x20; Refine Profiles split action.

\- The main segment starts the selected mode; the compact mode segment opens the selector.

\- Standard is the initial default and continues to call `StartPowerSweep`.

\- Fast calls `StartPowerSweepFast`; Long calls `StartPowerSweepLong`.

\- Stop, progress polling and profile apply paths remain unchanged.

\- Mode copy reflects discovery depth, confidence behavior, relative duration and the shared

&#x20; fail-closed supervised safety model. The UI does not parse `note` or `log` for mode state.

&#x20; \*\*SUPERSEDED by the 2026-06-28 mode-semantics note below\*\*: all modes now traverse the same

&#x20; complete frontier; copy must describe dwell/evidence, not discovery breadth.


\## Backend → Frontend (2026-06-28): corrected F2 frontier + mode semantics

The live Forge algorithm now matches the intended integrated F2 search. This note supersedes the
2026-06-26 descriptions of Fast as “fewer/shallower probes” and Long as “broader/deeper discovery.”

\- \*\*Start methods stay unchanged.\*\* Keep the existing mappings:

&#x20; Fast → `StartPowerSweepFast`; Standard → `StartPowerSweep`; Long → `StartPowerSweepLong`.

\- \*\*Identical discovery frontier in every mode.\*\* All three reset to stock and start at the highest real live-VF
&#x20; clock, discover the first sustainable Cmax through voltage descent, then characterize every real
&#x20; clock bin through 90% of Cmax. No mode tries fewer clocks or a shallower voltage range.

\- \*\*Mode semantics are evidence only:\*\*

&#x20; \*\*Fast\*\* — full-frontier discovery with 10 s dwells and no qualification pass. It produces
&#x20; a provisional preview only; `ApplyPower*` remains locked.

&#x20; \*\*Standard\*\* — 10 s discovery, then two independent 60 s reset/reapply qualification passes
&#x20; at every discovered boundary.

&#x20; \*\*Long\*\* — 10 s discovery, then three independent 120 s reset/reapply qualification passes
&#x20; at every discovered boundary. Longest run and strongest initial confidence.

\- \*\*Qualification and Apply:\*\* `PowerSweepProgress.profiles_qualified` is additive/default-false.
&#x20; The frontend must label unqualified F2 results as provisional and disable Apply. The service also
&#x20; rejects `ApplyPower*` for provisional F2 results, so stale or custom clients fail closed.

\- \*\*Expected fresh-GPU wall time:\*\* Fast ≈20–30 min, Standard ≈55–75 min, Long ≈90–120 min.
&#x20; Learned GPUs normally resume faster. These remain estimates, not deadlines.

\- \*\*Progress/safety:\*\* no definitive profiles are returned from a partial, cancelled, or
&#x20; safety-aborted run. Every mode uses the same arm→write→verify→dwell→checked-reset motor; nothing
&#x20; is auto-applied. A real run may still TDR/reboot and remains a supervised action.

\## Backend → Frontend (2026-06-28): durable F2 progress, ETA and cross-clock reuse

`GetPowerSweepProgress` gains additive, defaulted fields. Legacy payloads remain valid:

\- `mode: Option<String>`
\- `current_clock_mhz: Option<u32>`
\- `current_voltage_mv: Option<u32>`
\- `completed_steps: u32`
\- `total_steps_estimate: u32`
\- `elapsed_ms: u64`
\- `estimated_remaining_ms: Option<u64>`
\- `learned_points: u32`
\- `last_outcome: Option<String>`
\- `learning_saved: bool`
\- `frontier_complete: bool`
\- `profiles_qualified: bool`

The total and ETA are explicitly estimates: Cmax and cross-clock pruning become exact while the run
learns the frontier. The frontend must use these structured fields for the progress bar and current
target; `log` remains display-only.

Every completed candidate is appended to `f2_observations.jsonl` before the progress event says
`learning_saved`. `forge_state.json` now checkpoints live/partial progress as well as complete results,
so an interrupted service restores the run as `phase = "interrupted"` and `running = false`. A partial
run retains previous profile points for inspection but clears qualification until a complete
Standard/Long run establishes that the active boundaries are still safe. A complete Fast run may
synthesize provisional profiles, but cannot make them deployable.

The Technical Power Sweep log is permanent in Forge Progress and receives per-candidate lines for
planning, `Testing clock @ voltage`, outcome, p5/power and durable-save confirmation.

The next lower clock starts one physical VF bin above the previous clock's minimum stable anchor.
The previous clock's last power-bound `ClockDrop` remains the conservative fallback. If the optimized
warm-start cannot sustain (or plans no valid candidate), the same target retries from that fallback.
This skips known-redundant higher-voltage dwells without treating an aggressive warm-start as proof
that the clock is unsustainable.

Safe Loop semantics are also corrected: reset-clean `SilentError`/`Unstable` points are blacklisted
as frontier knowledge but do not increment `consecutive_crashes`. Only `DeviceLost`/TDR counts as a
crash and retains recovery state.

\## Backend → Frontend (2026-06-29): F2 qualification uses FailureSeekingGameLoop evidence

No IPC method, payload field, mode duration, pass count or Apply rule changes.

\- Fast and every discovery candidate continue using the steady power-heavy render. Cmax,
  near-power-limit behavior, p5 and `ClockDrop` semantics are unchanged.

\- Standard and Long reset/reapply qualification passes now use the versioned
  `FailureSeekingGameLoop`: PowerOpening, BoostEdge, HeavySpike, TextureRop, ComputeBurst,
  IdlePulse, MixedGame and PowerClosing. Each phase has independent checksum/coverage evidence.

\- Aggregate p5 from the mixed qualifier is diagnostic only and cannot produce `ClockDrop`, because
  its light phases intentionally do not represent the sustained discovery load.

\- Apply qualification now counts only current-contract qualification `Pass` evidence. Legacy or
  discovery positives may seed discovery but cannot unlock Apply. `Inconclusive` coverage does not
  mark the point bad; it retries once and then leaves the run unqualified/fail-closed.

\- A qualification `Fail` backs off automatically to the next higher physical VF bin, runs fresh
  steady `PowerRender` discovery there, and restarts all qualification passes. No manual bad-point
  registry or UI-provided prior is involved.

\- Standard/Long do not qualify old `prior_good` boundaries directly. The backend requires a fresh
  current-run `PowerRender` rediscovery before qualification can produce deployable evidence.

\- `ResetGpuTuning` remains the recovery escape hatch after TDR/interruption and is intentionally not
  blocked by the normal start/apply tuning lease. On success it resets stock, clears Safe Loop, clears
  the visible Forge checkpoint (`forge_state.json`) and returns the run view to `idle`; it does not
  erase the automatic F2 observation history. The frontend should keep Reset reachable when a run is
  stuck or Safe Loop recovery is pending.

\- Frontend action required: none. Existing Fast provisional copy, Standard/Long durations,
  `profiles_qualified` gate, progress polling and Apply behavior remain correct.



\## Backend → Frontend (2026-06-29): Reset all releases Safe Mode; new deep "forget everything" reset

Fixes the reported state where the app gets stuck in Needs Attention / Interrupted with no usable
option, persisting across manual PC restarts. Two related items — the first needs NO frontend change,
the second is a small additive request.

\- \*\*`ResetGpuTuning` now actually releases the Safe Loop latch.\*\* Previously it reset the GPU to
  stock and cleared the boot-flag + applied profile, but never rewrote `safe_loop.json`, so `safe_mode`
  and `consecutive_crashes` were effectively a one-way latch — "Reset all" could not clear a Needs
  Attention / Safe Mode state, and it survived reboots. Reset now also clears `safe_mode`, zeroes
  `consecutive_crashes` and returns Safe Loop `state` to `idle`, while PRESERVING learning (the
  unstable-region blacklist, `last_validated`, crash history) and the F2 observation frontier. UI
  effect: pressing the existing \*\*Reset all\*\* while `safe_loop.safe_mode` (or `state == "unstable"`)
  now returns the card to a normal, forgeable state. \*\*Frontend action required: none\*\* — just keep
  Reset all reachable in the Needs Attention / Interrupted branches (it already is).

\- \*\*New IPC `ResetGpuTuningFull` (additive, no params).\*\* A deeper "forget everything / start the GPU
  from zero" reset, requested alongside the normal Reset all. It does everything `ResetGpuTuning` does
  AND wipes all learning: the Safe Loop blacklist (whole record reset to default), the F2 observation
  frontier (`f2_observations.jsonl`) and legacy `gpu_knowledge.json`. Returns the same `GpuApply`
  status shape as `ResetGpuTuning`. \*\*Frontend request:\*\* add a second, clearly-secondary control near
  Reset all — e.g. "Full reset" / "Reset completo (apagar aprendizado)" — behind a stronger confirm
  dialog that spells out that learned profiles/observations are discarded. Normal Reset all stays the
  default; Full reset is the rare, destructive option.

\## Frontend implementation checkpoint (2026-06-29): post-TDR continuation wired (Codex)

\- In Needs Attention / Interrupted, the recommended action now offers \*\*Recover & continue\*\* as the
  primary path. It calls `ResetGpuTuning` to return stock and release the Safe Loop latch while
  preserving learning, then starts the selected Forge mode so the backend can continue from saved F2
  observations.

\- The existing mode picker remains available in that recovery branch, because selecting Fast/Standard/
  Long is harmless UI state and does not touch hardware until the combined recovery/start action runs.

\- `Reset all` remains a non-destructive recovery control. `Full reset` is now wired separately to
  `ResetGpuTuningFull` with a stronger confirmation that learned observations/knowledge/blacklist are
  discarded.

\- \*\*Crash accounting no longer inflated by clean restarts (informational, no payload change).\*\* A
  clean boot while already in Safe Mode no longer increments `consecutive_crashes`, and a user-initiated
  PC restart while a forge/apply was in flight is now recorded as a clean interruption (via a
  graceful-stop marker) instead of a phantom crash. `GetSafeLoopStatus.consecutive_crashes` therefore
  reads more truthfully; no field changed.



\## Backend → Frontend (2026-06-29): Cmax descent interleaves qualification (ETA may grow; no IPC change)

No IPC method or payload field changes. Standard/Long F2 discovery now qualifies each VF bin as it
descends (instead of qualifying only the deepest PowerRender point at the end), so the failure-seeking
qualifier never runs more than one bin below a proven point. Two frontend-visible effects:

\- \*\*Longer Standard/Long runs\*\* — qualification dwells now scale with the number of bins that qualify,
  not a single boundary. The existing "supervised, can take a while" framing still holds.

\- \*\*`estimated_remaining_ms` / `total_steps_estimate` start low and grow\*\* as deeper bins qualify, then
  settle. The progress bar may step backward early in a clock. These were already documented as
  estimates; no UI change is required, but avoid presenting the ETA as a firm countdown. `completed_steps`
  and the per-candidate log lines remain accurate.

\- \*\*Frontend action required: none.\*\* `profiles_qualified`, Apply gating and progress polling are
  unchanged.


\## Backend → Frontend (2026-06-30): FSGL2 default qualification (no IPC change)

No IPC method or payload field changes. This supersedes the earlier wording that a qualification
failure reruns fresh PowerRender and all qualification passes, and the temporary FSGL1 descent filter.

\- \*\*PowerRender remains measurement only.\*\* It finds sustainable/power-characterized bins and keeps
  Cmax, p5, cap and `ClockDrop` semantics comparable. It is not deployable stability evidence.

\- \*\*FSGL2 is now the descent qualifier.\*\* Standard/Long run FSGL2 pattern A 60 s and pattern B
  60 s while descending physical VF bins. FSGL1 remains available as a legacy/light profile but is not
  used by the current Standard/Long path.

\- \*\*FSGL2 is required for Apply.\*\* A deployable point must pass FSGL2 pattern A 60 s and pattern B
  60 s. `profiles_qualified == true` now means the synthesized points have current-contract FSGL2 A+B
  evidence. FSGL1-only and legacy/current discovery evidence remain provisional and keep Apply locked.

\- \*\*FSGL2 failure behavior.\*\* A real FSGL2 fail records that bin as unstable and stops the descent
  with the last FSGL2-qualified physical bin. `Inconclusive` retries once and then blocks Apply without
  marking the bin bad.

\- \*\*Frontend action required: none.\*\* Existing `profiles_qualified` UI gating, Apply enablement and
  progress polling remain correct.


\## Backend → Frontend (2026-06-30): FSGL3 golden-sample default (no IPC change)

No IPC method or payload field changes. This supersedes the FSGL2 default qualification note above.

\- \*\*FSGL3 is now the deployable qualifier.\*\* Standard/Long capture deterministic stock render
  goldens, then run FSGL3 A+B with per-frame on-GPU verification and deliberate droop bursts.
  PowerRender discovery and its Cmax/p5/`ClockDrop` semantics remain unchanged.

\- \*\*Apply now requires contract v4 FSGL3 A+B.\*\* `profiles_qualified == true` means every
  synthesized point has both current FSGL3 patterns. FSGL1/FSGL2, discovery-only and old-contract
  evidence remain provisional.

\- \*\*Stock capture may fail closed before descent.\*\* If any power/boost/texture-ROP golden is
  non-deterministic or the GPU device is lost, Forge ends at stock with a clear progress note.

\- \*\*Frontend action required: none.\*\* Existing progress polling, `profiles_qualified` gating and
  Apply enablement remain correct.



\## Backend → Frontend (2026-06-30): margin boundary, honest finish and automatic interrupted resume

The live F2 payload remains backward-compatible. Two optional `PowerSweepPoint` fields are additive:

\- `boundary_voltage_mv: Option<u32>` — learned F2 margin boundary before application policy.

\- `apply_margin_mv: Option<u32>` — effective upward margin after snapping to a physical VF bin.

`vf_table_voltage_mv` remains the exact physical bin used by Apply. For a current F2 point, UI copy
must distinguish the learned boundary from the applied VF bin instead of presenting them as the same
measurement.

`PowerSweepProgress.phase` now uses honest terminal states:

\- `finished` — complete frontier and qualified profiles; Apply may be available.

\- `provisional` — complete discovery/profile preview without qualification.

\- `incomplete` — safe partial ending; learning is preserved.

\- `interrupted` — recovery is retained. The Forge UI performs one automatic non-destructive
`ResetGpuTuning` + original `StartPowerSweep*` attempt when it reconnects. The persisted `mode` is now
the stable id `fast`, `standard` or `long`; legacy localized values remain accepted by the UI.

No new IPC method is required. Manual Stop must not be auto-resumed. Pre-hang telemetry is not a UI
safety state and must not be inferred from logs.


\## Backend → Frontend (2026-07-01): confirmed applied-bin sustained p99 and thermal validity

The live F2 payload remains backward-compatible. Existing `power_w` keeps its documented meaning as
steady-state mean power. `max_power_w` carries the real highest post-ramp PowerRender sample.
Additive `power_p99_w: Option<f32>` carries the sustained p99 and is the headline F2 profile/card
power.

\- Profile watts are calibrated at `vf_table_voltage_mv` after the unchanged application margin, not
  at `boundary_voltage_mv`.

\- F2 `perf_per_watt`, profile selection and power-bound frontier decisions use apply-bin/discovery
  p99, never mean power or the raw one-sample maximum.

\- `POWER_PEAK_PERCENTILE = 99`. P99 uses nearest-rank over every retained post-ramp sample; fewer
  than 100 samples fall back to measured raw max. No valid sample leaves p99 absent and profile
  calibration fails closed.

\- Discovery v4 compares adjacent PowerRender bins only while their p5 remains in the same clock
  regime. A p99 jump larger than both 8 W and 5% repeats the exact physical bin, with a maximum of
  three reset-clean attempts. At least two readings must agree; accepted groups use the highest
  actually measured p99. No interpolation or synthetic monotonic correction is allowed.

\- Additive observation telemetry records the attempt count/confirmation state, measured voltage
  min/avg/max/count and workload frames/FPS. A group without consensus is power-telemetry
  inconclusive and cannot enter synthesis or profile calibration.

\- `Validated` discovery still at 99% or more of the numeric cap continues to the next lower voltage
  bin. Standard/Long launch FSGL3 only from a confirmed off-cap discovery bin. FSGL3 itself, its
  golden, retry/continuity/recovery behavior and PowerRender discovery load are unchanged.

\- After the frontier is qualified and the Apply margin snaps upward, the backend fills any missing
  exact target/apply-bin p99 with a supervised discovery-only PowerRender dwell. The same v4
  anomaly/consensus rules apply. This backfill itself does not promote stability; qualification
  later runs the separate exact-Apply FSGL3 gate. Failure to confirm the backfill leaves profiles
  unavailable rather than inventing power.

\- Two optional/additive `PowerSweepPoint` fields are available: `max_temp_c: Option<f32>` and
  `thermal_throttled: bool`. Thermally throttled discovery is not eligible for profile calibration.

\- Card copy describes `power_p99_w` as measured sustained p99 and states that it is not a hard power
  limit. Frontend tolerates old payloads by falling back to `max_power_w`, then `power_w`.

\- Discovery contract is v4; v3 positive/power-bound evidence cannot enter v4 synthesis or resume.
  F2 Apply also rejects any restored profile that lacks a valid measured `power_p99_w`. The
  qualification-v4 sentence formerly here is superseded by the current contract below.

\## Backend runtime note (2026-07-01): adaptive F2 scheduling (no IPC change)

\- Compatible same-GPU discovery-v4 history and an isotonic trend over the last 3–4 qualified clocks
  may suggest the next frontier. Forge begins one physical bin above the prediction; the prediction
  is never evidence and is discarded when its inputs disagree by more than 25 mV.

\- While confirmed p99 remains at 99%+ of cap, discovery may skip 4/2/1 physical bins according to
  p5 deficit. Every jump remains bounded by 25 mV and the existing writer offset-step limit.

\- A reset-clean failure reached by a jump causes upward-only midpoint recovery. After the first
  approved off-cap point, discovery returns to adjacent-bin qualification. FSGL3, thermal handling,
  Safe Loop, Apply-bin p99 backfill, profile payloads and Apply behavior are unchanged.

\## Backend → Frontend (2026-07-02): electrical-regime reconciliation + exact-Apply v6

\- `PowerSweepPoint` adds optional/backward-compatible `p95_clock_mhz`,
  `apply_qualified` (default `false`) and `apply_qualification_version`.

\- The card keeps `target_clock_mhz` as the configured target. Display measured average, electrical
  regime p5 and sustained p95 as separate facts; neither measured percentile is a configured target.

\- A target/p5 gap beyond one 15 MHz physical bin maps to the nearest measured target at/above p5.
  The candidate inherits the maximum Apply anchor across that span. Under-anchored aliases are
  removed before synthesis; no profile power or voltage is interpolated.

\- Standard/Long set `profiles_qualified == true` only after every selected unique profile point has
  current A+B boundary evidence for its p5 regime and FSGL3 A+B evidence at its exact post-margin
  target/VF pair under qualification contract v6.
  Old/restored points lack that seal and Apply rejects them.

\- Exact Apply A and B run for five minutes each. Any inconclusive attempt requires two subsequent
  consecutive clean passes for that pattern. A reset-clean rejection also blocks lower-anchor
  aliases of the same p5 regime before backend re-synthesis; hard safety failures abort. No IPC
  method changed.

\- After A+B approval, `power_p99_w` on each selected profile is the maximum of its confirmed
  PowerRender calibration p99 and the p99 measured by the approved exact-Apply A+B pair. Frontier
  scoring remains PowerRender-homogeneous. Restored qualified v6 snapshots refresh this published
  value from `f2_observations.jsonl`; no new IPC field is required.

\## Backend ↔ Frontend (2026-07-03): automated qualification v7 + cooperative Stop

\- Qualification contract v7 replaces deployable FSGL3 A+B evidence with three automatic patterns:
  `high_fps`, `texture` and `transitions`. Standard/Long require all three at the frontier and at
  every selected exact Apply pair. Older positive qualification evidence remains readable but cannot
  unlock Apply.

\- Electrical support now uses measured `p95_clock_mhz` with zero physical-bin tolerance. `p5` remains
  the sustained performance floor; `p95` selects the highest sustained electrical regime whose
  measured Apply anchor and current v7 qualification must cover the candidate. Missing support,
  missing p95 or missing exact p99 fails closed. The highest p95 from the exact-Apply v7 set is
  reconciled again before profiles become final; a newly exposed higher regime causes re-synthesis.

\- `StopPowerSweep` is cooperative inside discovery and qualification GPU loops. Backend progress
  changes immediately to `phase == "stopping"` while the current bounded batch drains and the normal
  checked stock reset runs. A cancellation can never become positive or bad-point evidence.

\- No IPC payload field or method was removed. During a running Forge, the UI prevents overlapping
  refreshes, polls `GetPowerSweepProgress` + `GetSafeLoopStatus` at the existing fast cadence, and
  refreshes secondary diagnostics every 3 seconds. The Stop control updates optimistically to
  “Stopping…” and ignores repeated clicks.

\- The IPC-visible Forge log is bounded to its latest 240 lines to avoid cloning/serializing an
  unbounded payload. Completed measurement and qualification evidence remains durable in
  `f2_observations.jsonl`.


\## Backend ↔ Frontend (2026-07-03): stage-aware Forge time ceiling

The Forge Progress UI now presents the live remaining estimate, elapsed wall time, current estimated
run total and a separate conservative total ceiling. It does not infer Cmax or tuning policy from log
copy.

\- `elapsed_ms`, `estimated_remaining_ms`, `completed_steps`, `total_steps_estimate` and `phase`
  backward-compatible. `estimated_remaining_ms` should remain the backend's current best remaining
  estimate: frontier discovery keeps its elapsed/step self-correction, while calibration and final
  Apply use their explicit stage durations.

\- These additive/defaulted fields are now part of `PowerSweepProgress`:
  - `estimated_total_upper_ms: Option<u64>` — conservative estimated wall time from run start through
    completion. It must include work not yet appended to `total_steps_estimate`, including possible
    exact-bin power backfills and, until profile synthesis deduplicates them, up to three unique
    exact-Apply v7 qualification pairs.
  - `cmax_clock_mhz: Option<u32>` — first reset-clean sustainable real clock found by the current run.
  - `frontier_floor_clock_mhz: Option<u32>` — lowest real clock included by the 90%-of-Cmax rule.
  - `frontier_clock_count: Option<u32>` — number of real clocks in that inclusive physical domain.

\- Before Cmax, the three frontier fields and `estimated_total_upper_ms` remain `None`: a trustworthy
  inclusive 90% domain does not exist yet. As soon as Cmax is known, publish all four together and
  compute the upper estimate from the exact real-clock domain.

\- The upper estimate is not a deadline: inconclusive debt, retries or a newly exposed p95 support
  regime may raise it. It may tighten downward as uncertainty is removed, must never be below
  `elapsed_ms`, and must be refreshed when Cmax is found, a target plan is pruned/completed,
  calibration gaps are known, final Apply pairs are deduplicated, or a retry is scheduled.

\- Frontend fallback remains intentional for legacy/interrupted payloads: when
  `estimated_total_upper_ms` is absent, the UI shows “Refining” rather than manufacturing a maximum
  from duplicated tuning constants.



\## Backend ↔ Frontend (2026-07-15): Forge incident acknowledgement and field feedback

All changes are additive/defaulted. The frontend must use structured fields and must never resume an
interrupted Forge merely because the persisted phase is `interrupted`.

\- New unit request `AcknowledgeForgeIncident` releases only the acknowledgement latch. It returns the
  normal `SafeLoop` response; blacklist and incident history remain durable.

\- `SafeLoopStatus` adds `recovery_pending_ack: bool` and
  `pending_forge_incident: Option<ForgeIncident>`. When pending, Start and Apply are blocked and the
  primary action must be presented as review/continue rather than automatic recovery.

\- `PowerSweepProgress` adds `run_id: Option<String>` and ordered `run_sequence: Vec<String>`. Both
  default empty for old checkpoints. `needs_attention` is an explicit non-running phase.

\- New unit requests `ReportPowerGodforgeUnstable`, `ReportPowerBrokkrsUnstable` and
  `ReportPowerDeepCalmUnstable` resolve the chosen point from the current backend profile set. They
  add durable real-use evidence and invalidate qualification; the frontend sends no clock/voltage.

\- `ForgeLogExport` adds `run_ids: Vec<String>` and `incident_count: usize`. The human log and its
  companion JSONL are scoped to that sequence. Legacy checkpoints without run identity remain
  exportable but are labeled as legacy/global rather than presented as a clean current-run result.

\- `ResetGpuTuning` resets hardware and releases the Safe Mode latch but preserves an interrupted
  Forge checkpoint and its run identity. `ResetGpuTuningFull` remains the explicit destructive path.

(No other active backend → frontend requests)



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

