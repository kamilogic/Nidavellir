# Nidavellir — Decision Log

Durable technical decisions and their rationale. Newest first.

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
