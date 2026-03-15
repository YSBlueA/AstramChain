#!/bin/bash
set -e

INFO='\033[0;36m'
SUCCESS='\033[0;32m'
ERROR='\033[0;31m'
NC='\033[0m'

echo -e "${INFO}INFO  Astram Release Builder${NC}"
echo ""

case "$(uname -s)" in
    Linux*) PLATFORM="linux";;
    Darwin*) PLATFORM="macos";;
    *) echo -e "${ERROR}Unsupported platform${NC}"; exit 1;;
esac

echo -e "${INFO}INFO  Detected platform: $PLATFORM${NC}"

# ---------------------------
# GPU backend
# ---------------------------

echo -e "${INFO}INFO  Build backend: GPU (CUDA)${NC}"
export MINER_BACKEND="cuda"

NODE_BUILD_FLAGS=""
EXPLORER_BUILD_FLAGS=""

RELEASE_DIR="release/$PLATFORM"

if [ -d "$RELEASE_DIR" ]; then
    echo -e "${INFO}Cleaning previous release${NC}"
    rm -rf "$RELEASE_DIR"
fi

mkdir -p "$RELEASE_DIR/config"

# ---------------------------
# BUILD
# ---------------------------

echo -e "${INFO}Building components...${NC}"

cargo build --release --workspace \
    --exclude Astram-node \
    --exclude Astram-explorer \
    --exclude Astram-miner

cargo build --release -p Astram-node
cargo build --release -p Astram-miner --features cuda-miner
cargo build --release -p Astram-explorer

echo -e "${SUCCESS}Build completed${NC}"

# ---------------------------
# Build explorer web
# ---------------------------

echo -e "${INFO}Building explorer web...${NC}"

if ! command -v npm >/dev/null; then
    echo -e "${ERROR}npm required${NC}"
    exit 1
fi

pushd explorer/web >/dev/null

if [ -f package-lock.json ]; then
    npm ci
else
    npm install
fi

npm run build

popd >/dev/null

