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
passes clocks that stutter in games. Every dwell runs `run_combined` (ALU +
pointer-chase simultaneously).

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
