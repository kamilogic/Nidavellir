# PawnIO modules (bundled)

Signed `.bin` modules from [PawnIO.Modules releases](https://github.com/namazso/PawnIO.Modules/releases) (LGPL-2.1 — see `COPYING-PawnIO.Modules`).

| File | Purpose |
|------|---------|
| `IntelMSR.bin` | MSR read/write (IA32_PERF_STATUS, therm, RAPL, …) |
| `LpcIO.bin` | Super I/O / LPC — motherboard VIN voltages (Vcore, DRAM, …) |

The Core Service loads these via `PawnIOLib.dll` (installed with PawnIO). Search order:

1. `NIDAVELLIR_PAWNIO_MODULES` env var
2. `<exe>/pawnio-modules` or `<exe>/resources/pawnio-modules`
3. Dev path: `apps/ui/src-tauri/resources/pawnio-modules`

Install the PawnIO driver from https://pawnio.eu/ (admin). Without it, WMI/`nvidia-smi` sensors still work.
