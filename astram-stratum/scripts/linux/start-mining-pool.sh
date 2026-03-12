#!/bin/bash
# Astram Pool Mining Launcher for Linux
# Connects this node to the Astram Mining Pool at pool.Astramchin.com

set -e

POOL_URL="pool.Astramchin.com:3333"
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
ASTRAM_HOME="${HOME}/.Astram"
WALLET_FILE="${ASTRAM_HOME}/wallet.json"
DATA_DIR="${ASTRAM_HOME}/data"
NODE_EXE="${SCRIPT_DIR}/Astram-node"
WALLET_EXE="${SCRIPT_DIR}/wallet-cli"

echo ""
echo "  ====================================================="
echo "   ASTRAM MINING POOL  -  pool.Astramchin.com"
echo "  ====================================================="
echo ""

# Check node binary
if [ ! -f "$NODE_EXE" ]; then
    echo "[ERROR] Astram-node not found at: $NODE_EXE"
    echo ""
    echo "  Please run this script from the extracted Astram release folder."
    exit 1
fi

# CUDA / GPU check
if command -v nvidia-smi >/dev/null 2>&1; then
    GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)
    if [ -n "$GPU_NAME" ]; then
        echo "[OK] GPU detected: $GPU_NAME"
    else
        echo "[WARN] nvidia-smi found but no GPU detected."
    fi
else
    echo "[WARN] nvidia-smi not found. Pool mining requires an NVIDIA GPU with CUDA."
    echo "       Install the NVIDIA driver and CUDA Toolkit if you have an NVIDIA GPU."
    echo ""
    read -r -p "Continue anyway? (y/N) " ans
    case "$ans" in
        [Yy]*) ;;
        *) exit 0 ;;
    esac
fi

# Create directories
mkdir -p "$ASTRAM_HOME" "$DATA_DIR"

# Create wallet if missing
if [ ! -f "$WALLET_FILE" ]; then
    echo "[INFO] No wallet found. Creating a new wallet..."
    echo ""
    "$WALLET_EXE" new
    echo ""
    echo "  !! IMPORTANT: Back up your wallet file !!"
    echo "  Location : $WALLET_FILE"
    echo "  This file contains your private key."
    echo ""
    read -r -p "Press ENTER to continue..."
fi

# Read wallet address
if command -v python3 >/dev/null 2>&1; then
    WALLET_ADDR=$(python3 -c "import json; d=json.load(open('$WALLET_FILE')); print(d['address'])" 2>/dev/null)
elif command -v jq >/dev/null 2>&1; then
    WALLET_ADDR=$(jq -r '.address' "$WALLET_FILE" 2>/dev/null)
else
    echo "[ERROR] python3 or jq is required to read the wallet address."
    exit 1
fi

if [ -z "$WALLET_ADDR" ]; then
    echo "[ERROR] Could not read wallet address from $WALLET_FILE"
    echo "        Check that the file exists and contains an 'address' field."
    exit 1
fi

# Set MINER_BACKEND from BUILD_INFO.conf
BUILD_INFO="${SCRIPT_DIR}/BUILD_INFO.conf"
export MINER_BACKEND="cuda"
if [ -f "$BUILD_INFO" ]; then
    val=$(grep '^MINER_BACKEND=' "$BUILD_INFO" | cut -d= -f2)
    if [ -n "$val" ]; then
        export MINER_BACKEND="$val"
    fi
fi

# Summary
echo "  Mining wallet : $WALLET_ADDR"
echo "  Pool URL      : $POOL_URL"
echo "  Data dir      : $DATA_DIR"
echo "  Miner backend : $MINER_BACKEND"
echo ""
echo "  Starting miner... Press Ctrl+C to stop."
echo ""

# Launch node in pool mode
exec "$NODE_EXE" \
    --pool      "$POOL_URL" \
    --wallet    "$WALLET_ADDR" \
    --data-dir  "$DATA_DIR" \
    --http-bind "127.0.0.1:19533"
