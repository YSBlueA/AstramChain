#!/usr/bin/env pwsh
# Astram Release Build Script for Windows
# This script builds all components and packages them for distribution

$ErrorActionPreference = "Stop"

function Write-Info { Write-Host "INFO  $args" -ForegroundColor Cyan }
function Write-Success { Write-Host "OK    $args" -ForegroundColor Green }
function Write-Error { Write-Host "ERROR $args" -ForegroundColor Red }

Write-Info "Astram Release Builder for Windows"
Write-Host ""

function Select-Backend {
    Write-Info "Select build backend:"
    Write-Host "  1) CPU"
    Write-Host "  2) GPU (CUDA)"
    $choice = Read-Host "Choose [1-2] (default: 1)"
    switch ($choice) {
        "2" { return "cuda" }
        "gpu" { return "cuda" }
        "GPU" { return "cuda" }
        default { return "cpu" }
    }
}

$BuildBackend = Select-Backend
Write-Info "Build backend: $BuildBackend"

$NodeFeatureArgs = @()
$ExplorerFeatureArgs = @()
if ($BuildBackend -eq "cuda") {
    $NodeFeatureArgs += @("--features", "cuda-miner")
    $ExplorerFeatureArgs += @("--features", "cuda-miner")
    $env:MINER_BACKEND = "cuda"
} else {
    $NodeFeatureArgs += @("--no-default-features")
    $ExplorerFeatureArgs += @("--no-default-features")
    $env:MINER_BACKEND = "cpu"
}

# Clean previous release
$ReleaseDir = "release/windows"
if (Test-Path $ReleaseDir) {
    Write-Info "Cleaning previous release..."
    Remove-Item -Recurse -Force $ReleaseDir
}

# Create release directory
Write-Info "Creating release directory..."
New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
New-Item -ItemType Directory -Force -Path "$ReleaseDir/config" | Out-Null

# Build all components in release mode
Write-Info "Building all components in release mode..."
cargo build --release --workspace --exclude Astram-node --exclude Astram-explorer
cargo build --release -p Astram-node @NodeFeatureArgs
cargo build --release -p Astram-explorer @ExplorerFeatureArgs

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed!"
    exit 1
}

Write-Success "Build completed successfully!"

# Copy executables
Write-Info "Copying executables..."
$Executables = @(
    "Astram-node.exe",
    "Astram-dns.exe",
    "Astram-explorer.exe",
    "wallet-cli.exe"
)

foreach ($exe in $Executables) {
    $source = "target/release/$exe"
    if (Test-Path $source) {
        Copy-Item $source "$ReleaseDir/$exe"
        Write-Success "Copied $exe"
    } else {
        Write-Error "Missing: $exe"
    }
}

# Create launcher script
Write-Info "Creating launcher script..."
$LauncherContent = @'
#!/usr/bin/env pwsh
# Astram Launcher for Windows
# Usage: .\Astram.ps1 [node|dns|explorer|wallet] [args...]

