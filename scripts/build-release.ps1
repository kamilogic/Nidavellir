# Build nidavellir-service sidecar for Tauri bundle (externalBin naming convention).
# Run from repo root, or via npm beforeBuildCommand (cwd = apps/ui).
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
Set-Location $RepoRoot

Write-Host "[build-release] Repo root: $RepoRoot"

cargo build --release -p nidavellir-service

$rustcOut = rustc -vV
$triple = ($rustcOut | Select-String "^host: ").ToString().Replace("host:", "").Trim()
if (-not $triple) {
    throw "Could not parse rustc host triple"
}
Write-Host "[build-release] Host triple: $triple"

$srcExe = Join-Path $RepoRoot "target\release\nidavellir-service.exe"
if (-not (Test-Path $srcExe)) {
    throw "Missing $srcExe - cargo build failed?"
}

$binDir = Join-Path $RepoRoot "apps\ui\src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

$dstName = "nidavellir-service-$triple.exe"
$dstExe = Join-Path $binDir $dstName
Copy-Item -Force $srcExe $dstExe
Write-Host "[build-release] Copied sidecar to $dstExe"

$modulesSrc = Join-Path $RepoRoot "apps\ui\src-tauri\resources\pawnio-modules"
$modulesDst = Join-Path $binDir "pawnio-modules"
if (Test-Path $modulesSrc) {
    New-Item -ItemType Directory -Force -Path $modulesDst | Out-Null
    Copy-Item -Force (Join-Path $modulesSrc "*.bin") $modulesDst -ErrorAction SilentlyContinue
    Write-Host "[build-release] Copied PawnIO modules to $modulesDst"
}
