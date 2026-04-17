# Astram

Astram is a lightweight, PoW blockchain focused on fast propagation, clean design, and practical GPU mining.

## Components

| Component | Binary | Description |
|-----------|--------|-------------|
| **Astram-node** | `Astram-node` | Core node — P2P networking, chain validation, HTTP API, public RPC |
| **Astram-miner** | `Astram-miner` | Standalone GPU miner (CUDA) — solo or pool mode |
| **Astram-explorer** | `Astram-explorer` | Block explorer web UI — indexes data from the node |
| **wallet-cli** | `wallet-cli` | Command-line wallet — key management, balance, send |
| **AstramX Wallet** | Chrome extension | Browser extension wallet for dApps |
| **astram-stratum** | `astram-stratum` | Stratum mining pool server |
| **astram-dns** | `astram-dns` | Node discovery and bootstrap registry |

## Consensus

- **PoW rule**: Bitcoin-style numeric target check (`hash_u256 < target_u256`).
- **Hash algorithm**: KawPow-Blake3 — memory-hard (4 GB DAG), ASIC-resistant.
- **Difficulty encoding**: Compact target bits (`nBits`-style `u32`) in block header `difficulty` field.
- **Target block time**: 120 seconds.
- **Difficulty algorithm**: DWG3 (Dark Gravity Wave v3 style).
- **Retarget cadence**: Every block, using the most recent **24 blocks**.
- **Retarget formula**: `new_target = avg_past_target × actual_timespan / target_timespan`.
- **Damping**: 25% of computed move applied per block to reduce oscillation.
- **Stability guards**: `actual_timespan` clamped to `[target_timespan/3, target_timespan×3]`; per-block change limit of 4×.

## Prerequisites (Ubuntu 24.04)

### System packages
```bash
sudo apt update
sudo apt install -y build-essential pkg-config cmake clang libclang-dev git curl ca-certificates libssl-dev python3 nodejs npm
```

### Rust toolchain
```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
source "$HOME/.cargo/env"
rustup default stable
rustup update
```

### NVIDIA drivers and CUDA toolkit
```bash
sudo apt install -y ubuntu-drivers-common
sudo ubuntu-drivers autoinstall
sudo apt install -y nvidia-cuda-toolkit
sudo reboot
```

### Verify installation
```bash
rustc --version && cargo --version && nvcc --version && node -v && npm -v
```

## Quick Start

### Build

Linux/macOS:
```bash
./build-release.sh
```

Windows:
```powershell
./build-release.ps1
```

### Run

```bash
# Node (always required first)
./release/linux/Astram.sh node

# Miner (solo mode — mines directly to node)
./release/linux/Astram.sh miner

# Block explorer
./release/linux/Astram.sh explorer

# CLI wallet
./release/linux/Astram.sh wallet
```

Windows:
```powershell
./release/windows/Astram.ps1 node
./release/windows/Astram.ps1 miner
```

## Ports

| Service | Port | Bind | Description |
|---------|------|------|-------------|
| Node HTTP (local) | `19533` | `127.0.0.1` | Dashboard, full API, wallet/mining endpoints |
| Node Public RPC | `18533` | `0.0.0.0` | Read-only API accessible from outside |
| P2P | `8335` | `0.0.0.0` | Peer-to-peer block/tx propagation |
| Explorer | `8080` | `0.0.0.0` | Block explorer web UI |
| DNS Server | `8053` | `0.0.0.0` | Node discovery registry |
| Stratum Pool | `3333` | `0.0.0.0` | Stratum mining protocol (TCP) |
| Pool Stats | `8081` | `0.0.0.0` | Pool statistics API |
| Miner Dashboard | `8090` | `0.0.0.0` | Miner status and hashrate |

## Configuration

### wallet-cli (`~/.Astram/config.json`)

Created automatically on first run.

- Linux/macOS: `~/.Astram/config.json`
- Windows: `%APPDATA%\Astram\config.json`

```json
{
  "wallet_path": "<home>/.Astram/wallet.json",
  "node_rpc_url": "http://127.0.0.1:19533"
}
```

### Node (`config/nodeSettings.conf`)

Read from `config/nodeSettings.conf` next to the binary or in the working directory.

```ini
# Data directory (blockchain DB and logs)
DATA_DIR=~/.Astram/data

# P2P
P2P_BIND_ADDR=0.0.0.0
P2P_PORT=8335

# Local HTTP API
HTTP_BIND_ADDR=127.0.0.1
HTTP_PORT=19533

# Public read-only RPC (set to 0 to disable)
PUBLIC_RPC_PORT=18533

# DNS discovery server
DNS_SERVER_URL=http://161.33.19.183:8053

# Optional: comma-separated fallback bootstrap peers
# BOOTSTRAP_PEERS=1.2.3.4:8335,5.6.7.8:8335
```

### Miner (`config/minerSettings.conf`)

