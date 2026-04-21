# Architecture

## Overview

Astram is composed of seven runtime components:

| Component | Binary | Role |
|-----------|--------|------|
| **Astram-node** | `Astram-node` | Core node — P2P, chain DB, HTTP API |
| **Astram-miner** | `Astram-miner` | Standalone CUDA GPU miner |
| **Astram-explorer** | `Astram-explorer` | Block explorer web UI |
| **wallet-cli** | `wallet-cli` | CLI wallet and transaction tool |
| **AstramX Wallet** | Chrome extension | Browser extension wallet for dApps |
| **astram-stratum** | `astram-stratum` | Stratum mining pool server |
| **astram-dns** | `astram-dns` | Node discovery and bootstrap registry |

## Component Responsibilities

### Astram-node

- Maintains the blockchain database (RocksDB) and UTXO set.
- Validates PoW using compact target bits with numeric comparison (`hash_u256 < target_u256`).
- Retargets mining difficulty every block using DWG3 over a **24-block** window.
- Runs the P2P stack for block and transaction propagation.
- Exposes a **local HTTP API** (`127.0.0.1:19533`) — full API including wallet and mining endpoints.
- Exposes a **public read-only RPC** (`0.0.0.0:18533`) — safe read endpoints only; no wallet, mining, or relay.
- Registers with the DNS discovery server at startup and re-registers every 5 minutes.

### Astram-miner

- Standalone CUDA GPU miner — runs independently of the node process.
- **Solo mode**: polls the node HTTP API (`/status`, `/mempool`), builds a block template, mines via CUDA, and submits via `POST /mining/submit`.
- **Pool mode**: connects to an `astram-stratum` server over Stratum TCP, receives jobs, finds nonces, and submits shares.
- Hosts a status dashboard on port `8090` showing hashrate, mode, accepted/rejected shares, and blocks found.

### Astram-explorer

- Reads chain data from the node API every 10 seconds (incremental sync).
- Indexes blocks, transactions, UTXOs, and address balances in a local RocksDB.
- Serves a web UI on port `8080` for browsing blocks, transactions, and addresses.

### wallet-cli

- Generates wallets (Ed25519 + BIP39 24-word mnemonic) and manages keys.
- Queries balances and UTXOs from the node.
- Builds, signs, and submits transactions.
- Stores wallet config in a user-local JSON file.

### AstramX Wallet (Chrome Extension)

- Browser extension that injects `window.astramWallet` into web pages.
- Exposes `getBalance()` and `signTransaction()` to dApps.
- Displays a side-panel approval UI for user-confirmed transactions.
- Communicates with the node via the Public RPC.

### astram-stratum

- Implements the Stratum mining pool protocol (TCP, port `3333`).
- Fetches block templates from the node, distributes jobs to miners.
- Validates shares (pool difficulty) and submits found blocks to the node.
- Supports Variable Difficulty (VarDiff) targeting ~15 s share time.
- Uses PPLNS (Pay Per Last N Shares) for reward distribution.
- Exposes pool statistics on port `8081`.

### astram-dns

- Accepts node registrations via `POST /register`.
- Validates that the registering node's IP is public and the P2P port is reachable.
- Returns a ranked list of active nodes via `GET /nodes`.
- Removes stale nodes that have not re-registered within 1 hour.
- Runs on port `8053`.

## Data Flow

```
wallet-cli ──POST /tx──────────────────────────────┐
AstramX Wallet ──POST /tx──────────────────────────┤
                                                    ▼
                                          Astram-node (19533)
                                          ├── P2P layer (18335) ◄──► Peers
                                          ├── RocksDB (chain + UTXOs)
                                          └── Public RPC (18533) ◄── Explorer, dApps

Astram-miner ──GET /status, /mempool──► Astram-node (solo)
Astram-miner ──Stratum TCP (3333)────► astram-stratum ──POST /mining/submit──► Astram-node

Astram-explorer ──GET /blockchain/range──► Astram-node ──indexes──► Explorer RocksDB
                                                         └──► Web UI (8080)

astram-dns ──register/list──────────────────────────────── Node bootstrap
```

## Port Mapping

| Service | Port | Bind | Notes |
|---------|------|------|-------|
| Node HTTP (local) | `19533` | `127.0.0.1` | Full API — wallet, mining, relay |
| Node Public RPC | `18533` | `0.0.0.0` | Read-only — no wallet/mining |
| P2P | `18335` | `0.0.0.0` | Must be reachable for DNS registration |
| Block Explorer | `8080` | `0.0.0.0` | Web UI |
| DNS Registry | `8053` | `0.0.0.0` | Bootstrap server |
| Stratum Pool | `3333` | `0.0.0.0` | Stratum TCP |
| Pool Stats | `8081` | `0.0.0.0` | Pool statistics JSON |
| Miner Dashboard | `8090` | `0.0.0.0` | Hashrate and status |

