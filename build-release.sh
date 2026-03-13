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