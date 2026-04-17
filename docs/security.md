# Security

## Threat Model

Astram assumes a standard PoW threat model:

- An attacker with significant hash power can attempt reorgs or double spends.
- Network attackers can attempt Eclipse or partition attacks.
- Malicious peers can attempt DoS via malformed or excessive traffic.
- Local attackers with filesystem access can steal wallet keys.

## Key Assets

- **Private keys** stored in the local wallet file.
- **Chain data** stored in the node data directory (RocksDB).
- **Network reachability** of the P2P port for DNS registration.

## Wallet Security

- Wallet keys are stored as JSON files on disk.
- Key derivation uses Ed25519; addresses are `0x` + `SHA-256(pubkey)[0:20]`.
- BIP39 24-word mnemonics allow wallet recovery.
- Users must protect wallet files using OS-level permissions and backups.
- Do not expose wallet files on shared or multi-user systems.
- **Wallet encryption and hardware wallet support are not yet implemented.**

## Node Security

- The local HTTP API (`127.0.0.1:19533`) binds to localhost by default — do not expose it externally.
- The public RPC (`0.0.0.0:18533`) exposes read-only endpoints; wallet, mining, and relay endpoints are excluded. Still, consider firewall rules for untrusted environments.
- P2P ports should be open only when public DNS registration is intended.
- DNS registration requires a publicly reachable port; only run public nodes on secured hosts.
- Consensus validation enforces numeric PoW targets (`hash_u256 < target_u256`) — not prefix-only checks.
- Difficulty retargeting applies bounded timespan clamps and per-block damping to reduce abrupt oscillations.

## Network Security

- DNS discovery is centralized — treat it as a convenience bootstrap, not a trust anchor.
- Peer connection diversity limits (per-IP, per-/24, per-/16) reduce Eclipse risk.
- All incoming blocks and transactions are fully validated before acceptance.

## DoS and Resource Limits

The following limits are enforced in code:

### Mempool

| Limit | Value |
|-------|-------|
| Max transactions | 10,000 |
| Max total size | 300 MB |
| Expiry | 24 hours |
| Min relay fee | 1 Gwei/byte |

### In-memory state

| Limit | Value |
|-------|-------|
| Max orphan blocks | 100 |
| Max in-memory blocks | 500 |

### Transaction validation

| Limit | Value |
|-------|-------|
| Max transaction size | 100 KB |
| Max inputs per tx | 1,000 |
| Max outputs per tx | 1,000 |
| Min output value | 1 Twei |

### P2P

| Limit | Value |
|-------|-------|
| Max outbound peers | 8 |
| Max peers per IP | 3 |
| Max peers per /24 subnet | 2 |
| Max peers per /16 subnet | 4 |
| Handshake timeout | 30 seconds |
| Max inventory items per message | 50,000 |
| Block announce rate | 10/min per peer |

### Address rate limiting

Address-based rate limiting is applied to transaction submission to reduce spam from individual senders.

## Validation Constraints

- Max transaction size: 100 KB.
- Max inputs and outputs per transaction: 1,000 each.
- Minimum output value: 1 Twei (prevents dust).
- Duplicate inputs within a single transaction are rejected.
- Block timestamps must be greater than the genesis lower bound (`1738800000`) and not unreasonably far in the future.
- Reorg depth is capped at **100 blocks** to reduce deep reorg risk.
- Policy-level checkpoints enforce the known genesis hash and can be extended with milestone hashes.

## Attack Scenarios and Mitigations

| Attack | Mitigation |
|--------|-----------|
| Eclipse attack | Per-IP and per-subnet connection limits; DNS diversity scoring |
| Spam transactions | Size, count, dust, and fee floor limits; address rate limiting |
| Time-warp | Genesis timestamp lower bound; future timestamp bounds |
| Deep reorg | Maximum reorg depth (100 blocks); policy checkpoints |
| Invalid block/tx flood | Validation counters; early rejection before propagation |
| Stale LOCK file | Automatic detection and cleanup on node startup |
| DoS via large inventory | Max 50,000 items per inventory message |
| Peer spam | Block announce rate limit (10/min/peer) |

## Operational Hardening Checklist

- Run the node under a dedicated OS user with minimal privileges.
- Use a firewall to restrict the local HTTP API (`19533`) to trusted hosts only.
- Do not expose the local HTTP API to the internet — use the public RPC (`18533`) instead.
- Keep data directories on storage with proper access controls (mode `700` or equivalent).
- Protect wallet JSON files (`chmod 600`).
- Monitor logs for repeated peer failures or suspicious registration patterns.
- Forward only the P2P port (`8335`) through NAT; keep all other ports firewalled unless explicitly needed.
- Back up the wallet mnemonic and store it offline.

## Log Monitoring

Logs are written to rolling daily files:

- Node: `<DATA_DIR>/logs/node_*.log`
- Miner: `logs/miner_*.log`
- Explorer: `logs/explorer_*.log`

Last 5 daily files are retained per service. Monitor for:

- Repeated `[ERROR]` or `[WARN]` entries from the P2P layer.
- High `validation_failures` counts in `/status` response.
- Unexpected reorg events (`[SYNC]` or `[REORG]` log lines).

## Open Items

- Formal protocol-level security analysis is pending.
- DoS and resource-exhaustion limits should be revisited as the network grows.
- Wallet file encryption is not yet implemented.
- The DNS system has no authentication — a compromised DNS server can return malicious peer lists.
