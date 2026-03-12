#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
#  Astram Mining Pool — Linux start script
#  pool.Astramchin.com
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Colours ───────────────────────────────────────────────────────────────────
BOLD='\033[1m'; RESET='\033[0m'
CYAN='\033[36m'; GREEN='\033[32m'; YELLOW='\033[33m'; RED='\033[31m'

info()  { echo -e "${CYAN}[INFO]${RESET}  $*"; }
ok()    { echo -e "${GREEN}[OK]${RESET}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
error() { echo -e "${RED}[ERROR]${RESET} $*" >&2; exit 1; }

echo -e "${BOLD}"
cat <<'BANNER'
  ___         _
 / _ \  ___  | |_  _ _  __ _  _ __
 \__, / (_-< |  _|| '_|/ _` || '  \
  /_/  |/__/  \__||_|  \__,_||_|_|_|
 Mining Pool  |  pool.Astramchin.com
BANNER
echo -e "${RESET}"

# ── Configuration ─────────────────────────────────────────────────────────────
POOL_URL="pool.Astramchin.com:3333"
DATA_DIR="${HOME}/.Astram/data"
WALLET_FILE="${HOME}/.Astram/wallet.json"
NODE_EXE="./Astram-node"
WALLET_EXE="./wallet-cli"

# Allow override via environment
POOL_URL="${POOL_URL:-pool.Astramchin.com:3333}"
WALLET_ADDR="${WALLET_ADDR:-}"

# ── Prerequisite checks ───────────────────────────────────────────────────────
if [[ ! -x "$NODE_EXE" ]]; then
  error "Astram-node not found (or not executable) in current directory.
  Please download the Astram release from:
    https://github.com/YSBlueA/AstramChain/releases
  Extract the archive and run this script from the extracted folder."
fi

# Check for NVIDIA GPU
if ! command -v nvidia-smi &>/dev/null; then
  warn "nvidia-smi not found. CUDA GPU mining requires an NVIDIA GPU and driver."
else
  GPU=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1 || true)
  [[ -n "$GPU" ]] && ok "GPU detected: $GPU"
fi

# ── Create wallet if missing ──────────────────────────────────────────────────
mkdir -p "$(dirname "$WALLET_FILE")"

if [[ ! -f "$WALLET_FILE" ]]; then
  info "No wallet found. Creating a new wallet..."
  echo
  "$WALLET_EXE" generate || error "Failed to create wallet."
  echo
  echo -e "${YELLOW}⚠  IMPORTANT: Back up ${WALLET_FILE} — it contains your private key.${RESET}"
  echo
  read -rp "Press ENTER to continue..."
fi

# ── Read wallet address ───────────────────────────────────────────────────────
if [[ -z "$WALLET_ADDR" ]]; then
  WALLET_ADDR=$(python3 -c "import json,sys; d=json.load(open('$WALLET_FILE')); print(d['address'])" 2>/dev/null \
    || grep -o '"address"[[:space:]]*:[[:space:]]*"[^"]*"' "$WALLET_FILE" \
       | head -1 | sed 's/.*"address"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
fi

[[ -z "$WALLET_ADDR" ]] && error "Could not read wallet address from $WALLET_FILE"

# ── Summary ───────────────────────────────────────────────────────────────────
info "Mining wallet : ${GREEN}${WALLET_ADDR}${RESET}"
info "Pool URL      : ${CYAN}${POOL_URL}${RESET}"
info "Data dir      : ${DATA_DIR}"
echo

# ── Create data directory ─────────────────────────────────────────────────────
mkdir -p "$DATA_DIR"

# ── Launch ────────────────────────────────────────────────────────────────────
info "Starting Astram node with pool mining..."
echo "Press Ctrl+C to stop."
echo

exec "$NODE_EXE" \
  --pool      "$POOL_URL"   \
  --wallet    "$WALLET_ADDR" \
  --data-dir  "$DATA_DIR"   \
  --http-bind "127.0.0.1:19533"
