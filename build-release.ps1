#!/usr/bin/env pwsh
# NetCoin Release Build Script for Windows
# This script builds all components and packages them for distribution

$ErrorActionPreference = "Stop"

function Write-Info { Write-Host "ℹ️  $args" -ForegroundColor Cyan }
function Write-Success { Write-Host "✅ $args" -ForegroundColor Green }
function Write-Error { Write-Host "❌ $args" -ForegroundColor Red }

Write-Info "NetCoin Release Builder for Windows"
Write-Host ""

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
cargo build --release

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed!"
    exit 1
}

Write-Success "Build completed successfully!"

# Copy executables
Write-Info "Copying executables..."
$Executables = @(
    "netcoin-node.exe",
    "netcoin-dns.exe",
    "netcoin-explorer.exe",
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
# NetCoin Launcher for Windows
# Usage: .\netcoin.ps1 [node|dns|explorer|wallet] [args...]

param(
    [Parameter(Position=0)]
    [ValidateSet('node', 'dns', 'explorer', 'wallet')]
    [string]$Component = 'node',
    
    [Parameter(ValueFromRemainingArguments=$true)]
    [string[]]$RemainingArgs
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

switch ($Component) {
    'node'     { $exe = "netcoin-node.exe" }
    'dns'      { $exe = "netcoin-dns.exe" }
    'explorer' { $exe = "netcoin-explorer.exe" }
    'wallet'   { $exe = "wallet-cli.exe" }
}

$exePath = Join-Path $ScriptDir $exe

if (-not (Test-Path $exePath)) {
    Write-Host "Error: $exe not found" -ForegroundColor Red
    exit 1
}

Write-Host "Starting NetCoin $Component..." -ForegroundColor Green
& $exePath @RemainingArgs
'@

Set-Content -Path "$ReleaseDir/netcoin.ps1" -Value $LauncherContent

# Copy sample config
Write-Info "Creating sample configuration..."
$ConfigContent = @'
# NetCoin Configuration Example
# Copy this file and modify as needed

# Node Settings
NODE_PORT=8333
P2P_PORT=8335

# DNS Server
DNS_PORT=8053

# Explorer
EXPLORER_PORT=3000

# Data Directory
# Windows: %USERPROFILE%\.netcoin
DATA_DIR=%USERPROFILE%\.netcoin
'@

Set-Content -Path "$ReleaseDir/config/example.conf" -Value $ConfigContent

# Create README
Write-Info "Creating README..."
$ReadmeContent = @'
# NetCoin for Windows

## Quick Start

1. Extract this archive to a folder
2. Open PowerShell in this directory
3. Run a component:

```powershell
# Run blockchain node
.\netcoin.ps1 node

# Run DNS server
.\netcoin.ps1 dns

# Run blockchain explorer
.\netcoin.ps1 explorer

# Run wallet CLI
.\netcoin.ps1 wallet
```

## Components

- **netcoin-node.exe** - Main blockchain node (HTTP: 8333, P2P: 8335)
- **netcoin-dns.exe** - DNS discovery server (Port: 8053)
- **netcoin-explorer.exe** - Web-based blockchain explorer (Port: 3000)
- **wallet-cli.exe** - Command-line wallet interface

## System Requirements

- Windows 10 or later (64-bit)
- 4GB RAM minimum
- 10GB free disk space

## Data Directory

NetCoin stores blockchain data in: `%USERPROFILE%\.netcoin`

To reset the blockchain, delete this directory while no nodes are running.

## Configuration

See `config/example.conf` for configuration options.

## Support

For issues and documentation, visit: https://github.com/yourorg/netcoin
'@

Set-Content -Path "$ReleaseDir/README.md" -Value $ReadmeContent

# Create version info
$VersionMatch = Get-Content "node/Cargo.toml" | Select-String 'version = "(.+)"' | Select-Object -First 1
$Version = if ($VersionMatch) { $VersionMatch.Matches.Groups[1].Value } else { "unknown" }
$VersionInfo = @"
NetCoin v$Version
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
Write-Host "  2. Create a ZIP archive: Compress-Archive -Path release/windows/* -DestinationPath netcoin-windows-v$Version.zip"
Write-Host "  3. Share the ZIP file with users"
