# Nidavellir — Product Vision

> *Where silicon is forged to its prime.*

Nidavellir is **not** an overclock or undervolt tool. It is a system that **learns
each individual GPU's real behavior** and forges personalized profiles balancing
performance, efficiency and stability. The product is:

**Forge Knowledge → Refine Profiles → Deliver Transparent Performance.**
The sweep is just a tool to grow the forge's knowledge.

## Principles
- **Silicon Lottery First** — every GPU differs; never assume a fixed point (e.g.
  "1800 MHz @ 875 mV"). Every decision comes from *this* GPU's measurements.
- **Transparency First** — each profile states clock, voltage, power, efficiency and
  trade-offs (e.g. "1815 MHz · 181 W · −0.8% clock · −9.5% power").
- **No Babysitting** — survive TDR/reboot/failure, resume automatically, preserve
  knowledge. The program learns by **observing**, never by stress-testing during play.

## The three profiles (objective axis)
Synthesized from a **clock×power frontier** (multiple sustained clocks, real power
under game load). A separate **confidence axis** (Conservative .95 / Balanced .85 /
Aggressive .70 — the Wilson-LB gate from V2) gates eligibility for all three.

- **⚒️ Godforge — Performance First**: highest *sustainable* clock the power cap holds
  stably (NOT the advertised boost / idle peak). e.g. boost 1920 → sustained 1830 → Godforge 1830.
- **⚒️ Brokkr's Best — Balance First**: best **R = %power_saved ÷ %clock_lost** vs the
  Godforge clock. *Not* simply max MHz/W. e.g. 1830→1815 = −0.82% clock / −5.7% power → R≈6.95.
- **⚒️ Deep Calm — Efficiency First**: best **MHz/W**, accepting clock loss. e.g. 1770 MHz · 164 W · 10.79 MHz/W.

## Forge modes (first run)
- **Quick Forge (~5 min)** — limited exploration, few dwells; usable profiles fast.
- **Deep Forge (~20–30 min)** — more points/validations; high initial confidence.
- **Continuous Forging (recommended)** — Quick start + accumulated learning +
  occasional refinements; profiles improve over time. (Maps to `GpuKnowledge`.)

## Profile lifecycle
`Forged` (created, passed sweep+initial validation) → `Tempered` (validated under
synthetic + real load, no failures) → `Refined` (consistent over many sessions/hours)
→ `Legendary` (knowledge complete; active monitoring stops; re-forged only on new
driver/BIOS/GPU, a detected failure, or manual request).

## Safe Loop (reboot-survivable)
Arm a boot-flag (session, phase, candidate, ts) before each test. TDR →
`FailSeverity::Tdr`, continue. Reboot → on next boot: auto-start, detect the
interruption, record `Reboot`, recede to a safe region, resume. Limits:
**1 reboot → continue with backoff · 2 → end aggressive exploration · 3 → Safe Mode**.

## UI
The user runs **Forge GPU** / **Refine Profiles** (not "Run Sweep"), picks Godforge /
Brokkr's / Deep Calm, and keeps using the PC. A **Forge Progress** view shows %, current
stability, last discovery, and the learned frontier.

## Phase plan (status)
- **F1 — Profile model**: 3-profile synthesis with new metrics + V2 gate.
  - **F1a (DONE)** — `synthesize_forge_profiles()` (pure, unit-tested). Not yet wired.
  - **F1b (next)** — produce a real multi-clock frontier (extend the *safe flatten*
    sweep to several target clocks, measuring game-load power); decide knowledge
    keying by (clock, offset); wire synthesis into the live sweep.
- **F2** — Transparency (clock/power deltas + profile strings).
- **F3** — Forge modes (Quick/Deep/Continuous; breadth = #clock levels).
- **F4** — Reboot→knowledge auto-record + limits 1/2/3.
- **F5** — Lifecycle (Forged→Legendary).
- **F6** — Passive monitoring (no stress during play).
- **F7** — UI (Forge GPU / Refine / Forge Progress).

## Known tech debt
Two overlapping sweep engines: `gpu_power_sweep.rs` (safe flatten, single-clock,
GpuKnowledge+V2, game-power) and `gpu_sweep_real.rs`+`core::gpu_sweep` (lock-voltage
frontier, `synthesize_profiles`, Quality presets — but lock-voltage TDRs under game
load and uses an MHz/mV voltage proxy). F1b builds on the safe engine; the redundant
engine/synthesis should be consolidated.
