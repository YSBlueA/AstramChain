#!/usr/bin/env pwsh
# Astram Release Build Script for Windows
# This script builds all components and packages them for distribution

$ErrorActionPreference = "Stop"

function Write-Info { Write-Host "INFO  $args" -ForegroundColor Cyan }
function Write-Success { Write-Host "OK    $args" -ForegroundColor Green }
function Write-Error { Write-Host "ERROR $args" -ForegroundColor Red }

Write-Info "Astram Release Builder for Windows"
Write-Host ""

# GPU (CUDA) is the only supported miner backend on Windows
Write-Info "Build backend: GPU (CUDA)"
$BuildBackend = "cuda"
$env:MINER_BACKEND = $BuildBackend

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

cargo build --release --workspace --exclude Astram-node --exclude Astram-explorer --exclude Astram-miner
if ($LASTEXITCODE -ne 0) { Write-Error "Build failed (workspace)!"; exit 1 }

cargo build --release -p Astram-node
if ($LASTEXITCODE -ne 0) { Write-Error "Build failed (Astram-node)!"; exit 1 }

cargo build --release -p Astram-miner --features cuda-miner
if ($LASTEXITCODE -ne 0) { Write-Error "Build failed (Astram-miner)!"; exit 1 }

cargo build --release -p Astram-explorer --features cuda-miner
if ($LASTEXITCODE -ne 0) { Write-Error "Build failed (Astram-explorer)!"; exit 1 }

Write-Success "Build completed successfully!"

# Build explorer web frontend
Write-Info "Building explorer web frontend..."
if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    Write-Error "npm is required to build explorer/web"
    exit 1
}

Push-Location "explorer/web"
if (Test-Path "package-lock.json") {
    npm ci
} else {
    npm install
}

if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Write-Error "Failed to install explorer/web dependencies"
    exit 1
}

npm run build
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Write-Error "Failed to build explorer/web"
    exit 1
}
Pop-Location

if (-not (Test-Path "explorer/web/dist")) {
    Write-Error "Missing explorer/web/dist after build"
    exit 1
}

New-Item -ItemType Directory -Force -Path "$ReleaseDir/explorer_web" | Out-Null
Copy-Item -Recurse -Force "explorer/web/dist/*" "$ReleaseDir/explorer_web/"
Write-Success "Deployed explorer web to $ReleaseDir/explorer_web"

$ExplorerConfContent = @'
window.ASTRAM_EXPLORER_CONF = {
  apiBaseUrl: "https://explorer.astramchain.com/api"
};
'@
Set-Content -Path "$ReleaseDir/explorer_web/explorer.conf.js" -Value $ExplorerConfContent
Write-Success "Created explorer_web/explorer.conf.js"

# Copy pool web
Write-Info "Copying pool web..."
$PoolWebDir = "astram-stratum/web"
if (Test-Path "$PoolWebDir/public") {
    # public/ → pool_web/  (landing page)
    New-Item -ItemType Directory -Force -Path "$ReleaseDir/pool_web" | Out-Null
    Copy-Item -Recurse -Force "$PoolWebDir/public/*" "$ReleaseDir/pool_web/"
    # root index.html → pool_web/dashboard/index.html  (stats dashboard)
    if (Test-Path "$PoolWebDir/index.html") {
        New-Item -ItemType Directory -Force -Path "$ReleaseDir/pool_web/dashboard" | Out-Null
        Copy-Item -Force "$PoolWebDir/index.html" "$ReleaseDir/pool_web/dashboard/index.html"
    }
    Write-Success "Deployed pool web to $ReleaseDir/pool_web"
} elseif (Test-Path "$PoolWebDir/index.html") {
    New-Item -ItemType Directory -Force -Path "$ReleaseDir/pool_web" | Out-Null
    Copy-Item -Recurse -Force "$PoolWebDir/*" "$ReleaseDir/pool_web/"
    Write-Success "Deployed pool web to $ReleaseDir/pool_web"
} else {
    Write-Host "WARN  Pool web not found at $PoolWebDir (skipping)" -ForegroundColor Yellow
}

