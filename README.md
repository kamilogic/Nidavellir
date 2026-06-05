# Nidavellir

Where silicon is forged to its prime.

**Nidavellir** is a safety-first GPU tuning system for Windows.

It learns how your specific GPU behaves under real load, builds GPU-specific knowledge over time, and forges transparent performance profiles instead of applying generic undervolt or overclock presets.

Nidavellir is named after the legendary realm of the dwarf smiths in Norse mythology — where impossible artifacts were forged beyond the limits of raw material.

The same idea applies here:

> Every GPU is different. Nidavellir does not assume what your silicon can do. It measures, validates, learns, and forges.

[![License: GPL-3.0](https://img.shields.io/badge/License-GPLv3-blue.svg)](./LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0f766e)
![Status](https://img.shields.io/badge/status-v0.3%20%E2%80%94%20GPU%20forge%20in%20development-1d4ed8)

---

## Current Scope

Nidavellir is currently focused on:

```text
NVIDIA GPU tuning on Windows
````

Current development priorities:

* GPU V/F curve tuning
* automatic power/efficiency profiling
* Safe Loop crash recovery
* persistent GPU knowledge
* transparent profile generation

Out of scope for the current development phase:

* CPU tuning
* RAM tuning
* AMD GPU support
* Linux support

These may return later, but the project is currently GPU-first.

---

## Why Nidavellir Exists

Most GPUs ship with aggressive boost behavior:

* high voltage headroom;
* clocks that may not sustain under real power limits;
* unnecessary heat and fan noise;
* performance limited by power or thermals;
* no clear explanation of what the best tuning point actually is.

Manual undervolting can fix this, but it requires experience.

Nidavellir aims to make this process accessible:

```text
Detect GPU
→ Forge GPU
→ Learn safe operating regions
→ Build transparent profiles
→ Apply the profile you prefer
→ Recover safely if something fails
```

---

## Profiles

Nidavellir forges three profile types.

### Godforge

Maximum sustainable performance.

Godforge is for users who want the highest stable performance their GPU can sustain under load.

It is not based on advertised boost clocks or short-lived peaks.

It is based on measured, validated, sustainable behavior.

---

### Brokkr's Best

Recommended for most users.

Brokkr's Best aims to preserve nearly all gaming performance while significantly reducing power draw, heat, and fan noise.

It is the balanced profile:

```text
strong performance
+
lower power
+
stability-first validation
```

---

### Deep Calm

Maximum efficiency.

Deep Calm prioritizes the best performance-per-watt result, even if some performance is sacrificed.

It is intended for users who want:

* lower power draw;
* lower temperatures;
* quieter operation;
* efficient daily use.

---

## Forge Knowledge

Nidavellir does not apply a fixed formula.

It builds GPU-specific knowledge over time.

Example:

```text
+180 MHz → stable
+210 MHz → stable
+225 MHz → silent error
+255 MHz → hard reboot
```

That knowledge is preserved and used to avoid repeating unsafe regions.

The long-term goal is for each GPU to become better understood over time.

---

## Safe Loop

Nidavellir is designed around the assumption that tuning can fail.

The Safe Loop system protects the user by tracking risky steps and recovering after interrupted or failed tuning attempts.

Planned/active recovery behavior includes:

* boot flags before risky operations;
* detection of interrupted tuning;
* crash/reboot classification;
* automatic return to a safe state;
* blacklist of known unsafe regions;
* no persistence of known-bad profiles.

Safety is part of the product, not an afterthought.

---

## Installing

Nidavellir is not ready for general end-user installation yet.

Installer builds may exist for testing, but the current project state is still active development.

When end-user releases are available, they will be published through:

[GitHub Releases](https://github.com/kamilogic/nidavellir/releases)

Expected release package:

```text
Nidavellir_*_x64-setup.exe
```

Requirements:

* Windows 10/11 x64
* NVIDIA GPU
* Administrator permission for the core service

---

## Building from Source

### Prerequisites

* Windows 10/11 x64
* [Rust](https://rustup.rs/) with MSVC toolchain

```powershell
rustup default stable-x86_64-pc-windows-msvc
```

* [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)

  * Desktop development with C++
* [Node.js](https://nodejs.org/) 20+

---

## Development Workflow

### Core Service

The service requires administrator privileges for hardware-level operations.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/dev-service-admin.ps1
```

Alternative:

```powershell
cargo run -p nidavellir-service -- console
```

### UI

```powershell
cd apps/ui
npm install
npm run tauri:dev
```

---

## Release Installer

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-full-release.ps1
```

Expected output:

```text
target/release/bundle/nsis/Nidavellir_*_x64-setup.exe
```

---

## Architecture

```text
Tauri + Svelte UI
        |
        | named pipe IPC
        v
Nidavellir Core Service
        |
        | NVAPI / hardware interfaces
        v
GPU
```

The UI runs without administrator privileges.

The Core Service owns privileged hardware interactions.

---

## Project Layout

```text
nidavellir/
├── apps/
│   └── ui/                 Tauri + Svelte frontend
├── crates/
│   ├── core/               Shared core logic
│   ├── gpu-nvapi/          NVIDIA GPU control and V/F curve access
│   ├── driver-pawnio/      PawnIO backend for future CPU/MSR work
│   └── service/            Windows service and tuning orchestration
├── docs/
│   ├── contracts/          UI ↔ backend contracts
│   └── ui/                 UI/UX direction and design docs
├── scripts/                Development and release scripts
├── handoff.md              Continuity document for future sessions
├── AGENTS.md               Project-wide agent instructions
└── CLAUDE.md               Claude Code backend instructions
```

---

## Current Development Status

Nidavellir is currently in active GPU-focused development.

Implemented or in progress:

* NVIDIA V/F curve read/write through modern NVAPI path;
* VF ceiling concept;
* power-aware GPU sweep;
* Safe Loop recovery model;
* Forge Knowledge persistence model;
* profile synthesis model for:

  * Godforge;
  * Brokkr's Best;
  * Deep Calm;
* GPU-first UI redesign.

Near-term focus:

* multi-clock frontier sweep;
* robust maximum sustainable clock discovery;
* improved Brokkr's Best selection;
* Deep Calm restoration;
* persistent applied profile state;
* Forge Knowledge reconstruction on service startup;
* UI polish and design system.

---

## Tests

```powershell
cargo test -p nidavellir-core -p nidavellir-driver-pawnio -p nidavellir-service
```

Additional GPU-specific tests may require compatible NVIDIA hardware and should be treated carefully.

---

## License

GPL-3.0-or-later
