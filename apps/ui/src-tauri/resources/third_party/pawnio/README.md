# Bundled PawnIO (optional)

To ship the **unified installer** with a one-click PawnIO setup:

1. Download the official Windows installer from [https://pawnio.eu/](https://pawnio.eu/).
2. Rename it to exactly **`PawnIO-Setup.exe`** and place it in this folder:
   `apps/ui/src-tauri/resources/third_party/pawnio/PawnIO-Setup.exe`
3. Run `npm run tauri:build` (or CI release). The NSIS installer will offer to run it after install.

**Licensing:** Only redistribute PawnIO if your use complies with the PawnIO license and distribution terms. If this file is missing, the installer still works; the user can install PawnIO manually later.

**Silent install:** The hook runs `PawnIO-Setup.exe /S`. If your build does not support `/S`, remove silent mode in `windows/hooks.nsh` or run the installer manually after first launch.
