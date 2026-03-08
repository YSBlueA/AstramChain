# Astram

Astram is a lightweight, PoW blockchain focused on fast propagation, clean design, and practical GPU mining.

## Components

- **Astram-node**: Core node, P2P, mining, and HTTP API.
- **Astram-dns**: Node discovery service (public node registry).
- **Astram-explorer**: Local chain explorer (indexes from the node).
- **wallet-cli**: Command-line wallet and config tool.

## Consensus (Current)

- **PoW rule**: Bitcoin-style numeric target check (`hash_u256 < target_u256`).
- **Difficulty encoding**: Compact target bits (`nBits`-style `u32`) stored in block header `difficulty`.
- **Target block time**: `120` seconds (about 2 minutes).
- **Difficulty algorithm**: **DWG3** (Dark Gravity Wave v3 style).
- **Retarget cadence**: Every block, using the most recent 24 blocks.
- **Retarget formula**: `new_target = avg_past_target * actual_timespan / target_timespan`.
- **Stability guards**: `actual_timespan` is clamped to `[target_timespan/3, target_timespan*3]`.

## Quick Start

The release scripts build and package the GPU (CUDA) miner runtime.

## Prerequisites (Ubuntu 24.04)

For a fresh Ubuntu 24.04 installation, install dependencies and build tools:

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
```

After installing CUDA, reboot to ensure drivers are loaded:
```bash
sudo reboot
```

### Verify installation
```bash
rustc --version
cargo --version
nvcc --version
node -v
npm -v
```


### Release builds (recommended)

Linux/macOS:

```bash
./build-release.sh
./release/linux/Astram.sh node
```

Windows:

```powershell
./build-release.ps1
./release/windows/Astram.ps1 node
```

## Ports

- **Node HTTP + Dashboard**: `http://127.0.0.1:19533`
- **P2P**: `8335` (env: `NODE_PORT`)
- **Explorer**: `http://127.0.0.1:8080`
- **DNS Server**: `8053`

## Configuration

wallet-cli config (created on first run):

- Linux/macOS: `~/.Astram/config.json`
- Windows: `%APPDATA%\Astram\config.json`

Default values:

```json
{
  "wallet_path": "<home>/.Astram/wallet.json",
  "node_rpc_url": "http://127.0.0.1:19533"
}
```

Node settings are read from `config/nodeSettings.conf` in the release package or working directory.

Network parameters:

- **Release builds** use hardcoded mainnet values for security (cannot be overridden).
  - Network ID: `Astram-mainnet`
  - Chain ID: `1`
  - Network Magic: `0xA57A0001`
  - Genesis Hash: `0047bb75cef130263090ec45c9e5b464ab0f56c556821cb3a40d59dbf31e7216`
- **Debug builds** allow testnet and custom network overrides via environment variables:
  - Set `ASTRAM_NETWORK=testnet` for testnet (Chain ID `8888`, Magic `0xA57A22B8`)
  - Optional overrides: `ASTRAM_NETWORK_ID`, `ASTRAM_CHAIN_ID`, `ASTRAM_NETWORK_MAGIC`

## Dashboard and Explorer

- **Node Dashboard**: `http://127.0.0.1:19533`
- **Explorer**: `http://127.0.0.1:8080`

The launcher opens the dashboard a few seconds after starting the node.

## DNS Registration Policy

DNS only accepts **publicly reachable nodes**. During registration it validates:

- Public IP (no private or loopback ranges)
- Port is reachable from the DNS server

If DNS registration fails, the node exits to avoid running an unreachable instance.

## Roadmap

- Mining algorithm improvements
- Difficulty adjustment tuning
- Wallet UX (CLI and GUI)
- Testnet launch
- Public docs and explorer improvements

## Contributing

Contributions are welcome. Please open an issue or a discussion before large changes.

Suggested areas:

- Networking and P2P reliability
- Mining performance (CUDA)
- Explorer indexing and APIs
- Documentation and UX

## CUDA Requirements

GPU mining requires NVIDIA CUDA and a compatible GPU. CUDA builds use the `cuda-miner` feature.

- Install the NVIDIA driver and CUDA Toolkit
- Ensure `nvcc` is on your PATH
- Build with: `cargo build --release -p Astram-node --features cuda-miner`

## FAQ

**The node fails DNS registration. What do I do?**

- Ensure your node is reachable from the public internet on the P2P port (`NODE_PORT`, default `8335`).
- If you are behind NAT, forward the port on your router or run on a public server.
- Verify firewalls allow inbound TCP to the P2P port.

**Dashboard does not open automatically.**

- Open `http://127.0.0.1:19533` manually in your browser.
- Check that the node process is running and the port is not in use.

**I see a CUDA build error.**

- Confirm `nvcc --version` works.
- Install a compatible NVIDIA driver + CUDA Toolkit and rebuild.

## License

MIT License
