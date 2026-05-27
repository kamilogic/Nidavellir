# Full release: sidecar + frontend + NSIS installer (run from repo root)
$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
Set-Location $RepoRoot

& "$ScriptDir\build-release.ps1"

Set-Location (Join-Path $RepoRoot "apps\ui")
if (-not (Test-Path "node_modules")) {
  npm install
}
npm run tauri:build

Write-Host ""
Write-Host "Installer output: target\release\bundle\nsis\"
