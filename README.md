# Nidavellir

Where silicon is forged to its prime.

**The realm of the dwarf smiths in Norse mythology.** Where Mjolnir, Gungnir, and Draupnir were forged — legendary weapons and artifacts that surpassed the limits of raw material.

Nidavellir applies the same idea to silicon: an honest, safety-first auto-tuner for CPU, GPU, and RAM on Windows. It forges validated profiles — **Godforge**, **Brokkr's Best**, and **Deep Calm** — instead of blind offsets, and never promises what your hardware cannot deliver.

[![License: GPL-3.0](https://img.shields.io/badge/License-GPLv3-blue.svg)](./LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0f766e)
![Status](https://img.shields.io/badge/status-v0.1%20%E2%80%94%20foundations-1d4ed8)

---

## Installing (end users)

1. Download **`Nidavellir_*_x64-setup.exe`** from [GitHub Releases](https://github.com/kamilogic/nidavellir/releases) (or your distribution channel).
2. Run the installer — **UAC once** (admin required for Core Service + optional kernel driver).
3. Accept the optional **PawnIO driver** step when prompted (recommended for CPU MSR access).
4. Launch **Nidavellir** from the Start Menu.

The installer includes:

- UI (Tauri + WebView2 bootstrapper)
- **Nidavellir Core Service** (auto-starts at boot)
- Optional bundled **PawnIO** setup (if shipped in that release build)

You do **not** need Rust, Node, or a separate PawnIO download for normal use.

**Requirements:** Windows 10/11 x64

---

## Building from source (developers)

### Prerequisites

- Windows 10/11 x64
- [Rust](https://rustup.rs/) with **MSVC** toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) — *Desktop development with C++*
- [Node.js](https://nodejs.org/) 20+

### Dev workflow

```powershell
# Terminal 1 — Core Service (admin required for Super I/O / PawnIO LPC)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/dev-service-admin.ps1
# Or: open PowerShell as Administrator, then:
# cargo run -p nidavellir-service -- console

# Terminal 2 — UI hot reload (normal user is fine)
cd apps/ui
npm install
npm run tauri:dev
```

### Release installer (local)

```powershell
# From repo root — builds sidecar + NSIS installer
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-full-release.ps1
```

Output: `target/release/bundle/nsis/Nidavellir_*_x64-setup.exe`

### Optional: bundle PawnIO in the installer

See [apps/ui/src-tauri/resources/third_party/pawnio/README.md](apps/ui/src-tauri/resources/third_party/pawnio/README.md).

---

## Features

- **Capability-first tuning** – detects what is actually adjustable on your machine.
- **Privileged Core Service** – UI runs without admin; only the service touches MSR/PCI.
- **PawnIO driver path** – replaces blocklisted WinRing0 (see [nidavellir-v2-plano.md](nidavellir-v2-plano.md)).
- **Installer único para o usuário final** – UI Tauri + Core Service + (opcional) driver PawnIO em um só `.exe`.
- **Safe Loop (v0.2+)** – crash-surviving recovery before aggressive tuning ships.

## Architecture

```
UI (Tauri + Svelte 5, no admin)
        |  named pipe IPC
Core Service (Windows Service)
        |  PawnIO / future GPU APIs
   Hardware
```

## Status

**v0.1 — Foundations**

| Version | Focus |
|---|---|
| v0.1 | Detection, capability probe, unified installer, Core Service, UI shell |
| v0.2 | Safe Loop — boot-flag, watchdog, crash recovery |
| v0.3 | GPU undervolt sweep (NVAPI/ADLX) |

Full roadmap: [nidavellir-v2-plano.md](nidavellir-v2-plano.md)

## Project layout

```
nidavellir/
├── apps/ui/                 Tauri UI + NSIS installer config
├── crates/
│   ├── core/                Detection, capability, IPC, sensors
│   ├── driver-pawnio/       PawnIO backend
│   └── service/             Windows Service
├── scripts/
│   ├── build-release.ps1    Build sidecar for bundle
│   └── build-full-release.ps1  Full NSIS release
└── nidavellir-v2-plano.md
```

## Tests

```powershell
cargo test -p nidavellir-core -p nidavellir-driver-pawnio -p nidavellir-service
```

## License

GPL-3.0-or-later
