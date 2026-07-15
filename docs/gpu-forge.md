# GPU Forge — real GPU tuning (v0.3, NVAPI)

This documents the real, hardware-level GPU tuning built on the `feat/v0.3-nvapi`
work: how it characterizes, validates, applies and persists CPU/GPU profiles,
the methodology, the problems we hit, and how we solved them.

> Status: **real hardware** (NVIDIA / NVAPI). No simulation. Verified on an
> RTX 3060 Ti + i7-13700K dev rig. Deployability is decided automatically by the
> conservative qualification contract, but no finite synthetic suite proves every game/driver
> path. Runtime field failures remain first-class Safe Loop evidence and can tighten the next Forge;
> manual voltage knowledge is never encoded as source policy.

## Components

| Crate / module | Role |
|---|---|
| `crates/gpu-nvapi` | NVAPI bindings (via the `nvapi` crate). Read V/F curve; write core clock offset, core voltage lock, memory clock offset; `reset_all`. |
| `crates/gpu-stress` | Real GPU compute/render battery via **wgpu** (actual selected backend/adapter/driver recorded per F2 dwell): known-answer ALU, render/ROP/texture, memory-bound VRAM, pointer-chase, bandwidth and combined core+mem. |
| `crates/service/gpu_real.rs` | `Validate stability` battery (VRAM + ALU + memory + mixed). |
| `crates/service/gpu_sweep_real.rs` | Core undervolt/OC sweep (lock voltage, raise clock, combined load, Phase-E soak). |
| `crates/service/gpu_mem_sweep.rs` | Memory sweep — finds the GDDR6 **effective-bandwidth peak** with consistency + combined soak. |
| `crates/service/gpu_apply.rs` | Apply a profile to hardware, **persist**, and **re-apply on boot** (Safe Loop gated). |
| `crates/service/gpu_forge_all.rs` | One-click full pipeline. |
| `apps/ui` (Forge tab) | Read curve (chart), validate, sweeps (terminal log), apply, "Forge everything". EN/PT i18n. |

## The methodology (order matters)

```
1. VRAM integrity gate (stock)         — bad memory? stop, tuning won't help.
2. Core undervolt/OC  (combined load)  — Vcore is the shared rail; fix it FIRST.
3. Apply core.
4. Memory bandwidth peak (combined, at the applied core) — valid vs final Vcore.
5. Final whole-package soak (core+mem together) — the real-world judge.
6. Apply + persist  → re-applied on every boot (volatile offsets), Safe Loop gated.
```

**Why this order:** the GPU **core voltage (Vcore) powers both the shaders and
the on-die memory controller (MC)**. If you tune memory first and then undervolt
the core, the MC loses voltage and the memory OC becomes unstable (re-validation
cascade). Fixing Vcore first means memory is tuned against the final condition.

**Why combined load every step:** in a game the core and memory work at once,
loading the shared rail + power + thermals. Testing either axis in isolation
passes clocks that stutter in games. Every dwell runs `run_combined`, which
saturates three things simultaneously: **ALU** (shader cores), a
**bandwidth-streaming VRAM kernel** (memory-controller / DRAM throughput — the
real current draw through the shared rail, like a game), and a **pointer-chase**
(memory latency/addressing). A latency-bound pointer-chase alone leaves memory
util low (~20 %) and under-loads the rail; the bandwidth stream is what makes the
dwell game-realistic. ALU + chase are known-answer checked; the bandwidth stream
is load-only.

### Live F2 render workload split

The live F2 Forge asks two separate questions: homogeneous `PowerRender` characterizes power and
the voltage boundary; the failure-seeking qualifier tries to reject a point before it can become a
deployable profile. The current contracts are discovery v5 and qualification **v16**.

**Deterministic stock normalization and clock domain.** Before any candidate write, Forge resets to
stock and runs up to six 10 s preheat windows. It requires two consecutive usable windows with no
thermal throttle or telemetry failure, an end-temperature difference no greater than 2 °C, and a p5
difference no greater than 30 MHz. Failure to converge aborts before tuning. Forge then reports three
different facts instead of treating every upper clock as “Cmax”:

- **Ctable** — the maximum clock and bin count in the sane static physical V/F table.
- **Cboost** — the maximum live boost observed after stock preheat.
- **Cmax** — the first reset-clean sustainable clock actually proved by discovery.

Only live bins that also have a sane static-table identity and do not exceed Cboost enter the initial
candidate domain. Once Cmax exists, the measured profile frontier remains the inclusive Cmax→90%
Cmax range.

**Candidate Transaction (discovery).** One candidate attempt is one owned transaction:

1. Arm the Safe Loop boot flag, apply the anchored curve once, and verify the positive offset.
2. Run `PowerRender` and, for Standard/Long, the active qualification phases without resetting or
   reapplying between them; every phase therefore observes the same curve instance.
3. Perform one checked reset to stock, then clear the boot flag exactly once.
4. Persist same-curve `Qualification` observations before the `Discovery` observation.

