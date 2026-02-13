#!/bin/bash
# Astram Release Build Script for Linux/macOS
# This script builds all components and packages them for distribution

set -e

# Colors
INFO='\033[0;36m'
SUCCESS='\033[0;32m'
ERROR='\033[0;31m'
NC='\033[0m'

echo -e "${INFO}INFO  Astram Release Builder${NC}"
echo ""

# Detect platform
case "$(uname -s)" in
    Linux*)     PLATFORM="linux";;
    Darwin*)    PLATFORM="macos";;
    *)          echo -e "${ERROR}ERROR Unsupported platform$(NC)"; exit 1;;
esac

echo -e "${INFO}INFO  Detected platform: $PLATFORM${NC}"

# Select build backend
select_backend() {
    echo -e "${INFO}INFO  Select build backend:${NC}"
    echo "  1) CPU"
    echo "  2) GPU (CUDA)"
    read -r -p "Choose [1-2] (default: 1): " choice
    case "$choice" in
        2|gpu|GPU)
            BUILD_BACKEND="cuda"
            ;;
        *)
            BUILD_BACKEND="cpu"
            ;;
    esac
    echo -e "${INFO}INFO  Build backend: $BUILD_BACKEND${NC}"
}

select_backend

NODE_BUILD_FLAGS="--no-default-features"
EXPLORER_BUILD_FLAGS="--no-default-features"
if [ "$BUILD_BACKEND" = "cuda" ]; then
    NODE_BUILD_FLAGS="--features cuda-miner"
    EXPLORER_BUILD_FLAGS="--features cuda-miner"
    export MINER_BACKEND="cuda"
else
    export MINER_BACKEND="cpu"
fi

# Clean previous release
RELEASE_DIR="release/$PLATFORM"
if [ -d "$RELEASE_DIR" ]; then
    echo -e "${INFO}INFO  Cleaning previous release...${NC}"
    rm -rf "$RELEASE_DIR"
fi

# Create release directory
echo -e "${INFO}INFO  Creating release directory...${NC}"
mkdir -p "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR/config"

# Build all components in release mode
echo -e "${INFO}INFO  Building all components in release mode...${NC}"
cargo build --release --workspace --exclude Astram-node --exclude Astram-explorer
cargo build --release -p Astram-node $NODE_BUILD_FLAGS
cargo build --release -p Astram-explorer $EXPLORER_BUILD_FLAGS

echo -e "${SUCCESS}OK    Build completed successfully!${NC}"

# Copy executables
echo -e "${INFO}INFO  Copying executables...${NC}"
EXECUTABLES=(
    "Astram-node"
    "Astram-dns"
    "Astram-explorer"
    "wallet-cli"
)

for exe in "${EXECUTABLES[@]}"; do
    source="target/release/$exe"
    if [ -f "$source" ]; then
        cp "$source" "$RELEASE_DIR/$exe"
        chmod +x "$RELEASE_DIR/$exe"
        echo -e "${SUCCESS}OK    Copied $exe${NC}"
    else
        echo -e "${ERROR}ERROR Missing: $exe${NC}"
    fi
done

# Create launcher script
echo -e "${INFO}INFO  Creating launcher script...${NC}"
cat > "$RELEASE_DIR/Astram.sh" << 'LAUNCHER_EOF'
#!/bin/bash
# Astram Launcher for Linux/macOS
# Usage: ./Astram.sh [node|dns|explorer|wallet] [args...]

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
COMPONENT="${1:-node}"
shift || true

DEFAULT_CONFIG_DIR="$HOME/.Astram"
DEFAULT_CONFIG_FILE="$DEFAULT_CONFIG_DIR/config.json"
DEFAULT_WALLET_PATH="$DEFAULT_CONFIG_DIR/wallet.json"

ensure_config_defaults() {
    if [ ! -f "$DEFAULT_CONFIG_FILE" ]; then
        mkdir -p "$DEFAULT_CONFIG_DIR"
        cat > "$DEFAULT_CONFIG_FILE" << EOF
{
  "wallet_path": "$DEFAULT_WALLET_PATH",
    "node_rpc_url": "http://127.0.0.1:19533"
}
EOF
    fi

    if command -v python3 >/dev/null 2>&1; then
        local values
                values=$(python3 - "$DEFAULT_CONFIG_FILE" "$DEFAULT_WALLET_PATH" << 'PY'
import json
import os
import sys

config_path, default_wallet = sys.argv[1:3]

try:
    with open(config_path, "r", encoding="utf-8") as f:
        data = json.load(f)
except Exception:
    data = {}

if not isinstance(data, dict):
    data = {}

changed = False
for key, default in (
    ("wallet_path", default_wallet),
    ("node_rpc_url", "http://127.0.0.1:19533"),
):
    val = data.get(key)
    if not isinstance(val, str) or not val.strip() or val.strip().startswith("~"):
        data[key] = default
        changed = True

wallet_path = os.path.expanduser(data.get("wallet_path", default_wallet))

if not os.path.exists(wallet_path):
    data["wallet_path"] = default_wallet
    wallet_path = os.path.expanduser(default_wallet)
    changed = True

if changed:
    os.makedirs(os.path.dirname(config_path), exist_ok=True)
    with open(config_path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)

print(wallet_path)
PY
        )
        WALLET_PATH=$(printf "%s\n" "$values" | sed -n '1p')
    else
        WALLET_PATH="$DEFAULT_WALLET_PATH"
    fi

    mkdir -p "$(dirname "$WALLET_PATH")"
}

