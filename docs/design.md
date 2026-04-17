# Design

## Design Principles

- Simplicity over cleverness.
- Clear separation of concerns — node, miner, wallet, explorer, and pool are independent binaries.
- Minimal runtime dependencies and easy local testing.
- Reasonable defaults with explicit override paths.
- Fail fast on misconfiguration (e.g., DNS registration failure exits the node).

## Component Separation

Each runtime component is a standalone binary with a distinct responsibility:

| Component | Configuration | Dependency |
|-----------|--------------|------------|
| `Astram-node` | `config/nodeSettings.conf` | None (source of truth) |
| `Astram-miner` | `config/minerSettings.conf` | Astram-node (solo) or astram-stratum (pool) |
| `Astram-explorer` | Hardcoded node URL | Astram-node |
| `wallet-cli` | `~/.Astram/config.json` | Astram-node |
| `astram-stratum` | `config/pool.conf` | Astram-node |
| `astram-dns` | Environment / `nginx.conf` | None (independent) |

This separation means the node can run without the miner, the explorer, or the pool — each can be started independently.

## Configuration UX

- **wallet-cli** manages a user-local `config.json` for wallet path and node RPC URL.
- **Node** reads `config/nodeSettings.conf` at startup for runtime parameters.
- **Miner** reads `config/minerSettings.conf` and supports both solo and pool modes.
- All configuration files are plain text (`key=value`) — no specialized tooling required.
- Environment variable `RUST_LOG` overrides the default log level (`info`) for all services.

## Logging Design

Each service writes structured log output to **both stderr and a rotating daily file** simultaneously:

- stderr: real-time console output (no change from previous behavior).
- File: daily rotation with timestamps; the last **5 files** per service are retained and older files are deleted automatically.

| Service | Log directory |
|---------|--------------|
| Node | `<DATA_DIR>/logs/` |
| Miner | `logs/` (working directory) |
| Explorer | `logs/` (working directory) |

The `RUST_LOG` environment variable is respected — e.g., `RUST_LOG=debug` enables verbose output to both stderr and the log file.

## API Design

- The **local HTTP API** (`127.0.0.1:19533`) is the full API, including wallet queries, mempool relay, and mining submission. It is intentionally bound to localhost.
- The **public RPC** (`0.0.0.0:18533`) exposes a safe read-only subset suitable for dApps and external tooling. Wallet, mining, and relay endpoints are excluded. It can be disabled by setting `PUBLIC_RPC_PORT=0`.
- Both APIs are HTTP REST with JSON responses. No WebSocket or gRPC.
- Transaction serialization uses **bincode v2 standard** + Base64 for binary payloads over HTTP.

## Mining Design

Two mining modes are supported to accommodate solo miners and pool operators:

- **Solo mode**: The miner polls the node directly — no pool infrastructure needed. Simple and low-latency for small-scale operation.
- **Pool mode**: The miner connects via the Stratum protocol. The `astram-stratum` server aggregates miners, handles job distribution, validates shares, and submits found blocks. VarDiff adjusts per-miner difficulty dynamically.

The miner hosts a lightweight status dashboard on port `8090` showing real-time hashrate, mode, and share statistics.

## Explorer UX

- The explorer is separate from the node process — the node remains the single source of truth.
- The explorer syncs incrementally from the node every 10 seconds and maintains its own RocksDB index.
- If the explorer DB is out of date (missing UTXO data from an older version), it automatically resets and performs a full re-sync.

## Extensibility

- Feature flags enable optional CUDA mining (`cuda-miner` feature).
- Modular workspace crates allow independent development without breaking end-user tooling.
- The public RPC port can be disabled or firewalled without affecting the full local API.
- Network parameters (chain ID, magic) are overridable in debug builds for testnet and custom network development.

## Known Gaps

- No formal protocol specification document.
- Automated configuration validation is minimal.
- Wallet file encryption and hardware wallet support are not implemented.
- DAG disk caching is not yet implemented — DAG is regenerated every restart within an epoch.
- The DNS service is centralized; no decentralized peer discovery fallback exists.