# Copy executables
Write-Info "Copying executables..."
$Executables = @(
    "Astram-node.exe",
    "Astram-miner.exe",
    "Astram-stratum.exe",
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

# Copy pool mining scripts
Write-Info "Copying pool mining scripts..."
$PoolScriptsDir = "astram-stratum/scripts/windows"
if (Test-Path "$PoolScriptsDir/pool-mining.ps1") {
    Copy-Item "$PoolScriptsDir/pool-mining.ps1"      "$ReleaseDir/pool-mining.ps1"
    Copy-Item "$PoolScriptsDir/start-mining-pool.bat" "$ReleaseDir/start-mining-pool.bat"
    Write-Success "Copied pool mining scripts"
} else {
    Write-Host "WARN  Pool scripts not found at $PoolScriptsDir (skipping)" -ForegroundColor Yellow
}

# Create launcher script
Write-Info "Creating launcher script..."
$LauncherContent = @'
#!/usr/bin/env pwsh
# Astram Launcher for Windows
# Usage: .\Astram.ps1 [node|miner|stratum|dns|explorer|wallet] [args...]

param(
    [Parameter(Position=0)]
    [ValidateSet('node', 'miner', 'stratum', 'dns', 'explorer', 'wallet')]
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

# Load a .conf file (KEY=VALUE) and return as hashtable
function Load-ConfFile {
    param([string]$Path)
    $result = @{}
    if (Test-Path $Path) {
        Get-Content $Path | ForEach-Object {
            $line = $_.Trim()
            if ($line -and -not $line.StartsWith('#')) {
                if ($line -match '^([^=]+)=(.*)$') {
                    $result[$matches[1].Trim()] = $matches[2].Trim()
                }
            }
        }
    }
    return $result
}

$config = Ensure-ConfigDefaults

# Load build configuration (MINER_BACKEND)
$BuildInfoFile = Join-Path $ScriptDir "BUILD_INFO.conf"
if (Test-Path $BuildInfoFile) {
    Get-Content $BuildInfoFile | ForEach-Object {
        if ($_ -match "^([^=]+)=(.*)$") {
            $key = $matches[1]
            $value = $matches[2]
            if ($key -eq "MINER_BACKEND") {
                $env:MINER_BACKEND = if ([string]::IsNullOrWhiteSpace($value)) { "cuda" } else { $value }
            }
        }
    }
}

if ([string]::IsNullOrWhiteSpace($env:MINER_BACKEND)) {
    $env:MINER_BACKEND = "cuda"
}

switch ($Component) {
    'node'     { $exe = "Astram-node.exe" }
    'miner'    { $exe = "Astram-miner.exe" }
    'stratum'  { $exe = "Astram-stratum.exe" }
    'dns'      { $exe = "Astram-dns.exe" }
    'explorer' { $exe = "Astram-explorer.exe" }
    'wallet'   { $exe = "wallet-cli.exe" }
}

$exePath = Join-Path $ScriptDir $exe

if (-not (Test-Path $exePath)) {
    Write-Host "Error: $exe not found" -ForegroundColor Red
    exit 1
}

# Wallet auto-create: needed for node, miner (reward address) and stratum (pool fee address)
if ($Component -in @('node', 'miner', 'stratum') -and -not (Test-Path $config.wallet_path)) {
    Write-Host "Wallet file not found. Creating a new wallet at $($config.wallet_path)" -ForegroundColor Yellow
    & (Join-Path $ScriptDir "wallet-cli.exe") generate
}

# Miner: show mode from config file before starting
if ($Component -eq 'miner') {
    $MinerConf = Join-Path $ScriptDir "config\minerSettings.conf"
    $MiningMode = "pool"
    if (Test-Path $MinerConf) {
        $modeMatch = Get-Content $MinerConf | Where-Object { $_ -match "^MINING_MODE\s*=" } | Select-Object -First 1
        if ($modeMatch -match "=\s*(.+)$") { $MiningMode = $matches[1].Trim() }
    }
    Write-Host ""
    Write-Host "  Mining mode : $MiningMode" -ForegroundColor Cyan
    if ($MiningMode -eq "solo") {
        Write-Host "  Requires    : Astram-node.exe running on this machine" -ForegroundColor Yellow
        Write-Host "  Edit config\minerSettings.conf to switch to pool mode" -ForegroundColor DarkGray
    } else {
        Write-Host "  Pool        : pool.astramchain.com:3333" -ForegroundColor Cyan
        Write-Host "  Edit config\minerSettings.conf to switch to solo mode" -ForegroundColor DarkGray
    }
    Write-Host ""
}

# Stratum: load poolSettings.conf and inject as environment variables
if ($Component -eq 'stratum') {
    $PoolConf = Join-Path $ScriptDir "config\poolSettings.conf"
    $poolSettings = Load-ConfFile $PoolConf
    foreach ($key in $poolSettings.Keys) {
        [System.Environment]::SetEnvironmentVariable($key, $poolSettings[$key], 'Process')
    }

    $pNodeRpc  = if ($poolSettings['NODE_RPC_URL']) { $poolSettings['NODE_RPC_URL'] } else { 'http://127.0.0.1:19533' }
    $pStratum  = if ($poolSettings['STRATUM_BIND']) { $poolSettings['STRATUM_BIND'] } else { '0.0.0.0:3333' }
    $pStats    = if ($poolSettings['STATS_BIND'])   { $poolSettings['STATS_BIND']   } else { '0.0.0.0:8081' }

    Write-Host ""
    Write-Host "  Stratum pool server" -ForegroundColor Cyan
    Write-Host ("  Node RPC    : " + $pNodeRpc)  -ForegroundColor Cyan
    Write-Host ("  Stratum     : " + $pStratum)  -ForegroundColor Cyan
    Write-Host ("  Stats API   : " + $pStats)    -ForegroundColor Cyan
    Write-Host "  Requires    : Astram-node.exe running on this machine" -ForegroundColor Yellow
    Write-Host "  Edit config\poolSettings.conf to change pool settings" -ForegroundColor DarkGray
    Write-Host ""
}

Write-Host "Starting Astram $Component..." -ForegroundColor Green

if ($RemainingArgs -and $RemainingArgs.Count -gt 0) {
    & $exePath @RemainingArgs
} else {
    & $exePath
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

# DNS discovery server
DNS_SERVER_URL=https://seed.astramchain.com

# Network selection (default: mainnet)
# Uncomment to use testnet:
# ASTRAM_NETWORK=testnet
# Mainnet: Network ID Astram-mainnet, Chain ID 1, Network Magic 0xA57A0001
# Testnet: Network ID Astram-testnet, Chain ID 8888, Network Magic 0xA57A22B8
# Optional overrides:
# ASTRAM_NETWORK_ID=custom-network-id
# ASTRAM_CHAIN_ID=12345
# ASTRAM_NETWORK_MAGIC=0xA57A0001

# Data directory
DATA_DIR=%USERPROFILE%\.Astram\data
'@

Set-Content -Path "$ReleaseDir/config/nodeSettings.conf" -Value $NodeSettingsContent

# Create miner settings config
Write-Info "Creating miner settings configuration..."
$MinerSettingsContent = @'
# Astram Miner Settings
# This file is read by Astram-miner.exe at startup.
# Location: config/minerSettings.conf  (next to the miner binary)

# ----- Mining Mode -----------------------------------------------------------------------
# pool : Connect to a Stratum mining pool.
#        Rewards are split among pool participants, providing steady payouts.
#        Recommended for most users — no need to run a full node.
#
# solo : Mine directly against your own node (Astram-node.exe must be running).
#        100% of the block reward goes to your wallet when you find a block.
#        Best suited for high-hashrate miners or testing.
#        Requires: Astram-node.exe running on the same machine.
#
MINING_MODE=pool

# ----- Pool Mode Settings (used when MINING_MODE=pool) -----------------------------------------------------------------------
# Address and port of the Stratum mining pool server.
POOL_HOST=pool.astramchain.com
POOL_PORT=3333

# Worker name shown on the pool dashboard.
# Format: <wallet_address>.<worker_name>
# The wallet address is read automatically from your wallet file.
WORKER_NAME=worker1

# ----- Solo Mode Settings (used when MINING_MODE=solo) -----------------------------------------------------------------------
# HTTP API URL of the local Astram node.
# Change the port if you modified HTTP_PORT in nodeSettings.conf.
NODE_RPC_URL=http://127.0.0.1:19533
'@

Set-Content -Path "$ReleaseDir/config/minerSettings.conf" -Value $MinerSettingsContent
Write-Success "Created config/minerSettings.conf (default: pool mode)"

# Create pool (stratum) settings config
Write-Info "Creating pool settings configuration..."
$PoolSettingsContent = @'
# Astram Stratum Pool Settings
# This file is read by Astram-stratum.exe via the launcher (Astram.ps1 stratum).
# All values here are injected as environment variables before the process starts.
# You can also set these as system environment variables directly — they take precedence.

# ------ Node Connection----------------------------------------------------------------
# Full Astram node must be running. Stratum uses it to fetch block templates
# and submit completed blocks.
NODE_RPC_URL=http://127.0.0.1:19533

# ------ Pool Fee Address----------------------------------------------------------------
# Wallet address that receives the pool fee from every mined block.
# If left blank, the address from your wallet file is used automatically.
# POOL_ADDRESS=ASRMxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# ------ Network Ports----------------------------------------------------------------

# Stratum port: miners connect here (standard Stratum protocol)
STRATUM_BIND=0.0.0.0:3333

# getblocktemplate JSON-RPC port (for GBT-compatible mining software)
GBT_BIND=0.0.0.0:8332

# Stats REST API port: pool dashboard and monitoring
STATS_BIND=0.0.0.0:8081

# ------ Economics----------------------------------------------------------------
# Pool fee percentage deducted from each block reward before distribution (%).
POOL_FEE_PERCENT=1.0

# PPLNS window: number of recent accepted shares used for reward distribution.
# Larger = smoother payouts. Smaller = more sensitive to luck.
PPLNS_WINDOW=30000

# ------ VarDiff (variable miner difficulty) ----------------------------------------------------------------
# Minimum difficulty assigned to a miner (leading zero count).
VARDIFF_MIN=4

# Maximum difficulty assigned to a miner.
VARDIFF_MAX=1024

# Target seconds between accepted shares per miner (e.g. 15 = 1 share/15s).
# VarDiff adjusts each miner's difficulty to hit this target.
VARDIFF_TARGET_SECS=15

# ------ Payout ----------------------------------------------------------------
# Minimum pending balance (in ASRM) before a miner is paid out.
# Miners below this threshold keep accumulating until the next interval.
PAYOUT_THRESHOLD_ASRM=10

# How often (seconds) to scan and execute pending payouts. Default: 600 (10 min).
PAYOUT_INTERVAL_SECS=600

# RocksDB path for persisting miner balances across pool restarts.
POOL_DB_PATH=pool_data
'@

Set-Content -Path "$ReleaseDir/config/poolSettings.conf" -Value $PoolSettingsContent
Write-Success "Created config/poolSettings.conf"

# Create README
Write-Info "Creating README..."
$ReadmeContent = @'
# Astram for Windows

## Option A — Join the Mining Pool (Recommended)

Double-click `start-mining-pool.bat` (or right-click > Run with PowerShell on `pool-mining.ps1`).

The script will:
1. Detect your NVIDIA GPU
2. Create a wallet automatically if you don't have one
3. Connect to the pool at `pool.astramchain.com:3333`

Pool dashboard: https://pool.astramchain.com

## Option B — Run Your Own Node + Miner

Open PowerShell in this directory and run each component in a separate window:

```powershell
# 1. Start the blockchain node (syncs with the network)
.\Astram.ps1 node

# 2. Start the miner (after the node has synced)
.\Astram.ps1 miner

# Other components
.\Astram.ps1 stratum    # Run your own mining pool
.\Astram.ps1 dns        # DNS discovery server
.\Astram.ps1 explorer   # Blockchain explorer
.\Astram.ps1 wallet     # Wallet CLI
```

### Miner Mode

Edit `config\minerSettings.conf` to choose your mining mode:

- **pool** (default) — Connect to `pool.astramchain.com:3333`. No local node required.
- **solo** — Mine directly against your local node. Full block reward goes to your wallet.
  Requires `Astram-node.exe` running on the same machine.

### Running Your Own Pool (Stratum)

Edit `config\poolSettings.conf`, then:

```powershell
# Terminal 1: start the node
.\Astram.ps1 node

# Terminal 2: start the pool server
.\Astram.ps1 stratum
```

Miners connect to `<your-ip>:3333` using standard Stratum protocol.
Pool stats are available at `http://localhost:8081`.

## Components

- **start-mining-pool.bat** - One-click pool mining launcher
- **pool-mining.ps1** - PowerShell pool mining script
- **Astram-node.exe** - Main blockchain node (HTTP: 19533, P2P: 8335)
- **Astram-miner.exe** - GPU miner (pool or solo mode, NVIDIA CUDA required)
- **Astram-stratum.exe** - Stratum mining pool server (Stratum: 3333, Stats: 8081)
- **Astram-dns.exe** - DNS discovery server (Port: 8053)
- **Astram-explorer.exe** - Web-based blockchain explorer (Port: 3000)
- **wallet-cli.exe** - Command-line wallet interface

## System Requirements

- Windows 10 or later (64-bit)
- 4GB RAM minimum
- 10GB free disk space
- NVIDIA GPU (4GB+ VRAM recommended)
- NVIDIA driver + CUDA Toolkit installed (`nvcc` available)

## Mining Backend

- This release is **GPU-only**.
- Node mining backend is fixed to CUDA (`MINER_BACKEND=cuda`).

## Data Directory

Astram stores blockchain data in: `%APPDATA%\Astram`

To reset the blockchain, delete this directory while no nodes are running.

## Network Selection

Edit `config/nodeSettings.conf` to choose a network:

- Mainnet: Network ID Astram-mainnet, Chain ID 1
- Testnet: Network ID Astram-testnet, Chain ID 8888
- Mainnet Network Magic: 0xA57A0001
- Testnet Network Magic: 0xA57A22B8

## Support

- GitHub: https://github.com/YSBlueA/AstramChain
- Pool: https://pool.Astramchin.com
'@

Set-Content -Path "$ReleaseDir/README.md" -Value $ReadmeContent

# Create version info
$VersionMatch = Get-Content "node/Cargo.toml" | Select-String 'version = "(.+)"' | Select-Object -First 1
$Version = if ($VersionMatch) { $VersionMatch.Matches.Groups[1].Value } else { "unknown" }
$VersionInfo = @"
Astram v$Version
Built: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
Platform: Windows x64
Miner Backend: $BuildBackend
"@

Set-Content -Path "$ReleaseDir/VERSION.txt" -Value $VersionInfo

# Create build info file for launcher to read
$BuildInfoContent = @"
MINER_BACKEND=$BuildBackend
"@
Set-Content -Path "$ReleaseDir/BUILD_INFO.conf" -Value $BuildInfoContent

Write-Success "Release package created successfully!"
Write-Host ""
Write-Info "Release directory: $ReleaseDir"
Write-Info "To distribute: compress the folder and share the archive"
Write-Host ""
Write-Info "Next steps:"
Write-Host "  1. Test the executables in release/windows/"
Write-Host "  2. Create a ZIP archive: Compress-Archive -Path release/windows/* -DestinationPath Astram-windows-v$Version.zip"
Write-Host "  3. Share the ZIP file with users"