ensure_config_defaults

case "$COMPONENT" in
    node)
        EXE="Astram-node"
        ;;
    dns)
        EXE="Astram-dns"
        ;;
    explorer)
        EXE="Astram-explorer"
        ;;
    wallet)
        EXE="wallet-cli"
        ;;
    *)
        echo "Usage: $0 [node|dns|explorer|wallet] [args...]"
        exit 1
        ;;
esac

EXE_PATH="$SCRIPT_DIR/$EXE"

if [ ! -f "$EXE_PATH" ]; then
    echo "Error: $EXE not found"
    exit 1
fi

if [ "$COMPONENT" = "node" ] && [ ! -f "$WALLET_PATH" ]; then
    echo "Wallet file not found. Creating a new wallet at $WALLET_PATH"
    "$SCRIPT_DIR/wallet-cli" generate
fi

open_dashboard() {
    local url="http://localhost:19533"
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$url" >/dev/null 2>&1 &
    elif command -v open >/dev/null 2>&1; then
        open "$url" >/dev/null 2>&1 &
    else
        echo "Open in your browser: $url"
    fi
}

echo "Starting Astram $COMPONENT..."
if [ "$COMPONENT" = "node" ]; then
    "$EXE_PATH" "$@" &
    NODE_PID=$!
    sleep 10
    open_dashboard
    wait "$NODE_PID"
else
    "$EXE_PATH" "$@"
fi
LAUNCHER_EOF

chmod +x "$RELEASE_DIR/Astram.sh"

# Create node settings config
echo -e "${INFO}INFO  Creating node settings configuration...${NC}"
cat > "$RELEASE_DIR/config/nodeSettings.conf" << 'CONFIG_EOF'
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

# Data directory
DATA_DIR=~/.Astram/data
CONFIG_EOF

# Create README
echo -e "${INFO}INFO  Creating README...${NC}"
cat > "$RELEASE_DIR/README.md" << README_EOF
# Astram for $PLATFORM

## Quick Start

1. Extract this archive to a folder
2. Open a terminal in this directory
3. Run a component:

\`\`\`bash
# Run blockchain node
./Astram.sh node

# Run DNS server
./Astram.sh dns

# Run blockchain explorer
./Astram.sh explorer

# Run wallet CLI
./Astram.sh wallet
\`\`\`

## Components

- **Astram-node** - Main blockchain node (HTTP: 19533, P2P: 8335)
- **Astram-dns** - DNS discovery server (Port: 8053)
- **Astram-explorer** - Web-based blockchain explorer (Port: 3000)
- **wallet-cli** - Command-line wallet interface

## System Requirements

- $PLATFORM (64-bit)
- 4GB RAM minimum
- 10GB free disk space

## Data Directory

Astram stores blockchain data in: \`~/.Astram\`

To reset the blockchain, delete this directory while no nodes are running.

## Support

For issues and documentation, visit: https://github.com/YSBlueA/AstramChain
README_EOF

# Create version info
VERSION=$(grep '^version =' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
cat > "$RELEASE_DIR/VERSION.txt" << VERSION_EOF
Astram v$VERSION
Built: $(date '+%Y-%m-%d %H:%M:%S')
Platform: $PLATFORM x64
VERSION_EOF

echo -e "${SUCCESS}OK    Release package created successfully!${NC}"
echo ""
echo -e "${INFO}INFO  Release directory: $RELEASE_DIR${NC}"
echo -e "${INFO}INFO  To distribute: compress the folder and share the archive${NC}"
echo ""
echo -e "${INFO}INFO  Next steps:${NC}"
echo "  1. Test the executables in $RELEASE_DIR/"
echo "  2. Create a tarball: tar -czf Astram-$PLATFORM-v$VERSION.tar.gz -C release $PLATFORM"
echo "  3. Share the tarball with users"

