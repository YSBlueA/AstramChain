#!/usr/bin/env pwsh
# Astram Pool Mining Launcher
# Connects this miner to the Astram Mining Pool at pool.astramchain.com

$ErrorActionPreference = "Stop"

# Configuration
$PoolUrl    = "pool.astramchain.com:3333"
$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$AstramHome = if ($env:APPDATA) { Join-Path $env:APPDATA "Astram" } else { Join-Path $env:USERPROFILE ".Astram" }
$WalletFile = Join-Path $AstramHome "wallet.json"
$MinerExe   = Join-Path $ScriptDir "Astram-miner.exe"
$WalletExe  = Join-Path $ScriptDir "wallet-cli.exe"

# Banner
Write-Host ""
Write-Host "  =====================================================" -ForegroundColor Cyan
Write-Host "   ASTRAM MINING POOL  -  pool.astramchain.com" -ForegroundColor Cyan
Write-Host "  =====================================================" -ForegroundColor Cyan
Write-Host ""

# Check miner binary
if (-not (Test-Path $MinerExe)) {
    Write-Host "[ERROR] Astram-miner.exe not found at: $MinerExe" -ForegroundColor Red
    Write-Host ""
    Write-Host "  Please run this script from the extracted Astram release folder." -ForegroundColor Yellow
    Read-Host "Press ENTER to exit"
    exit 1
}

# CUDA / GPU check
$gpuName = $null
try {
    $gpuName = (& nvidia-smi --query-gpu=name --format=csv,noheader 2>$null | Select-Object -First 1).Trim()
} catch {}

if ($gpuName) {
    Write-Host "[OK] GPU detected: $gpuName" -ForegroundColor Green
} else {
    Write-Host "[WARN] nvidia-smi not found. Pool mining requires an NVIDIA GPU with CUDA." -ForegroundColor Yellow
    Write-Host "       Install the NVIDIA driver and CUDA Toolkit if you have an NVIDIA GPU." -ForegroundColor Yellow
    Write-Host ""
    $ans = Read-Host "Continue anyway? (y/N)"
    if ($ans -notmatch '^[Yy]') { exit 0 }
}

# Create wallet directory
New-Item -ItemType Directory -Force -Path $AstramHome | Out-Null

# Create wallet if missing
if (-not (Test-Path $WalletFile)) {
    Write-Host "[INFO] No wallet found. Creating a new wallet..." -ForegroundColor Yellow
    Write-Host ""
    & $WalletExe generate
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[ERROR] Failed to create wallet." -ForegroundColor Red
        Read-Host "Press ENTER to exit"
        exit 1
    }
    Write-Host ""
    Write-Host "  !! IMPORTANT: Back up your wallet file !!" -ForegroundColor Yellow
    Write-Host "  Location : $WalletFile" -ForegroundColor Yellow
    Write-Host "  This file contains your private key." -ForegroundColor Yellow
    Write-Host ""
    Read-Host "Press ENTER to continue"
}

# Read wallet address
try {
    $wallet = Get-Content -Raw -Path $WalletFile | ConvertFrom-Json
    $WalletAddr = $wallet.address
} catch {
    Write-Host "[ERROR] Could not parse wallet file: $WalletFile" -ForegroundColor Red
    Write-Host "        $_" -ForegroundColor Red
    Read-Host "Press ENTER to exit"
    exit 1
}

if ([string]::IsNullOrWhiteSpace($WalletAddr)) {
    Write-Host "[ERROR] Wallet address is empty in $WalletFile" -ForegroundColor Red
    Read-Host "Press ENTER to exit"
    exit 1
}

# Set MINER_BACKEND from BUILD_INFO.conf
$BuildInfoFile = Join-Path $ScriptDir "BUILD_INFO.conf"
$env:MINER_BACKEND = "cuda"
if (Test-Path $BuildInfoFile) {
    Get-Content $BuildInfoFile | ForEach-Object {
        if ($_ -match "^MINER_BACKEND=(.+)$") { $env:MINER_BACKEND = $matches[1] }
    }
}

# Summary
Write-Host "  Mining wallet : $WalletAddr" -ForegroundColor Green
Write-Host "  Pool URL      : $PoolUrl"    -ForegroundColor Cyan
Write-Host "  Miner backend : $env:MINER_BACKEND"
Write-Host ""
Write-Host "  Starting miner... Press Ctrl+C to stop." -ForegroundColor Green
Write-Host ""

# Launch miner (reads config/minerSettings.conf automatically)
& $MinerExe

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "[ERROR] Miner exited with code $LASTEXITCODE. See output above." -ForegroundColor Red
    Read-Host "Press ENTER to exit"
}