param(
    [Parameter(Position=0)]
    [ValidateSet('node', 'dns', 'explorer', 'wallet')]
    [string]$Component = 'node',
    
    [Parameter(ValueFromRemainingArguments=$true)]
    [string[]]$RemainingArgs
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$DefaultBase = if ($env:APPDATA) { Join-Path $env:APPDATA "Astram" } else { Join-Path $env:USERPROFILE ".Astram" }
$DefaultConfigFile = Join-Path $DefaultBase "config.json"
$DefaultWalletPath = Join-Path $DefaultBase "wallet.json"

function Ensure-ConfigDefaults {
    if (-not (Test-Path $DefaultConfigFile)) {
        New-Item -ItemType Directory -Force -Path (Split-Path $DefaultConfigFile -Parent) | Out-Null
        $defaultConfig = @{
            wallet_path = $DefaultWalletPath
            node_rpc_url = "http://127.0.0.1:19533"
        }
        $defaultConfig | ConvertTo-Json -Depth 3 | Set-Content -Path $DefaultConfigFile
    }

    try {
        $config = Get-Content -Raw -Path $DefaultConfigFile | ConvertFrom-Json
    } catch {
        $config = [pscustomobject]@{}
    }

    $changed = $false
    if (-not $config.wallet_path -or [string]::IsNullOrWhiteSpace($config.wallet_path)) {
        $config | Add-Member -Force -NotePropertyName wallet_path -NotePropertyValue $DefaultWalletPath
        $changed = $true
    }
    if (-not $config.node_rpc_url -or [string]::IsNullOrWhiteSpace($config.node_rpc_url)) {
        $config | Add-Member -Force -NotePropertyName node_rpc_url -NotePropertyValue "http://127.0.0.1:19533"
        $changed = $true
    }
    if (-not (Test-Path $config.wallet_path)) {
        $config.wallet_path = $DefaultWalletPath
        $changed = $true
    }

    if ($changed) {
        New-Item -ItemType Directory -Force -Path (Split-Path $DefaultConfigFile -Parent) | Out-Null
        $config | ConvertTo-Json -Depth 3 | Set-Content -Path $DefaultConfigFile
    }

    New-Item -ItemType Directory -Force -Path (Split-Path $config.wallet_path -Parent) | Out-Null

    return $config
}

$config = Ensure-ConfigDefaults

switch ($Component) {
    'node'     { $exe = "Astram-node.exe" }
    'dns'      { $exe = "Astram-dns.exe" }
    'explorer' { $exe = "Astram-explorer.exe" }
    'wallet'   { $exe = "wallet-cli.exe" }
}

$exePath = Join-Path $ScriptDir $exe

if (-not (Test-Path $exePath)) {
    Write-Host "Error: $exe not found" -ForegroundColor Red
    exit 1
}

if ($Component -eq 'node' -and -not (Test-Path $config.wallet_path)) {
    Write-Host "Wallet file not found. Creating a new wallet at $($config.wallet_path)" -ForegroundColor Yellow
    & (Join-Path $ScriptDir "wallet-cli.exe") generate
}

Write-Host "Starting Astram $Component..." -ForegroundColor Green
if ($Component -eq 'node') {
    # Open browser in background after a delay
    Start-Job -ScriptBlock {
        Start-Sleep -Seconds 10
        Start-Process "http://localhost:19533"
    } | Out-Null
    
    # Run node in current console
    if ($RemainingArgs -and $RemainingArgs.Count -gt 0) {
        & $exePath @RemainingArgs
    } else {
        & $exePath
    }
} else {
    if ($RemainingArgs -and $RemainingArgs.Count -gt 0) {
        & $exePath @RemainingArgs
    } else {
        & $exePath
    }
}
'@

Set-Content -Path "$ReleaseDir/Astram.ps1" -Value $LauncherContent

# Create node settings config
Write-Info "Creating node settings configuration..."
$NodeSettingsContent = @'
# Astram Node Settings
# Update addresses and ports as needed

# P2P listener
P2P_BIND_ADDR=0.0.0.0
P2P_PORT=8335

# HTTP API server
HTTP_BIND_ADDR=127.0.0.1
HTTP_PORT=19533

# Ethereum JSON-RPC server
ETH_RPC_BIND_ADDR=127.0.0.1
ETH_RPC_PORT=8545

# DNS discovery server
DNS_SERVER_URL=http://161.33.19.183:8053

# Network selection (default: mainnet)
# Uncomment to use testnet:
# ASTRAM_NETWORK=testnet
# Mainnet: Network ID Astram-mainnet, Chain ID 1
# Testnet: Network ID Astram-testnet, Chain ID 8888
# Optional overrides:
# ASTRAM_NETWORK_ID=custom-network-id
# ASTRAM_CHAIN_ID=12345

# Data directory
DATA_DIR=%USERPROFILE%\.Astram\data
'@

Set-Content -Path "$ReleaseDir/config/nodeSettings.conf" -Value $NodeSettingsContent

# Create README
Write-Info "Creating README..."
$ReadmeContent = @'
# Astram for Windows

## Quick Start

1. Extract this archive to a folder
2. Open PowerShell in this directory
3. Run a component:

```powershell
# Run blockchain node
.\Astram.ps1 node

# Run DNS server
.\Astram.ps1 dns

# Run blockchain explorer
.\Astram.ps1 explorer

# Run wallet CLI
.\Astram.ps1 wallet
```

## Components

- **Astram-node.exe** - Main blockchain node (HTTP: 19533, P2P: 8335)
- **Astram-dns.exe** - DNS discovery server (Port: 8053)
- **Astram-explorer.exe** - Web-based blockchain explorer (Port: 3000)
- **wallet-cli.exe** - Command-line wallet interface

## System Requirements

- Windows 10 or later (64-bit)
- 4GB RAM minimum
- 10GB free disk space

## Data Directory

Astram stores blockchain data in: `%USERPROFILE%\.Astram`

To reset the blockchain, delete this directory while no nodes are running.

## Network Selection

Edit `config/nodeSettings.conf` to choose a network:

- Mainnet: Network ID Astram-mainnet, Chain ID 1
- Testnet: Network ID Astram-testnet, Chain ID 8888

## Support

For issues and documentation, visit: https://github.com/yourorg/Astram
'@

Set-Content -Path "$ReleaseDir/README.md" -Value $ReadmeContent

# Create version info
$VersionMatch = Get-Content "node/Cargo.toml" | Select-String 'version = "(.+)"' | Select-Object -First 1
$Version = if ($VersionMatch) { $VersionMatch.Matches.Groups[1].Value } else { "unknown" }
$VersionInfo = @"
Astram v$Version
Built: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
Platform: Windows x64
"@

Set-Content -Path "$ReleaseDir/VERSION.txt" -Value $VersionInfo

Write-Success "Release package created successfully!"
Write-Host ""
Write-Info "Release directory: $ReleaseDir"
Write-Info "To distribute: compress the folder and share the archive"
Write-Host ""
Write-Info "Next steps:"
Write-Host "  1. Test the executables in release/windows/"
Write-Host "  2. Create a ZIP archive: Compress-Archive -Path release/windows/* -DestinationPath Astram-windows-v$Version.zip"
Write-Host "  3. Share the ZIP file with users"

