# GPU Forge — real GPU tuning (v0.3, NVAPI)

This documents the real, hardware-level GPU tuning built on the `feat/v0.3-nvapi`
work: how it characterizes, validates, applies and persists CPU/GPU profiles,
the methodology, the problems we hit, and how we solved them.

> Status: **real hardware** (NVIDIA / NVAPI). No simulation. Verified on an
> RTX 3060 Ti + i7-13700K dev rig. Final stability is always confirmed in a real
> game/benchmark — see "Honest limits".

## Components

| Crate / module | Role |
|---|---|
| `crates/gpu-nvapi` | NVAPI bindings (via the `nvapi` crate). Read V/F curve; write core clock offset, core voltage lock, memory clock offset; `reset_all`. |
| `crates/gpu-stress` | Real GPU compute battery via **wgpu/Vulkan**: ALU known-answer, memory-bound (large VRAM table), VRAM integrity, pointer-chase, bandwidth, **combined core+mem**. |
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

The live F2 Forge deliberately asks two different questions with two different render dwells:

- **Discovery / Fast:** the steady eight-instance textured `PowerRender` determines Cmax,
  near-power-limit behavior, sustained p5 and the voltage boundary. Its homogeneous load keeps
  clock/power measurements comparable across physical VF bins.
- **Standard / Long qualification:** FSGL2 A+B is the default interleaved per-bin qualifier. FSGL1
  remains available as a lighter legacy profile but is paused for the current hardware trial. The
  qualifier uses deterministic transient profiles that cross PowerOpening,
  BoostEdge, HeavySpike, TextureRop, ComputeBurst, IdlePulse, MixedGame and PowerClosing. Each phase
  has its own checksum and coverage evidence, so only an unexpected divergence is a `SilentError`.

**Interleaved per-bin descent (Standard / Long).** Discovery does not run `PowerRender` all the way to
the deepest survivable bin and only then qualify it — `PowerRender` tolerates more than the
failure-seeking qualifier (and than real games), so the deepest PowerRender point is often too aggressive
and qualifying it there risks a TDR. Instead, the descent stops at the FIRST sustained (under-cap)
point, qualifies it with FSGL2 pattern A and pattern B, and only then steps one real VF bin lower:
`PowerRender` there measures that bin's power and gates whether to attempt FSGL2 at all. Each deeper
bin is FSGL2-qualified before going lower; the first FSGL2 failure stops the descent and leaves the
last FSGL2-qualified bin as the accepted boundary. (Fast keeps descending to the PowerRender floor —
it is provisional and never qualifies.) Negative observations make the learned frontier automatically
select the deepest bin that still has current FSGL2 A+B evidence.

The qualifier is an orthogonal rejection test, not a replacement for power characterization.
Its aggregate p5 includes intentional light phases and therefore cannot create `ClockDrop`; that
classification remains exclusive to the steady discovery render. Qualification evidence is versioned
separately from discovery evidence; current Apply qualification counts only current-contract FSGL2
passes and requires both distinct patterns A+B. FSGL1-qualified, legacy-qualified and discovery-only
points remain provisional. No manual bad-point registry is encoded, and Standard/Long never qualify an
old `prior_good` boundary without current-run rediscovery first. No synthetic workload is claimed to
certify a particular game without supervised calibration.

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
