# Astram

Astram is a lightweight, PoW blockchain focused on fast propagation, clean design, and practical mining on CPU or GPU.

## Components

- **Astram-node**: Core node, P2P, mining, and HTTP API.
- **Astram-dns**: Node discovery service (public node registry).
- **Astram-explorer**: Local chain explorer (indexes from the node).
- **wallet-cli**: Command-line wallet and config tool.

## Quick Start

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

The release scripts prompt for **CPU vs GPU** and create a runnable package.

### Build from source

CPU build:

```bash
cargo build --release --workspace --exclude Astram-node --exclude Astram-explorer
cargo build --release -p Astram-node --no-default-features
cargo build --release -p Astram-explorer --no-default-features
```

GPU build (CUDA):

```bash
cargo build --release --workspace --exclude Astram-node --exclude Astram-explorer
cargo build --release -p Astram-node --features cuda-miner
cargo build --release -p Astram-explorer --features cuda-miner
```

Run:

```bash
./target/release/Astram-node
./target/release/Astram-dns
./target/release/Astram-explorer
./target/release/wallet-cli
```

## Ports

- **Node HTTP + Dashboard**: `http://127.0.0.1:19533`
- **P2P**: `8335` (env: `NODE_PORT`)
- **Explorer**: `http://127.0.0.1:8080`
- **DNS Server**: `8053`

## Configuration

On first run, the node creates a config file at:

- Linux/macOS: `~/.Astram/config.json`
- Windows: `%APPDATA%\Astram\config.json`

Default values:

```json
{
  "wallet_path": "<home>/.Astram/wallet.json",
  "node_rpc_url": "http://127.0.0.1:19533",
  "data_dir": "<home>/.Astram/data"
}
```

Useful env vars:

- `DNS_SERVER_URL`: DNS server base URL.
- `NODE_PORT`: P2P port advertised to DNS.

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
- Mining performance (CPU/CUDA)
- Explorer indexing and APIs
- Documentation and UX

## CUDA Requirements

GPU mining requires NVIDIA CUDA and a compatible GPU. CUDA builds use the `cuda-miner` feature.

- Install the NVIDIA driver and CUDA Toolkit
- Ensure `nvcc` is on your PATH
- Build with: `cargo build --release -p Astram-node --features cuda-miner`

If you do not have CUDA installed, use CPU builds (`--no-default-features`).

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
- Use a CPU build if CUDA is not installed.

## License

MIT License
