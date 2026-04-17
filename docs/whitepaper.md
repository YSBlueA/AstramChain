# Astram Whitepaper (Implementation Overview)

## Abstract

Astram is a lightweight Proof-of-Work blockchain with a focus on fast propagation, practical GPU mining, and a compact operational footprint. This document summarizes the current implementation as found in this repository.

## Goals

- Provide a simple PoW chain that can be run and tested locally.
- Support CUDA-based GPU mining with ASIC-resistant memory-hard hashing.
- Expose a straightforward HTTP API for tooling and dApp integration.
- Maintain a minimal wallet UX via CLI and browser extension wallet tools.

## System Model

Astram consists of:

| Component | Role |
|-----------|------|
| **Astram-node** | P2P networking, chain validation, HTTP API, public RPC |
| **Astram-miner** | Standalone CUDA GPU miner (solo and pool modes) |
| **astram-stratum** | Stratum mining pool server |
| **astram-dns** | Public node discovery and bootstrap registry |
| **Astram-explorer** | Block explorer that indexes data from the node |
| **wallet-cli** | CLI key management and transaction submission |
| **AstramX Wallet** | Chrome extension wallet for dApps |

## Consensus and Mining

Consensus is Proof-of-Work using a Bitcoin-style numeric target check (`hash_u256 < target_u256`).  
Mining uses the KawPow-Blake3 algorithm — a memory-hard design with a 4 GB DAG, inspired by KawPow (Ravencoin) but using Blake3 instead of SHA-256.

### PoW Target Model

- The block header stores compact target bits (`nBits`-style `u32`) in the `difficulty` field.
- Validation converts compact bits to a full 256-bit target and enforces `hash_u256 < target_u256`.
- This model supports fine-grained retargeting without large discrete jumps.

### Difficulty Adjustment (DWG3)

- **Target block time**: 120 seconds.
- **Algorithm**: DWG3 (Dark Gravity Wave v3 style).
- **Retarget cadence**: Every block.
- **Timing window**: Most recent **24 blocks**.
- **Core formula**: `new_target = avg_past_target × actual_timespan / target_timespan`.
- **Timespan clamp**: `actual_timespan` bounded to `[target_timespan/3, target_timespan×3]`.
- **Damping**: Each block applies 25% of the computed move to reduce oscillation.
- **Per-block limit**: Target change capped at 4× in either direction.
- **Bounds**: Target clamped between `POW_LIMIT_BITS` (max) and `POW_MIN_BITS` (min).

### KawPow-Blake3 Hash Algorithm

Three-stage process per mining attempt:

```
Stage 1:  Blake3(BlockHeader)          →  initial_hash  [32 bytes]
Stage 2:  Memory-hard mix (4 GB DAG)   →  mixed_data    [128 bytes]
Stage 3:  Blake3(mixed_data)           →  final_hash    [32 bytes]

final_hash is compared against the target.
```

**DAG (Directed Acyclic Graph)**:

| Parameter | Value |
|-----------|-------|
| Size | 4 GB (4,294,967,296 bytes) |
| Item size | 128 bytes |
| Item count | ~32 M items |
| Epoch length | 7,500 blocks |
| Mix iterations | 64 random reads per hash |

The DAG regenerates each epoch. Epoch 0 seed = `Blake3("Astram Genesis DAG Seed")`; each subsequent epoch seeds from `Blake3(prev_seed)`.  
First-run generation takes ~3–5 minutes; the GPU requires ≥ 4 GB VRAM.

### Block Reward and Halving

| Parameter | Value |
|-----------|-------|
| Initial block reward | 8 ASRM |
| Halving interval | 210,000 blocks |
| Max supply target | 42,000,000 ASRM |

### Fee Policy

| Parameter | Value |
|-----------|-------|
| Base minimum fee | 0.0001 ASRM (100 Twei) |
| Per-byte relay fee | 200 Gwei/byte |
| Default wallet fee | 300 Gwei/byte |

Minimum accepted fee = `base_fee + tx_size_bytes × relay_fee_per_byte`.

## Token Distribution Model

### Supply and Issuance

- Base unit: 1 ASRM = 10¹⁸ ram.
- Initial block reward: 8 ASRM, halving every 210,000 blocks.
- Max supply: ~42,000,000 ASRM.

