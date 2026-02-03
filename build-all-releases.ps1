#!/usr/bin/env pwsh
# Cross-platform Release Builder
# This script helps coordinate building releases for all platforms
# Note: You can only build native binaries on each platform

$ErrorActionPreference = "Stop"

function Write-Info { Write-Host "ℹ️  $args" -ForegroundColor Cyan }
function Write-Success { Write-Host "✅ $args" -ForegroundColor Green }
function Write-Warning { Write-Host "⚠️  $args" -ForegroundColor Yellow }
function Write-Error { Write-Host "❌ $args" -ForegroundColor Red }

Write-Info "NetCoin Multi-Platform Release Builder"
Write-Host ""

# Detect current platform
if ($IsWindows -or $env:OS -eq "Windows_NT") {
    $CurrentPlatform = "Windows"
} elseif ($IsMacOS) {
    $CurrentPlatform = "macOS"
} elseif ($IsLinux) {
    $CurrentPlatform = "Linux"
} else {
    Write-Error "Unknown platform"
    exit 1
}

Write-Info "Current platform: $CurrentPlatform"
Write-Host ""

# Build for current platform
Write-Info "Building release for $CurrentPlatform..."
if ($CurrentPlatform -eq "Windows") {
    & .\build-release.ps1
} else {
    & bash build-release.sh
}

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed!"
    exit 1
}

Write-Success "Release built successfully!"
Write-Host ""

# Show next steps
Write-Info "Release Package Information:"
Write-Host ""

if ($CurrentPlatform -eq "Windows") {
    $versionMatch = Get-Content "node/Cargo.toml" | Select-String 'version = "(.+)"' | Select-Object -First 1
    $version = if ($versionMatch) { $versionMatch.Matches.Groups[1].Value } else { "unknown" }
    $releaseDir = "release\windows"
    $archiveName = "netcoin-windows-v$version.zip"
    
    Write-Host "  Platform:  Windows x64"
    Write-Host "  Location:  $releaseDir"
    Write-Host "  Archive:   $archiveName"
    Write-Host ""
    Write-Info "To create distribution archive:"
    Write-Host "  Compress-Archive -Path $releaseDir\* -DestinationPath $archiveName"
} else {
    $platform = $CurrentPlatform.ToLower()
    $releaseDir = "release/$platform"
    
    Write-Host "  Platform:  $CurrentPlatform x64"
    Write-Host "  Location:  $releaseDir"
    Write-Host ""
    Write-Info "To create distribution archive:"
    Write-Host "  tar -czf netcoin-$platform.tar.gz -C release $platform"
}

Write-Host ""
Write-Warning "Cross-compilation notes:"
Write-Host "  To build for other platforms, run this script on each target platform:"
Write-Host "  - Windows: Run build-release.ps1"
Write-Host "  - Linux:   Run build-release.sh"
Write-Host "  - macOS:   Run build-release.sh"
Write-Host ""
Write-Info "After building on all platforms, you will have:"
Write-Host "  release/windows/ - Windows binaries"
Write-Host "  release/linux/   - Linux binaries"
Write-Host "  release/macos/   - macOS binaries"