mkdir -p "$RELEASE_DIR/explorer_web"
cp -r explorer/web/dist/* "$RELEASE_DIR/explorer_web/"

cat > "$RELEASE_DIR/explorer_web/explorer.conf.js" <<'EOF'
window.ASTRAM_EXPLORER_CONF = {
  apiBaseUrl: "https://explorer.astramchain.com/api"
};
EOF
echo -e "${SUCCESS}Created explorer_web/explorer.conf.js${NC}"

# ---------------------------
# Copy pool web
# ---------------------------

echo -e "${INFO}Copying pool web...${NC}"

POOL_WEB_DIR="astram-stratum/web"

if [ -d "$POOL_WEB_DIR/public" ]; then
    # public/ → pool_web/  (landing page)
    mkdir -p "$RELEASE_DIR/pool_web"
    cp -r "$POOL_WEB_DIR/public/." "$RELEASE_DIR/pool_web/"
    # root index.html → pool_web/dashboard/index.html  (stats dashboard)
    if [ -f "$POOL_WEB_DIR/index.html" ]; then
        mkdir -p "$RELEASE_DIR/pool_web/dashboard"
        cp "$POOL_WEB_DIR/index.html" "$RELEASE_DIR/pool_web/dashboard/index.html"
    fi
    echo -e "${SUCCESS}Deployed pool web to $RELEASE_DIR/pool_web${NC}"
elif [ -f "$POOL_WEB_DIR/index.html" ]; then
    mkdir -p "$RELEASE_DIR/pool_web"
    cp -r "$POOL_WEB_DIR/." "$RELEASE_DIR/pool_web/"
    echo -e "${SUCCESS}Deployed pool web to $RELEASE_DIR/pool_web${NC}"
else
    echo -e "\033[0;33mWARN  Pool web not found at $POOL_WEB_DIR (skipping)\033[0m"
fi

# ---------------------------
# Copy executables
# ---------------------------

echo -e "${INFO}Copying executables${NC}"

EXECUTABLES=(
"Astram-node"
"Astram-miner"
"Astram-stratum"
"Astram-dns"
"Astram-explorer"
"wallet-cli"
)

for exe in "${EXECUTABLES[@]}"; do

    SRC="target/release/$exe"

    if [ -f "$SRC" ]; then
        cp "$SRC" "$RELEASE_DIR/"
        chmod +x "$RELEASE_DIR/$exe"
        echo -e "${SUCCESS}Copied $exe${NC}"
    else
        echo -e "${ERROR}Missing $exe${NC}"
    fi

done

# ---------------------------
# Pool scripts
# ---------------------------

echo -e "${INFO}Copying pool scripts${NC}"

POOL_DIR="astram-stratum/scripts/linux"

if [ -f "$POOL_DIR/start-mining-pool.sh" ]; then
    cp "$POOL_DIR/start-mining-pool.sh" "$RELEASE_DIR/"
    chmod +x "$RELEASE_DIR/start-mining-pool.sh"
fi

# ---------------------------
# Launcher script (Astram.sh)
# ---------------------------

echo -e "${INFO}Creating launcher script...${NC}"

cat > "$RELEASE_DIR/Astram.sh" <<'LAUNCHER'
#!/usr/bin/env bash
# Astram Launcher for Linux/macOS
# Usage: ./Astram.sh [node|miner|stratum|dns|explorer|wallet] [args...]

set -e

COMPONENT="${1:-node}"
shift || true

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_BASE="${HOME}/.Astram"
DEFAULT_CONFIG_FILE="${DEFAULT_BASE}/config.json"
DEFAULT_WALLET_PATH="${DEFAULT_BASE}/wallet.json"

ensure_config_defaults() {
    mkdir -p "$(dirname "$DEFAULT_CONFIG_FILE")"
    if [ ! -f "$DEFAULT_CONFIG_FILE" ]; then
        cat > "$DEFAULT_CONFIG_FILE" <<JSON
{
  "wallet_path": "${DEFAULT_WALLET_PATH}",
  "node_rpc_url": "http://127.0.0.1:19533"
}
JSON
    fi
}

load_conf_file() {
    local path="$1"
    if [ -f "$path" ]; then
        while IFS='=' read -r key value; do
            key="${key%%#*}"
            key="${key// /}"
            value="${value%%#*}"
            value="${value# }"
            value="${value% }"
            # Expand environment variables in the value (e.g. ${HOME})
            value=$(eval "echo \"$value\"" 2>/dev/null || echo "$value")
            if [ -n "$key" ]; then
                export "$key=$value"
            fi
        done < <(grep -v '^[[:space:]]*#' "$path" | grep '=')
    fi
}

ensure_config_defaults

# Load build configuration (MINER_BACKEND)
BUILD_INFO_FILE="${SCRIPT_DIR}/BUILD_INFO.conf"
if [ -f "$BUILD_INFO_FILE" ]; then
    while IFS='=' read -r key value; do
        if [ "$key" = "MINER_BACKEND" ]; then
            export MINER_BACKEND="${value:-cuda}"
        fi
    done < <(grep -v '^[[:space:]]*#' "$BUILD_INFO_FILE" | grep '=')
fi

MINER_BACKEND="${MINER_BACKEND:-cuda}"

case "$COMPONENT" in
    node)     EXE="Astram-node" ;;
    miner)    EXE="Astram-miner" ;;
    stratum)  EXE="Astram-stratum" ;;
    dns)      EXE="Astram-dns" ;;
    explorer) EXE="Astram-explorer" ;;
    wallet)   EXE="wallet-cli" ;;
    *)
        echo "Usage: $0 [node|miner|stratum|dns|explorer|wallet] [args...]"
        exit 1
        ;;
esac

EXE_PATH="${SCRIPT_DIR}/${EXE}"

if [ ! -f "$EXE_PATH" ]; then
    echo "Error: $EXE not found at $EXE_PATH" >&2
    exit 1
fi

# Wallet auto-create: needed for node, miner, stratum
if [[ "$COMPONENT" =~ ^(node|miner|stratum)$ ]] && [ ! -f "$DEFAULT_WALLET_PATH" ]; then
    echo "Wallet file not found. Creating a new wallet at $DEFAULT_WALLET_PATH"
    "${SCRIPT_DIR}/wallet-cli" generate
fi

# Miner: show mode from config file before starting
if [ "$COMPONENT" = "miner" ]; then
    MINER_CONF="${SCRIPT_DIR}/config/minerSettings.conf"
    MINING_MODE="pool"
    if [ -f "$MINER_CONF" ]; then
        mode_line=$(grep -m1 '^MINING_MODE' "$MINER_CONF" || true)
        if [ -n "$mode_line" ]; then
            MINING_MODE="${mode_line#*=}"
            MINING_MODE="${MINING_MODE// /}"
        fi
    fi
    echo ""
    echo "  Mining mode : $MINING_MODE"
    if [ "$MINING_MODE" = "solo" ]; then
        echo "  Requires    : Astram-node running on this machine"
        echo "  Edit config/minerSettings.conf to switch to pool mode"
    else
        echo "  Pool        : pool.astramchain.com:3333"
        echo "  Edit config/minerSettings.conf to switch to solo mode"
    fi
    echo ""
fi

# Stratum: load poolSettings.conf and inject as environment variables
if [ "$COMPONENT" = "stratum" ]; then
    POOL_CONF="${SCRIPT_DIR}/config/poolSettings.conf"
    load_conf_file "$POOL_CONF"

    P_NODE_RPC="${NODE_RPC_URL:-http://127.0.0.1:19533}"
    P_STRATUM="${STRATUM_BIND:-0.0.0.0:3333}"
    P_STATS="${STATS_BIND:-0.0.0.0:8081}"

    echo ""
    echo "  Stratum pool server"
    echo "  Node RPC    : $P_NODE_RPC"
    echo "  Stratum     : $P_STRATUM"
    echo "  Stats API   : $P_STATS"
    echo "  Requires    : Astram-node running on this machine"
    echo "  Edit config/poolSettings.conf to change pool settings"
    echo ""
fi

echo "Starting Astram ${COMPONENT}..."
exec "$EXE_PATH" "$@"
LAUNCHER

chmod +x "$RELEASE_DIR/Astram.sh"

# ---------------------------
# Config files
# ---------------------------

echo -e "${INFO}Creating node settings configuration...${NC}"

cat > "$RELEASE_DIR/config/nodeSettings.conf" <<'CONF'
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
DATA_DIR=${HOME}/.Astram/data
CONF

echo -e "${INFO}Creating miner settings configuration...${NC}"

cat > "$RELEASE_DIR/config/minerSettings.conf" <<'CONF'
# Astram Miner Settings
# This file is read by Astram-miner at startup.
# Location: config/minerSettings.conf  (next to the miner binary)

# ----- Mining Mode -----------------------------------------------------------------------
# pool : Connect to a Stratum mining pool.
#        Rewards are split among pool participants, providing steady payouts.
#        Recommended for most users — no need to run a full node.
#
# solo : Mine directly against your own node (Astram-node must be running).
#        100% of the block reward goes to your wallet when you find a block.
#        Best suited for high-hashrate miners or testing.
#        Requires: Astram-node running on the same machine.
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
CONF

echo -e "${INFO}Creating pool settings configuration...${NC}"

cat > "$RELEASE_DIR/config/poolSettings.conf" <<'CONF'
# Astram Stratum Pool Settings
# This file is read by Astram-stratum via the launcher (Astram.sh stratum).
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
CONF

echo -e "${SUCCESS}Created config/poolSettings.conf${NC}"

# ---------------------------
# README
# ---------------------------

echo -e "${INFO}Creating README...${NC}"

cat > "$RELEASE_DIR/README.md" <<'README'
# Astram for Linux/macOS

## Option A — Join the Mining Pool (Recommended)

```bash
chmod +x start-mining-pool.sh
./start-mining-pool.sh
```

The script will:
1. Detect your NVIDIA GPU
2. Create a wallet automatically if you don't have one
3. Connect to the pool at `pool.astramchain.com:3333`

Pool dashboard: https://pool.astramchain.com

## Option B — Run Your Own Node + Miner

Open a terminal in this directory and run each component in a separate window:

```bash
# 1. Start the blockchain node (syncs with the network)
./Astram.sh node

# 2. Start the miner (after the node has synced)
./Astram.sh miner

# Other components
./Astram.sh stratum    # Run your own mining pool
./Astram.sh dns        # DNS discovery server
./Astram.sh explorer   # Blockchain explorer
./Astram.sh wallet     # Wallet CLI
```

### Miner Mode

Edit `config/minerSettings.conf` to choose your mining mode:

- **pool** (default) — Connect to `pool.astramchain.com:3333`. No local node required.
- **solo** — Mine directly against your local node. Full block reward goes to your wallet.
  Requires `Astram-node` running on the same machine.

### Running Your Own Pool (Stratum)

Edit `config/poolSettings.conf`, then:

```bash
# Terminal 1: start the node
./Astram.sh node

# Terminal 2: start the pool server
./Astram.sh stratum
```

Miners connect to `<your-ip>:3333` using standard Stratum protocol.
Pool stats are available at `http://localhost:8081`.

## Components

- **start-mining-pool.sh** - One-click pool mining launcher
- **Astram-node** - Main blockchain node (HTTP: 19533, P2P: 8335)
- **Astram-miner** - GPU miner (pool or solo mode, NVIDIA CUDA required)
- **Astram-stratum** - Stratum mining pool server (Stratum: 3333, Stats: 8081)
- **Astram-dns** - DNS discovery server (Port: 8053)
- **Astram-explorer** - Web-based blockchain explorer (Port: 3000)
- **wallet-cli** - Command-line wallet interface

## System Requirements

- Linux or macOS (64-bit)
- 4GB RAM minimum
- 10GB free disk space
- NVIDIA GPU (4GB+ VRAM recommended)
- NVIDIA driver + CUDA Toolkit installed (`nvcc` available)

## Mining Backend

- This release is **GPU-only**.
- Node mining backend is fixed to CUDA (`MINER_BACKEND=cuda`).

## Data Directory

Astram stores blockchain data in: `~/.Astram`

To reset the blockchain, delete this directory while no nodes are running.

## Network Selection

Edit `config/nodeSettings.conf` to choose a network:

- Mainnet: Network ID Astram-mainnet, Chain ID 1
- Testnet: Network ID Astram-testnet, Chain ID 8888
- Mainnet Network Magic: 0xA57A0001
- Testnet Network Magic: 0xA57A22B8

## Support

- GitHub: https://github.com/YSBlueA/AstramChain
- Pool: https://pool.astramchain.com
README

# ---------------------------
# Build info (same as ps1)
# ---------------------------

cat > "$RELEASE_DIR/BUILD_INFO.conf" <<EOF
MINER_BACKEND=cuda
EOF

# ---------------------------
# Version
# ---------------------------

VERSION=$(grep '^version' node/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

cat > "$RELEASE_DIR/VERSION.txt" <<EOF
Astram v$VERSION
Built: $(date "+%Y-%m-%d %H:%M:%S")
Platform: $PLATFORM x64
Miner Backend: cuda
EOF

echo -e "${SUCCESS}Release package created successfully${NC}"
echo ""
echo -e "${INFO}Release directory: $RELEASE_DIR${NC}"
echo -e "${INFO}To distribute: tar -czf Astram-${PLATFORM}-v${VERSION}.tar.gz -C release ${PLATFORM}${NC}"
echo ""
echo -e "${INFO}Next steps:${NC}"
echo "  1. Test the executables in $RELEASE_DIR/"
echo "  2. Create an archive: tar -czf Astram-${PLATFORM}-v${VERSION}.tar.gz -C release ${PLATFORM}"
echo "  3. Share the archive with users"