## Data Model

- **Transaction model**: UTXO-based.
- **Keys**: Ed25519 key pairs.
- **Address format**: `0x` + hex(SHA-256(pubkey)[0:20]).
- **Serialization**: bincode v2 standard configuration, shared across all crates.
- **Block header fields**: `index`, `previous_hash`, `merkle_root`, `timestamp`, `nonce`, `difficulty`.
- **Merkle root**: Blake3-based.

## Genesis Specification

| Parameter | Value |
|-----------|-------|
| Genesis hash | `0047bb75cef130263090ec45c9e5b464ab0f56c556821cb3a40d59dbf31e7216` |
| Timestamp lower bound | 1738800000 (Unix, ≈ Feb 6, 2026) |

Blocks with timestamps earlier than the lower bound are rejected.

## Networking

### Peer Discovery and Scoring

1. The node registers with the DNS server at startup and every 5 minutes.
2. The DNS server returns a ranked candidate list.
3. The node probes candidate latency over TCP and scores each peer:
   - Height: 30%
   - Uptime: 20% (capped at 168 h)
   - Latency: 50% (lower is better)
4. Highest-scoring reachable peers are connected first; self and localhost addresses are excluded.

### P2P Protocol Messages

| Message | Purpose |
|---------|---------|
| `Handshake` / `HandshakeAck` | Exchange protocol version, network/chain IDs, height, listening port |
| `Version` / `VerAck` | Lightweight version confirmation |
| `GetHeaders` / `Headers` | Header chain synchronization |
| `Inv` / `GetData` | Announce and request blocks or transactions |
| `Block` / `Tx` | Deliver full objects |
| `Ping` / `Pong` | Liveness checks |

Frames use a 4-byte network magic prefix before the message payload.

### Connection and Relay Flow

1. Peers connect and exchange `Handshake` / `HandshakeAck` with network metadata.
2. A node advertises new objects via `Inv`.
3. The peer requests content using `GetData`.
4. The sender responds with `Block` or `Tx`.

### Synchronization

- Nodes request headers first, then fetch blocks based on the local tip.
- Background sync runs continuously; tolerates timeouts and peer disconnects.
- Startup sync waits for at least one connected peer before proceeding.

### Peer Safety Controls

- Per-IP, per-/24, and per-/16 connection limits reduce Eclipse risk.
- Handshake timeout (30 s) and inventory size limits prevent resource abuse.
- Block announcement rate limiting (10/min/peer) reduces spam.
- Stale LOCK file detection and cleanup on startup.

## Network Specification

### Identity and Versioning

| Parameter | Mainnet | Testnet |
|-----------|---------|---------|
| Protocol version | 1 | 1 |
| Network ID | `Astram-mainnet` | `Astram-testnet` |
| Chain ID | 1 | 8888 |
| Network Magic | `0xA57A0001` | `0xA57A22B8` |

### Network Selection

- **Release builds**: Hardcoded to mainnet. Environment overrides are disabled.
- **Debug builds**: Allow network selection via `ASTRAM_NETWORK=testnet` or overrides `ASTRAM_NETWORK_ID`, `ASTRAM_CHAIN_ID`, `ASTRAM_NETWORK_MAGIC`.

## HTTP API

### Local API (`127.0.0.1:19533`)

Full API — includes wallet, mining, and relay endpoints.

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Node health check |
| `GET /status` | Detailed node status (height, mempool, peers, mining) |
| `GET /counts` | Block, transaction, and volume counts |
| `GET /blockchain` | Basic blockchain info |
| `GET /blockchain/range?from=&to=` | Blocks by height range |
| `GET /blockchain/db` | All blocks from DB |
| `GET /mempool` | Mempool transactions (Base64-encoded bincode) |
| `GET /address/{addr}/balance` | Address balance (ram) |
| `GET /address/{addr}/utxos` | Address UTXO list |
| `POST /tx` | Submit and validate a transaction |
| `POST /tx/relay` | Relay a transaction from a peer |
| `POST /mining/submit` | Submit a mined block |
| `GET /debug/block-counts` | Memory vs DB block count debug |

### Public RPC (`0.0.0.0:18533`)

Read-only subset — safe to expose externally.  
Wallet, mining, relay, and internal debug endpoints are excluded.

