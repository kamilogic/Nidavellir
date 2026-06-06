# Nidavellir — Decision Log

Durable technical decisions and their rationale. Newest first.

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
