# Nidavellir ⚒️

> *The realm of the dwarf smiths in Norse mythology. Where Mjolnir, Gungnir, and Draupnir were forged — legendary weapons and artifacts that surpassed the limits of raw material.*

**Nidavellir** is an open-source tool that analyzes your hardware, stress-tests it, learns its individual silicon limits (silicon lottery), and generates 3 optimization profiles applied at hardware level via UEFI + Windows.

No manual sliders. No guessing. Just what your silicon can deliver.

---

## The 3 Profiles

After the learning phase (30min–4h), the silicon curve model of your hardware automatically generates:

| Profile | Goal | CPU | GPU | RAM |
|---|---|---|---|---|
| **Godforge** (Max) ⚡ | Maximum sustained performance | Max turbo, high PL, C-states OFF | Max stable clock, high PL | Best validated timings |
| **Brokkr's Best** (Efficient) ♻️ | Maximum efficiency (perf/watt) | Optimal undervolt, PL at knee of curve | Efficient V/F sweet spot | Best validated timings |
| **Deep Calm** (Eco) 🍃 | Savings with no perceptible loss (≥95% stock) | Undervolt + light underclock, low PL | Reduced power limit | Best validated timings |

Profile names can be customized. The key point: they are **derived from learning**, not generic presets.

---

## Architecture

### 2 Automated Phases

```
PHASE 1: WINDOWS
  CPU: MSR sweep (FIVR offset, PL1/PL2, turbo ratios, C-states)
  GPU: NVAPI/ADLX (V/F curve, power limit)
  RAM: diagnostics + SPD read (via SMBus)
  ReBAR: PCIe detection + alert if OFF
  Monitor: WHEA, watchdog, temp, power
  Output: silicon_profile.json

  ──→ AUTOMATIC REBOOT ──→

PHASE 2: UEFI
  Load profile from ESP
  RAM: timing tuning + frequency (memory controller)
  CPU: validation in isolated environment
  Refine profile → confidence rating (Bronze/Silver/Gold)
  Output: silicon_profile_refined.json

  ──→ REBOOT → WINDOWS → PROFILES READY
```

### 3 Hardware Access Layers

```
Layer 1 (Universal) — Covers ~97% of gains
  MSR, PCIe, SMBus, WMI, NVAPI, ADLX
  ✅ Everything that matters for all 3 profiles
  ✅ 100% implemented on Windows (no reboot)

Layer 2 (UEFI NVRAM DB) — BIOS settings
  Resizable BAR, XMP, C-state enables
  Community database per motherboard + BIOS version
  Future: automatic IFR parser for mapping

Layer 3 (VRM/EC) — Depth
  LLC, DIGI VRM, fan curves
  Only implemented if community DB contributions exist
  ❌ Not required for any profile
```

### Crash Handling (Safe Loop)

```
WHEA monitor → detects correctable error → reverts BEFORE crash
Boot flag    → detects post-reboot crash on next startup
Bugcheck     → analyzed → parameter marked invalid in model
Next iteration avoids the unstable region
```

---

## Tech Stack

| Layer | Technology | Rationale |
|---|---|---|
| Desktop framework | Tauri v2 | ~5MB, secure, native Rust ↔ UI IPC |
| Backend | Rust | Memory safety, performance, MSR/IO access |
| Frontend | Svelte 5 | Reactive, compiled, low boilerplate |
| Kernel driver | WinRing0 / PawnIO | MSR + PCI config + SMBus |
| GPU API | NVAPI + ADLX Rust bindings | V/F curve, power limit |
| Optimization | argmin crate (Rust) | Bayesian optimization + pattern search |
| UEFI module | EDK2 / Rust UEFI | Memory controller, boot driver |

---

## Roadmap

| Release | Modules | Deliverable |
|---|---|---|
| v0.1 | HW Detector + Monitor | Detect and display sensors |
| v0.2 | Manual Tuning | CPU/GPU sliders via MSR+NVAPI |
| v0.3 | Stress Test Engine | CPU FFT, GPU compute, RAM patterns |
| v0.4 | Auto Sweep (Layer 1) | Automated CPU+GPU sweep |
| v0.5 | 3 Profiles Generated | Modeled curve → functional profiles |
| v0.6 | ReBAR checker | Notification if OFF |
| v0.7 | UEFI Boot Driver | RAM timing tuning + validation |
| v0.8 | Full Auto Pipeline | Phase1→reboot→Phase2→profiles ready |
| v0.9 | Background Learning | Collect data during normal use |
| v1.0 | Community Database | Bootstrap + anonymous submission |

---

## Repository Structure

```
nidavellir/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── detector/       # HW detection (CPUID, WMI, SPD, SMBus)
│   │   ├── tuner/          # MSR, NVAPI, ADLX control
│   │   ├── stress/         # CPU/GPU/RAM stress tests
│   │   ├── optimizer/      # Bayesian optimization engine
│   │   ├── profile/        # Profile save/load/apply
│   │   ├── monitor/        # WHEA, watchdog, sensors
│   │   └── service/        # Windows service (auto-apply on boot)
│   ├── driver/             # WinRing0/PawnIO integration
│   └── Cargo.toml
├── src/                    # Svelte 5 frontend (Tauri webview)
│   ├── lib/
│   │   ├── components/     # UI components
│   │   ├── stores/         # Svelte stores (reactive state)
│   │   └── views/          # Page views (Dashboard, Tuner, Benchmark)
│   └── App.svelte
├── uefi/                   # EDK2 UEFI boot driver
│   ├── src/
│   ├── nidavellir.dsc      # EDK2 package declaration
│   └── nidavellir.inf      # EDK2 module definition
└── README.md
```

---

## License

**GPLv3** — open for forks, contributions, and auditing.
