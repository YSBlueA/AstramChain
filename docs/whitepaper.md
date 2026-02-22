# Astram Whitepaper (Implementation Overview)

## Abstract

Astram is a lightweight Proof-of-Work blockchain with a focus on fast propagation, practical mining on CPU or GPU, and a compact operational footprint. This document summarizes the current implementation as found in this repository.

## Goals

- Provide a simple PoW chain that can be run and tested locally.
- Support both CPU and optional CUDA mining backends.
- Expose a straightforward HTTP API for tooling integration.
- Maintain a minimal wallet UX via a command-line wallet tool.

## System Model

Astram consists of:

- A node process that handles P2P, mining, and HTTP APIs.
- A DNS discovery server used for public node registry.
- An explorer that reads data from the node.
- A CLI wallet used for key management and transaction submission.

## Consensus and Mining

- Consensus is Proof-of-Work using a Bitcoin-style numeric target check (`hash_u256 < target_u256`).
- Mining can run on CPU or CUDA (feature-flag based builds).
- Difficulty is dynamically adjusted to converge toward a 120-second block interval.

### PoW target model

- The block header stores compact target bits (`nBits`-style `u32`) in the `difficulty` field.
- Validation converts compact bits to a full 256-bit target and enforces `hash_u256 < target_u256`.
- This model supports fine-grained retargeting without large 16x step jumps.

### Difficulty adjustment

- Target block time: 120 seconds.
- Retarget cadence: every block, using a rolling 30-block timing window.
- Core formula: `new_target = old_target * actual_timespan / target_timespan`.
- Timespan clamp: `actual_timespan` is bounded to `[target_timespan/4, target_timespan*4]`.
- Damping: each block applies 25% of the computed move to reduce oscillation.
- Bounds: target is clamped between a network maximum target (`POW_LIMIT_BITS`) and minimum target (`POW_MIN_BITS`).
- Validation additionally rejects per-block target changes beyond 4x in either direction.

### Block reward and halving

- Initial block reward: 8 ASRM.
- Halving interval: 210,000 blocks.
- Max supply target: 42,000,000 ASRM.

### Fee policy

- Minimum relay fee is base + per-byte cost.
- Base minimum fee: 0.0001 ASRM.
- Per-byte minimum fee: 200 Gwei/byte.
- Default wallet fee uses a higher per-byte rate: 300 Gwei/byte.

## Token Distribution Model

### Supply and issuance

- Base unit: 1 ASRM = 10^18 ram.
- Initial block reward: 8 ASRM.
- Halving interval: 210,000 blocks.
- Max supply target: 42,000,000 ASRM.

### Fees

- Base minimum fee: 0.0001 ASRM.
- Per-byte relay fee: 200 Gwei/byte.
- Default wallet fee: 300 Gwei/byte.

## Data Model

- The system uses a UTXO-based transaction model.
- Blocks include a header and a transaction list.
- Block and transaction serialization relies on bincode configuration shared across crates.

## Genesis Specification

- Genesis timestamp lower bound: 1738800000 (Unix time).
- Blocks with timestamps earlier than this are rejected.
- Genesis block hash: TBD.
- Genesis merkle root: TBD.
- Genesis block content and hash are defined by the implementation and build artifacts.

## Networking

- P2P networking is used for block and transaction propagation.
- Nodes optionally register with a DNS discovery service for public reachability.
- DNS registration validates public reachability before accepting a node.

### Peer discovery and scoring

- The DNS service returns a candidate list of nodes.
- Nodes score candidates using a composite metric: height (30%), uptime (20%), latency (50%).
- Localhost and self-registered addresses are excluded from outbound selection.

### P2P protocol messages

- Handshake and versioning: `Handshake`, `HandshakeAck`, `Version`, `VerAck`.
- Chain sync: `GetHeaders`, `Headers`.
- Inventory relay: `Inv`, `GetData`, followed by `Block` or `Tx`.
- Liveness: `Ping` and `Pong`.

### Connection and relay flow

1. Peers connect and exchange handshake/version metadata.
2. A node advertises new objects via `Inv`.
3. The peer requests content using `GetData`.
4. The sender responds with `Block` or `Tx`.

### Synchronization

- Nodes request headers first, then fetch blocks based on the local tip.
- Periodic header sync runs in the background to maintain progress.
- Sync is tolerant of delays and continues after timeouts.

### Peer safety controls

- Connection limits: per-IP and per-subnet caps reduce Eclipse risk.
- Handshake timeout and inventory size limits prevent resource abuse.
- Block announcement rate limiting reduces spam pressure.

## Network Specification

### Identity and versioning

- Protocol version: 1.
- Mainnet Network ID: Astram-mainnet.
- Mainnet Chain ID: 1.
- Testnet Network ID: Astram-testnet.
- Testnet Chain ID: 8888.

### Network selection

- Default is mainnet when no environment variables are set.
- Set `ASTRAM_NETWORK=testnet` to switch to testnet defaults.
- Override with `ASTRAM_NETWORK_ID` or `ASTRAM_CHAIN_ID` for custom networks.