No positive phase is reusable before step 3 proves both `reset_to_stock_ok` and
`boot_flag_cleared`. A reset, clear or persistence failure is terminal/inconclusive, never positive;
device loss retains the boot flag for recovery. A p99-consensus retry closes its current transaction
cleanly before a new attempt is armed.

**Power-cap hysteresis.** A valid numeric board limit outranks the sampled cap flag. Sustained p99 is
`NearCap` at **≥99%**, `OffCap` at **≤98%**, and `Ambiguous` strictly between those thresholds;
missing/invalid p99 is also ambiguous. If the numeric limit is unavailable, the sampled cap flag is
the compatibility fallback. The ambiguous band receives bounded exact-candidate retries; persistent
ambiguity stays inconclusive and cannot define the frontier. `ClockDrop` classification remains
exclusive to homogeneous `PowerRender`, not the qualifier's light/heavy mix.

**Qualification v16 provenance and integrity.** Every current dwell records the service build
version/revision, semantic workload fingerprint, actual selected wgpu backend, adapter and driver
identity/details, checksum method, and the stock-golden capture configuration/values. Older JSONL
lines remain readable, but pre-v16 positive evidence cannot unlock Apply. Current positive discovery,
frontier qualification and exact-Apply qualification additionally require proven transaction cleanup.

`MixedGame` is now genuinely interleaved: every frame records BoostEdge, TextureRop and PowerRender
as three render passes in one encoder/frame/submit instead of time-slicing whole workload blocks.
BoostEdge and MixedGame use a GPU reduction/compare every 16 frames. The check is sparse to keep the
render workload dominant, but mismatch state is cumulative: every sampled mismatch contributes to
the final verdict and `checksum_count` reports the checks actually performed.

The qualifier remains an orthogonal rejection test, not a replacement for power characterization.
Stock goldens are session-scoped; no old `prior_good` can become current without current-run evidence.
No finite synthetic suite is a proof over every future game/driver path.

**Applied-bin power and electrical reconciliation.** The learned boundary and applied point differ:
the policy adds +12 mV and snaps upward to a physical V/F bin. Profile synthesis therefore requires
current, thermally valid `PowerRender` p99/p5 calibration at that exact apply bin. A calibrated point
whose measured p95 reaches a higher electrical regime must use that regime's measured Apply anchor
and current qualification; no power or voltage is interpolated.

**Exact-Apply stability closure.** Standard/Long remain provisional until every unique selected
`(target, Apply VF bin)` completes, in order, **Texture for 5 minutes**, **TransitionShock for
8 minutes**, and **Endurance for 20 minutes**. Adding voltage can expose a higher sustained boost
regime, so this gate is not inherited from the lower boundary. A reset-clean rejection removes the
candidate and triggers re-synthesis; inconclusive evidence remains debt; hard recovery failures abort.
The published profile power remains the conservative maximum confirmed across homogeneous
PowerRender calibration and the approved exact-Apply dwells.

The field-failed **1845 MHz @ 862 mV** point is the calibration discriminator for the next physical
A/B against known-safe bins. **DX11 coverage is not implemented and the final gate is not shortened**
until that A/B shows that the proposed change distinguishes the failed point without losing safe-bin
specificity. The long Endurance stage remains necessary because the latest field failure appeared
well after the shorter Texture and TransitionShock stages had passed.

**Cooperative cancellation and UI headroom.** Every live discovery/qualification render receives
the Forge cancellation token and checks it between bounded GPU frames/dispatches. Stop enters
`stopping`, submits no new batches, drains the current bounded work and performs checked transaction
cleanup. Cancellation is recorded as inconclusive/cancelled, never as bad or validated evidence. The
UI reads structured progress fields rather than parsing logs; completed evidence remains durable in
`f2_observations.jsonl`.

## Problems hit → solutions

- **The GPU wasn't actually being stressed** (sat at ~4% util / 64 W). Kernels
  were tiny (~6e9 ops, <1 ms); the elapsed time was CPU-side overhead.
  **Fix:** sustained back-to-back dispatch loops that saturate the GPU
  (100 % / ~177 W), with an **LCG jump-ahead** (affine fast-exponentiation) so
  the CPU reference is O(log n) regardless of how many rounds ran.

- **Detecting instability before a crash.** Undervolt fails "gently" (silent
  compute errors — caught by known-answer tests *before* a hard hang). Raising
  the clock fails "hard" (TDR / device lost, often with **no** silent-error
  warning — a hung shader, not a wrong number). You cannot always pre-empt a
  TDR; the goal is to make it rare, informative, and recoverable. **Mitigations:**
  - **Near-cliff fine stepping:** once we know a cliff from a higher voltage,
    the clock step shrinks (e.g. 15 → 5 MHz) as we approach it, so the next
    probe is likelier to land in the silent-error zone than to TDR.
  - **Device-lost is non-fatal:** a TDR sets the ceiling at the last stable
    reading, the wgpu device is **recreated**, and the sweep continues with the
    remaining voltages and the long validation — it never throws away the work
    it already found, and the pipeline still delivers/persists a profile.
  - longer dwell, stop at first silent error, large margin, and the **Safe Loop**
    (boot-flag) so a crash that does reach the driver recovers on reboot and the
    bad profile is not re-applied.