## Mining Architecture

### Solo Mode

```
Astram-miner  ──GET /status──►  Astram-node
              ◄── chain info ──
              ──GET /mempool──►
              ◄── pending txs ──
              ── CUDA mine ────
              ──POST /mining/submit──►
```

### Pool Mode (Stratum)

```
Astram-miner  ──TCP Stratum──►  astram-stratum  ──POST /mining/submit──►  Astram-node
              ◄── job notify ──
              ── CUDA mine ────
              ──mining.submit──►
              ◄── result ──
```

The Stratum pool supports VarDiff (target ~15 s share time), PPLNS payout, and share validation against both pool and block difficulty.

## Parameter Summary

| Category | Parameter | Value |
|----------|-----------|-------|
| Consensus | Target block time | 120 seconds |
| Consensus | Retarget cadence | Every block |
| Consensus | Retarget window | 24 blocks |
| Consensus | Retarget formula | `new_target = avg_past_target × actual_timespan / target_timespan` |
| Consensus | Per-update damping | 25% toward computed target |
| Consensus | Timespan clamp | `[target_timespan/3, target_timespan×3]` |
| Consensus | Per-block change limit | 4× in either direction |
| Consensus | PoW check | `hash_u256 < target_u256` |
| Consensus | Difficulty encoding | Compact bits (`nBits`-style `u32`) |
| Consensus | Hash algorithm | KawPow-Blake3 (4 GB DAG) |
| Consensus | DAG epoch length | 7,500 blocks |
| Consensus | Initial block reward | 8 ASRM |
| Consensus | Halving interval | 210,000 blocks |
| Consensus | Max supply target | 42,000,000 ASRM |
| Consensus | Max reorg depth | 100 blocks |
| Network | Protocol version | 1 |
| Network | Mainnet Network ID | `Astram-mainnet` |
| Network | Mainnet Chain ID | 1 |
| Network | Mainnet Network Magic | `0xA57A0001` |
| Network | Testnet Network ID | `Astram-testnet` |
| Network | Testnet Chain ID | 8888 |
| Network | Testnet Network Magic | `0xA57A22B8` |
| Network | Max outbound peers | 8 |
| Network | Max peers per IP | 3 |
| Network | Max peers per /24 | 2 |
| Network | Max peers per /16 | 4 |
| Network | Handshake timeout | 30 seconds |
| Network | Max inventory per message | 50,000 |
| Network | Block announce rate | 10/min per peer |
| Fees | Base minimum fee | 0.0001 ASRM |
| Fees | Per-byte relay fee | 200 Gwei/byte |
| Fees | Default wallet fee | 300 Gwei/byte |
| Limits | Max transaction size | 100 KB |
| Limits | Max inputs per tx | 1,000 |
| Limits | Max outputs per tx | 1,000 |
| Limits | Min output value | 1 Twei |
| Limits | Max mempool transactions | 10,000 |
| Limits | Max mempool size | 300 MB |
| Limits | Mempool expiry | 24 hours |
| Limits | Max orphan blocks | 100 |
| Limits | Max in-memory blocks | 500 |

## Security Considerations

- PoW security assumptions are standard: economic cost of reorgs and double spends.
- DNS registration only accepts publicly reachable nodes.
- Local wallet keys are stored as JSON and must be protected by the user.
- Reorg depth is capped at 100 blocks to limit deep reorg risk.
- Checkpoints are enforced at the policy level (not consensus-breaking).

See [security.md](security.md) for full threat model and hardening checklist.

## Upgrade & Fork Policy

- No formal on-chain upgrade mechanism is defined in this implementation.
- Protocol changes require coordinated client releases and operator upgrades.
- Backward-incompatible changes are treated as scheduled hard forks.

## Limitations and Open Questions

- Formal protocol specification and testnet parameters are not yet published.
- The DNS system is centralized and should be treated as a convenience service, not a trust anchor.
- DAG disk caching is not yet implemented (re-generation takes ~3–5 min per epoch restart).
- Wallet encryption and hardware wallet support are not implemented.

## Roadmap

- DAG disk caching (skip re-generation on restart).
- Formal protocol specification.
- Expanded test coverage for consensus and P2P.
- Testnet launch.
- Wallet UX and explorer feature improvements.