### Message set

- `Handshake` and `HandshakeAck` exchange `HandshakeInfo` (protocol version, software version, features, network/chain IDs, height, listening port).
- `Version` and `VerAck` provide lightweight version confirmation.
- `GetHeaders` / `Headers` are used for header synchronization.
- `Inv` / `GetData` relay inventory and request payloads.
- `Block` / `Tx` deliver full objects.
- `Ping` / `Pong` support liveness checks.

### Limits and safety controls

- Max outbound peers: 8.
- Max peers per IP: 3.
- Max peers per /24: 2; max per /16: 4; outbound diversity target: 3 distinct /16.
- Handshake timeout: 30 seconds.
- Max inventory items per message: 50,000.
- Block announce rate limit: 10 per minute per peer.

## Parameter Summary

| Category  | Parameter                 | Value                                                         |
| --------- | ------------------------- | ------------------------------------------------------------- |
| Consensus | Target block time         | 120 seconds                                                   |
| Consensus | Retarget cadence          | Every block (rolling 30-block timing window)                  |
| Consensus | Retarget formula          | `new_target = old_target * actual_timespan / target_timespan` |
| Consensus | Per-update damping        | 25% toward computed target                                    |
| Consensus | Timespan clamp            | `[target_timespan/4, target_timespan*4]`                      |
| Consensus | PoW target                | `hash_u256 < target_u256`                                     |
| Consensus | Difficulty encoding       | Compact bits (`nBits`-style `u32`)                            |
| Consensus | Initial block reward      | 8 ASRM                                                        |
| Consensus | Halving interval          | 210,000 blocks                                                |
| Consensus | Max supply target         | 42,000,000 ASRM                                               |
| Network   | Protocol version          | 1                                                             |
| Network   | Mainnet Network ID        | Astram-mainnet                                                |
| Network   | Mainnet Chain ID          | 1                                                             |
| Network   | Testnet Network ID        | Astram-testnet                                                |
| Network   | Testnet Chain ID          | 8888                                                          |
| Network   | Max outbound peers        | 8                                                             |
| Network   | Max peers per IP          | 3                                                             |
| Network   | Max peers per /24         | 2                                                             |
| Network   | Max peers per /16         | 4                                                             |
| Network   | Outbound /16 diversity    | 3 distinct subnets                                            |
| Network   | Handshake timeout         | 30 seconds                                                    |
| Network   | Max inventory per message | 50,000                                                        |
| Network   | Block announce rate       | 10 per minute per peer                                        |
| Fees      | Base minimum fee          | 0.0001 ASRM                                                   |
| Fees      | Per-byte relay fee        | 200 Gwei/byte                                                 |
| Fees      | Default wallet fee        | 300 Gwei/byte                                                 |
| Limits    | Max transaction size      | 100 KB                                                        |
| Limits    | Max inputs per tx         | 1000                                                          |
| Limits    | Max outputs per tx        | 1000                                                          |
| Limits    | Min output value          | 1 Twei                                                        |
| Limits    | Max reorg depth           | 100                                                           |

## APIs

- HTTP API: serves chain data, mempool queries, and node status.

## Configuration

- Wallet configuration is stored in a user config file used by the wallet-cli.
- Node runtime settings are read from config/nodeSettings.conf at startup.

## Security Considerations (Summary)

- PoW security assumptions are standard: economic cost of reorgs and double spends.
- DNS registration only accepts publicly reachable nodes to reduce false listings.
- Local wallet keys are stored as JSON and must be protected by the user.

### Validation constraints

- Max transaction size: 100 KB.
- Max inputs/outputs per transaction: 1000 each.
- Minimum output value: 1 Twei.
- Block timestamps must be greater than median-time-past and not too far in the future.
- Reorg depth is capped to reduce deep reorg risk.

### Operational security

- HTTP and JSON-RPC servers default to localhost bindings.
- Expose P2P ports only when DNS registration is required.
- Run nodes with restricted OS permissions and monitor for repeated invalid blocks.

### Attack scenarios and mitigations

- Eclipse attempts: enforced subnet diversity and per-IP limits reduce monopolization.
- Spam transactions: size, count, and dust limits increase attacker cost.
- Time-warp attempts: median-time-past and future timestamp bounds restrict manipulation.
- Deep reorgs: maximum reorg depth rejects large reorganizations.
- Invalid blocks/txs: validation counters and early rejection reduce propagation of bad data.

## Upgrade & Fork Policy

- No formal on-chain upgrade mechanism is defined in this implementation.
- Protocol changes require coordinated client releases and operator upgrades.
- Backward-incompatible changes are expected to be treated as scheduled hard forks.

## Limitations and Open Questions

- Formal protocol specification and testnet parameters are not yet documented.
- The DNS system is centralized and should be treated as a convenience service.
- Transaction fee policy is implemented in code but not yet formalized in a separate spec.

## Roadmap (High Level)

- Expand documentation to include formal protocol rules.
- Add better testing coverage for consensus and P2P behavior.
- Improve wallet UX and explorer features.
