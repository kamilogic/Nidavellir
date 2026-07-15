# Nidavellir single dev launcher.
#
# Starts BOTH halves of the app from one command and hot-reloads on change:
#   1. The Core Service — ELEVATED (admin), in its own window. Auto-rebuilds + restarts on any
#      Rust change if `cargo-watch` is installed (else runs once; install hint printed).
#   2. The Tauri UI — normal user, in THIS window, with frontend hot-reload (Vite/tauri dev).
#
# The two stay separate ON PURPOSE: the service needs elevation for PawnIO/hardware writes, and you
# do not want the whole UI running as admin. They talk over the named pipe, same as production.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $RepoRoot) { $RepoRoot = (Get-Location).Path }

# --- 1. Elevated Core Service (own admin window) ---------------------------------------------
$haveWatch = $null -ne (Get-Command cargo-watch -ErrorAction SilentlyContinue)
if ($haveWatch) {
    $svcRun = "cargo watch -c -w crates -x 'run -p nidavellir-service -- console'"
    $svcNote = "auto-rebuild ON (cargo-watch: save a .rs and the service rebuilds + restarts)"
} else {
    $svcRun = "cargo run -p nidavellir-service -- console"
    $svcNote = "auto-rebuild OFF - run 'cargo install cargo-watch' once to enable hot service reload"
}

$svcCmd = @"
Set-Location '$RepoRoot'
Write-Host '[dev] Core Service (elevated) - $svcNote' -ForegroundColor Cyan
$svcRun
"@

Write-Host "[dev] Launching elevated Core Service in a new window ($svcNote)..." -ForegroundColor Green
Start-Process powershell -Verb RunAs -ArgumentList '-NoExit', '-Command', $svcCmd

# --- 2. Tauri UI with hot-reload (this window, normal user) -----------------------------------
Set-Location (Join-Path $RepoRoot "apps\ui")
if (-not (Test-Path "node_modules")) {
    Write-Host "[dev] Installing UI dependencies (first run)..." -ForegroundColor Cyan
    npm install
}

Write-Host "[dev] Starting Tauri UI (frontend hot-reload). Close this window or Ctrl+C to stop the UI." -ForegroundColor Green
Write-Host "[dev] Note: the elevated service window is separate - close it manually when done." -ForegroundColor DarkGray
npm run tauri:dev