All ports are configurable. `PUBLIC_RPC_PORT=0` disables the public RPC.

## Configuration Model

### wallet-cli — `config.json`

Stored in `~/.Astram/config.json` (Linux/macOS) or `%APPDATA%\Astram\config.json` (Windows).  
Used only by wallet-cli; not read by the node.

```json
{
  "wallet_path": "~/.Astram/wallet.json",
  "node_rpc_url": "http://127.0.0.1:19533"
}
```

### Node — `config/nodeSettings.conf`

Read from `config/nodeSettings.conf` next to the binary or in the working directory.

Key settings:

| Key | Default | Description |
|-----|---------|-------------|
| `DATA_DIR` | `~/.Astram/data` | Blockchain DB root; logs go to `<DATA_DIR>/logs/` |
| `P2P_BIND_ADDR` | `0.0.0.0` | P2P listen address |
| `P2P_PORT` | `18335` | P2P listen port |
| `HTTP_BIND_ADDR` | `127.0.0.1` | Local HTTP API bind address |
| `HTTP_PORT` | `19533` | Local HTTP API port |
| `PUBLIC_RPC_PORT` | `18533` | Public read-only RPC (0 = disabled) |
| `DNS_SERVER_URL` | `http://161.33.19.183:8053` | DNS bootstrap server |
| `BOOTSTRAP_PEERS` | _(empty)_ | Comma-separated fallback peers |

### Miner — `config/minerSettings.conf`

| Key | Default | Description |
|-----|---------|-------------|
| `MINING_MODE` | `solo` | `solo` or `pool` |
| `NODE_RPC_URL` | `http://127.0.0.1:19533` | Node URL for solo mode |
| `POOL_HOST` | `127.0.0.1` | Stratum pool host |
| `POOL_PORT` | `3333` | Stratum pool port |
| `WORKER_NAME` | `worker1` | Worker identifier |
| `STATUS_PORT` | `8090` | Miner dashboard port |

## Storage

| Component | Storage | Location |
|-----------|---------|----------|
| Node chain DB | RocksDB | `<DATA_DIR>/` (default `~/.Astram/data/`) |
| Node log files | Rolling daily files | `<DATA_DIR>/logs/` — last 5 files kept |
| Explorer DB | RocksDB | `explorer_data/` (working dir) |
| Explorer logs | Rolling daily files | `logs/` (working dir) — last 5 files kept |
| Miner logs | Rolling daily files | `logs/` (working dir) — last 5 files kept |
| Wallet keys | JSON file | `<wallet_path>` (default `~/.Astram/wallet.json`) |
| CLI config | JSON file | `~/.Astram/config.json` |

## P2P Protocol

### Messages

| Message | Direction | Purpose |
|---------|-----------|---------|
| `Handshake` / `HandshakeAck` | Bidirectional | Initial connection with network/chain metadata |
| `Version` / `VerAck` | Bidirectional | Version confirmation |
| `GetHeaders` / `Headers` | Request/Response | Header synchronization |
| `Inv` | Broadcast | Announce new blocks or transactions |
| `GetData` | Request | Fetch block or transaction by hash |
| `Block` / `Tx` | Response | Deliver full object |
| `Ping` / `Pong` | Bidirectional | Liveness check |

### Connection Limits

| Limit | Value |
|-------|-------|
| Max outbound peers | 8 |
| Max peers per IP | 3 |
| Max peers per /24 subnet | 2 |
| Max peers per /16 subnet | 4 |
| Handshake timeout | 30 seconds |
| Max inventory items per message | 50,000 |
| Block announce rate | 10/min per peer |

### Peer Scoring (DNS Discovery)

When connecting to DNS-discovered peers, the node scores candidates using:

- **Height** — 30% weight
- **Uptime** — 20% weight (capped at 168 hours)
- **Latency** — 50% weight (lower is better)

Top-scored peers are connected first; unreachable peers (latency probe failed) are skipped.

## Operational Notes

- The node must have a publicly reachable P2P port for DNS registration; if registration fails, the node exits.
- The local HTTP API binds to `127.0.0.1` by default and should stay private.
- The public RPC (`18533`) is safe to expose externally — it has no wallet or mining endpoints.
- The explorer connects to the node at `http://127.0.0.1:19533` by default.