```ini
# Mining mode: solo (direct node) or pool (Stratum)
MINING_MODE=solo

# Solo mode: node to mine against
NODE_RPC_URL=http://127.0.0.1:19533

# Pool mode settings
POOL_HOST=127.0.0.1
POOL_PORT=3333
WORKER_NAME=worker1

# Miner dashboard port
STATUS_PORT=8090
```

## Log Files

Each service writes logs to a rotating daily file and to stderr simultaneously.

| Service | Log directory | File prefix |
|---------|--------------|-------------|
| Node | `<DATA_DIR>/logs/` | `node_` |
| Miner | `logs/` (working dir) | `miner_` |
| Explorer | `logs/` (working dir) | `explorer_` |

- Files rotate at midnight; the previous file is renamed with a timestamp suffix.
- **Only the last 5 log files are kept**; older files are deleted automatically.
- Set `RUST_LOG=debug` (or any level) to override the default `info` level.

## Network Parameters

### Mainnet (release builds — hardcoded, cannot be overridden)

| Parameter | Value |
|-----------|-------|
| Network ID | `Astram-mainnet` |
| Chain ID | `1` |
| Network Magic | `0xA57A0001` |
| Genesis Hash | `0047bb75cef130263090ec45c9e5b464ab0f56c556821cb3a40d59dbf31e7216` |

### Testnet (debug builds only)

Set `ASTRAM_NETWORK=testnet` or override `ASTRAM_NETWORK_ID`, `ASTRAM_CHAIN_ID`, `ASTRAM_NETWORK_MAGIC`.

| Parameter | Value |
|-----------|-------|
| Network ID | `Astram-testnet` |
| Chain ID | `8888` |
| Network Magic | `0xA57A22B8` |

## Mining

### Solo mining

The miner polls the node's HTTP API, builds a block template, mines via CUDA, and submits the found block directly.

```ini
# config/minerSettings.conf
MINING_MODE=solo
NODE_RPC_URL=http://127.0.0.1:19533
```

### Pool mining (Stratum)

Start the Stratum pool server, then point miners at it.

```ini
# config/minerSettings.conf
MINING_MODE=pool
POOL_HOST=127.0.0.1
POOL_PORT=3333
WORKER_NAME=worker1
```

### Hardware requirements

- **Minimum VRAM**: 4 GB (for the 4 GB KawPow-Blake3 DAG).
- **Supported**: Any NVIDIA GPU with CUDA support and ≥ 4 GB VRAM.
- On first start the miner generates the DAG (~3–5 minutes); subsequent epochs regenerate every 7,500 blocks.

## Token Economics

| Parameter | Value |
|-----------|-------|
| Ticker | ASRM |
| Base unit | 1 ram = 10⁻¹⁸ ASRM |
| Initial block reward | 8 ASRM |
| Halving interval | 210,000 blocks |
| Max supply | 42,000,000 ASRM |
| Base tx fee | 0.0001 ASRM |
| Relay fee | 200 Gwei/byte |
| Default wallet fee | 300 Gwei/byte |

## Dashboard and Explorer

- **Node Dashboard**: `http://127.0.0.1:19533`
- **Miner Dashboard**: `http://localhost:8090`
- **Block Explorer**: `http://localhost:8080`

## DNS Registration Policy

DNS only accepts **publicly reachable nodes**. During registration it validates:

- Public IP (no private or loopback ranges).
- The P2P port is reachable from the DNS server.

If DNS registration fails at startup, the node exits. Ensure your P2P port (default `8335`) is open and forwarded if behind NAT.

## CUDA Requirements

```bash
# Verify CUDA is available
nvcc --version

# Build manually with CUDA
cargo build --release -p Astram-miner --features cuda-miner
```

## FAQ

**The node fails DNS registration. What do I do?**
- Ensure port `8335` (or your configured `P2P_PORT`) is open for inbound TCP connections.
- If behind NAT, forward the port on your router.
- Verify firewall rules.

**Dashboard does not open automatically.**
- Open `http://127.0.0.1:19533` manually in your browser.

**I see a CUDA build error.**
- Confirm `nvcc --version` works.
- Install the NVIDIA driver and CUDA Toolkit, then rebuild.

**Where are my log files?**
- Node logs: `<DATA_DIR>/logs/` (default `~/.Astram/data/logs/` on Linux).
- Miner / Explorer logs: `logs/` directory in the working directory.

**How long is log history kept?**
- The last 5 daily log files per service are kept; older files are removed automatically.

## Roadmap

- DAG disk caching (skip 3–5 min regeneration on restart)
- Difficulty adjustment tuning
- Wallet UX improvements
- Testnet launch
- Formal protocol specification

## Contributing

Contributions are welcome. Please open an issue or a discussion before large changes.

Suggested areas:
- Networking and P2P reliability
- Mining performance (CUDA kernel optimization)
- Explorer indexing and APIs
- Documentation and UX

## License

MIT License
