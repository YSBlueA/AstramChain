@echo off
setlocal enabledelayedexpansion
title Astram Mining Pool

echo.
echo  =====================================================
echo   ASTRAM MINING POOL  -  pool.Astramchin.com
echo  =====================================================
echo.

:: Configuration
set "POOL_URL=pool.Astramchin.com:3333"
set "NODE_EXE=Astram-node.exe"
set "WALLET_EXE=wallet-cli.exe"
set "WALLET_ADDR="

:: Check miner binary
if not exist "%NODE_EXE%" (
    echo [ERROR] %NODE_EXE% not found in current directory.
    echo.
    echo  Please download the Astram release from:
    echo  https://github.com/YSBlueA/AstramChain/releases
    echo.
    echo  Extract the ZIP and run this script from the same folder.
    echo.
    pause
    exit /b 1
)

:: Create data and wallet directories
set "ASTRAM_HOME=%APPDATA%\Astram"
if not exist "%ASTRAM_HOME%" mkdir "%ASTRAM_HOME%"
if not exist "%ASTRAM_HOME%\data" mkdir "%ASTRAM_HOME%\data"

:: Create wallet if missing
set "WALLET_FILE=%ASTRAM_HOME%\wallet.json"
if not exist "%WALLET_FILE%" (
    echo [INFO] No wallet found. Creating a new wallet...
    echo.
    "%WALLET_EXE%" generate
    if errorlevel 1 (
        echo [ERROR] Failed to create wallet.
        pause
        exit /b 1
    )
    echo.
    echo  !! IMPORTANT: Back up your wallet file !!
    echo  Location: %WALLET_FILE%
    echo  This file contains your private key.
    echo.
    pause
)

:: Read wallet address using PowerShell (handles paths with spaces)
for /f "usebackq delims=" %%A in (`powershell -NoProfile -Command "(Get-Content \"$env:APPDATA\Astram\wallet.json\" | ConvertFrom-Json).address"`) do set "WALLET_ADDR=%%A"

if "!WALLET_ADDR!"=="" (
    echo [ERROR] Could not read wallet address from %WALLET_FILE%
    echo  Check that the file exists and contains an "address" field.
    pause
    exit /b 1
)

:: Show summary
echo  Mining wallet : !WALLET_ADDR!
echo  Pool URL      : %POOL_URL%
echo  Data dir      : %ASTRAM_HOME%\data
echo.
echo  Starting miner... Press Ctrl+C to stop.
echo.

:: Launch the node
"%NODE_EXE%" --pool "%POOL_URL%" --wallet "!WALLET_ADDR!" --data-dir "%ASTRAM_HOME%\data" --http-bind "127.0.0.1:19533"

if errorlevel 1 (
    echo.
    echo [ERROR] Node exited with an error. See output above.
    pause
)
