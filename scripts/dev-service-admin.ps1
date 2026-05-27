# Start Nidavellir Core Service elevated (required for PawnIO Super I/O on many boards).
$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if ((Split-Path -Leaf $RepoRoot) -eq "scripts") {
    $RepoRoot = Split-Path -Parent $RepoRoot
}

$cmd = @"
Set-Location '$RepoRoot'
Write-Host '[dev-service-admin] Building nidavellir-service...' -ForegroundColor Cyan
cargo build -p nidavellir-service
if (`$LASTEXITCODE -ne 0) { exit `$LASTEXITCODE }
Write-Host '[dev-service-admin] Starting Core Service (console)...' -ForegroundColor Green
cargo run -p nidavellir-service -- console
"@

Start-Process powershell -Verb RunAs -ArgumentList '-NoExit', '-Command', $cmd