- **VRAM truncated curve.** NVAPI splits the V/F table into two arrays; reading
  only the first cut the curve at ~943 mV. **Fix:** read both → full 450–1087 mV
  curve, matching MSI Afterburner.

- **GDDR6 memory validation is hard.** On-die link CRC *corrects/retries* errors,
  hiding them from a linear read/verify, while consumer cards expose no ECC
  counters. **Fixes:**
  - **Pointer-chase** test: a wrong read derails the whole chase (cascade) —
    far more sensitive to uncorrected/addressing errors than linear verify.
  - **Bandwidth consistency**, not peak: taking the peak hid the dips that *are*
    the in-game stutters (CRC retries). We measure (peak, min) per clock; a
    clean clock holds steady, an unstable one dips. Stop at the first
    **inconsistent** clock (min/peak < ~93 %).
  - **Combined core+mem soak**: memory-only tests passed clocks (e.g. +900 MHz)
    that stuttered in games because the shared rail wasn't loaded. The combined
    soak + back-off recedes until a clock survives game-like load.

- **Bandwidth peak ≠ best.** Past the GDDR6 ECC/CRC wall, more MHz = more
  correction = *less* real bandwidth. We find the effective-bandwidth peak/knee,
  not the max clock — better than Afterburner's "crank until artifacts".

## The elastic V/F ceiling (how the undervolt is applied)

A hard voltage lock or a rigid clock pin makes the GPU run a fixed clock at a fixed
voltage. Under a heavy, near-power-cap game load that removes the card's ability to
manage its own power, and it **TDRs** (driver reset / black screen). MSI Afterburner
avoids this by editing the **V/F curve** instead: it keeps the curve free below a
chosen point and **flattens it to the right** of that point. The card still drops
clocks/voltage on light load (elasticity preserved), but never boosts past the
validated point.

Nidavellir does the same via the modern NVAPI **`ClkVfPoints`** family (per-point
curve offsets), which is what Afterburner/the NVIDIA app use on current drivers:

- **Read** the live curve — `(index → voltage → frequency)` per point via
  `ClkVfPointsGetStatus`.
- **Apply a ceiling** at the validated `(voltage Vp, clock Fp)`: every point whose
  voltage ≥ Vp gets a per-point frequency offset that flattens it to Fp; lower-
  voltage points are left untouched (elastic). No voltage lock, no clock pin.
- **Reset** zeroes every point's offset.

Verified on an RTX 3060 Ti (driver 595.97): applying a ceiling drops the clock the
card sustains under load to the ceiling value (and its power with it), and reset
restores stock boost — all while the card keeps managing its own power.

If the modern API is **not** available (older driver, or a GPU that doesn't expose
it), apply falls back to a global clock offset + an NVML max-clock cap — less
elastic, but it works everywhere. The Forge view shows which mode is active.

### Supported GPUs for the V/F-curve method

The elastic ceiling needs NVIDIA's per-point curve API, present on **desktop
Pascal and newer**:

- **Supported:** GTX 10-series (Pascal), GTX 16-series (Turing), RTX 20 (Turing),
  RTX 30 (Ampere), RTX 40 (Ada), RTX 50 (Blackwell) — desktop cards on a current
  driver (R550+; verified on 595.97).
- **Fallback (offset + clock cap):** Maxwell and older; cards/drivers that don't
  expose `ClkVfPoints`; most **laptop** GPUs (vendor-locked curves).
- **Not supported:** non-NVIDIA GPUs (NVAPI is NVIDIA-only) — AMD is on the roadmap.

The program **detects this at runtime** (`vf_curve_supported()`), so the UI always
reflects what your exact GPU + driver actually allow rather than a static list.

## Apply & persist

GPU offsets are **volatile** (lost on reboot/driver reload). "Apply" writes the
profile (lock voltage + clock offset / memory offset) **and persists it**; the
Core Service **re-applies it on every boot** — gated by the Safe Loop: if the
boot-flag is still armed (last apply crashed) or Safe Mode is active, it is **not**
re-applied. "Reset to stock" clears it.

## Honest limits

- No synthetic test fully certifies consumer GDDR6 stability (ECC masks errors,
  no counters). The tool gets close (combined load, consistency, long soak) and
  applies margin — **final confirmation is a real game/benchmark session.**
- Finding the absolute OC ceiling (Godforge) inherently risks a TDR/black screen;
  the Safe Loop makes it recoverable. For safety, prefer undervolt + moderate OC.

## Next steps (deferred)

- AMD path (ADLX) — currently NVIDIA/NVAPI only.
- Persisted knowledge base / community priors (roadmap v0.7+).
- CPU and RAM tuning (project is GPU-focused first by design).
- UI polish (test layer for now).
