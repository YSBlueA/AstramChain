# NetCoin Security Implementation

## Overview

This document describes the comprehensive security measures implemented in NetCoin to protect against various attack vectors including 51% attacks, timestamp manipulation, deep reorganizations, and network-level DoS attacks.

## 1. Cumulative Work-Based Chain Selection

**Implementation:** `core/src/blockchain/mod.rs::calculate_chain_work()`

**Security Features:**

- Work calculation with overflow protection using `checked_pow` and `saturating_add`
- Difficulty validation (0-32 range) to prevent invalid blocks
- Difficulty-based work calculation: `work = 2^difficulty`
- Rejects blocks with invalid difficulty values

**Protection Against:**

- Integer overflow attacks on work calculation
- Chain selection manipulation via invalid difficulty
- Computational complexity attacks

**Code Location:** [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs)

## 2. Deep Reorganization Policy Separation

**Implementation:** `core/src/security.rs::validate_reorg_depth()`

**Security Features:**

- Maximum reorg depth: 100 blocks (`MAX_REORG_DEPTH`)
- Warning threshold: 50 blocks
- Critical alerts for deep reorganizations
- Separate policy flag: `enable_deep_reorg_alerts`

**Protection Against:**

- 51% attacks with pre-mined chains
- History rewriting attacks
- Exchange deposit manipulation

**Code Location:** [core/src/security.rs](core/src/security.rs)

## 3. Median-Time-Past Timestamp Validation

**Implementation:** `core/src/blockchain/mod.rs::validate_median_time_past()`

**Security Features:**

- BIP-113 style validation using 11-block median
- Block timestamp must be greater than median of previous 11 blocks
- Prevents backward time manipulation
- Future block time validation (max 2 hours ahead)
- Genesis timestamp enforcement (1738800000)

**Protection Against:**

- Time warp attacks
- Difficulty manipulation via timestamp
- Fake difficulty reductions

**Code Location:** [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs)

## 4. Difficulty Adjustment with Slow Start

**Implementation:** `core/src/blockchain/mod.rs::calculate_adjusted_difficulty()`

**Security Features:**

- Slow start for first 100 blocks: `difficulty = 1 + (height / 20)` capped at 3
- Minimum difficulty enforcement: `MIN_DIFFICULTY = 1`
- Prevents instant high-difficulty attacks on new networks
- Gradual difficulty increase during bootstrap

**Protection Against:**

- Initial network instability
- Early mining monopolization
- Bootstrap attacks

**Code Location:** [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs)

## 5. Orphan Block Cache Limits

**Implementation:** `node/src/p2p/service.rs`, `node/src/lib.rs`

**Security Features:**

- Maximum orphan blocks: 100 (`MAX_ORPHAN_BLOCKS`)
- Orphan timeout: 1800 seconds (30 minutes, `ORPHAN_TIMEOUT`)
- Oldest-first eviction when pool is full
- Automatic cleanup of expired orphans

**Protection Against:**

- Orphan block flooding
- Memory exhaustion attacks
- Chain spam attacks

**Code Location:** [node/src/p2p/service.rs](node/src/p2p/service.rs)

## 6. Memory Block Limits

**Implementation:** `node/src/lib.rs`, `node/src/main.rs`, `node/src/p2p/service.rs`

**Security Features:**

- Maximum in-memory blocks: 500 (`MAX_MEMORY_BLOCKS`)
- Automatic eviction of oldest blocks when limit exceeded
- `enforce_memory_limit()` called after every block insertion
- Warning logs when approaching/exceeding limit
- ✅ **Status:** Fully implemented and enforced

**Protection Against:**

- Memory exhaustion attacks
- DoS via block spam
- Out-of-memory crashes

**Code Location:**

- [node/src/lib.rs](node/src/lib.rs) - Limit constant and enforcement function
- [node/src/main.rs](node/src/main.rs) - Applied after mining and P2P blocks
- [node/src/p2p/service.rs](node/src/p2p/service.rs) - Applied after sync blocks

## 7. Transaction Replay Protection

**Implementation:** Built into UTXO model and blockchain reorganization

**Security Features:**

- UTXO rollback on chain reorganization
- Transaction replay on valid chain
- Double-spend detection via UTXO tracking
- Transaction invalidation on reorg

**Protection Against:**

- Double-spend attacks post-reorg
- Transaction replay attacks
- Invalid state transitions

**Code Location:** [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs)

## 8. Checkpoint Policy System

**Implementation:** `core/src/checkpoint.rs`, `core/src/blockchain/mod.rs`

**Policy Design (NOT Consensus):**

- Checkpoints are **POLICY-LEVEL** protections, not consensus rules
- Used by pools, exchanges, explorers for safe reference points
- Does not affect basic block validity, only chain selection policy
- Prevents long-range history rewrites at critical heights

**Security Features:**

- Hardcoded block hashes at specific heights (genesis, milestones)
- Policy check: `validate_against_checkpoints()` - rejects conflicting chains
- Reorg protection: `check_reorg_against_checkpoints()` - prevents reorgs below checkpoint
- Latest checkpoint height tracking for finality determination
- Human-readable descriptions for each checkpoint
- ✅ **Status:** Fully implemented with policy separation

**Protection Against:**

- Very deep reorgs past critical blocks (50+ blocks)
- Long-range history alteration attacks
- Eclipse attacks with fake chain history
- 51% attacks on early/vulnerable network state

**Code Location:**

- [core/src/checkpoint.rs](core/src/checkpoint.rs) - Policy implementation
- [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs) - Integration with validation and reorg

**Checkpoint Structure:**

```rust
Checkpoint {
    height: 0,
    hash: "genesis_hash",
    description: "Genesis Block - Network Origin"
}
```

**Key Distinction:**

- ❌ NOT consensus: Other nodes can accept blocks that violate checkpoints
- ✅ Policy only: This node will reject chains conflicting with its checkpoints
- Purpose: Provides stability anchor for exchanges/pools without forcing network-wide rules

## 9. Network-Level Defenses

**Implementation:** `node/src/p2p/manager.rs`

**Security Features:**

- **IP Connection Limiting:** Maximum 3 peers per IP (`MAX_PEERS_PER_IP`)
  - Enforced in `handle_incoming()` before accepting connections
  - IP tracking via `peer_ips: HashMap<String, Vec<PeerId>>`
  - Automatic cleanup on peer disconnect
- **Message Size Validation:**
  - INV message limit: 50,000 items (`MAX_INV_PER_MESSAGE`)
  - GetData message limit: 50,000 items
  - Drops excessive messages with warning logs

- **Rate Limiting Constants:**
  - Handshake timeout: 30 seconds (`HANDSHAKE_TIMEOUT_SECS`)
  - Block announce rate: 10 per minute (`BLOCK_ANNOUNCE_RATE_LIMIT`)

- ✅ **Status:** IP limiting and message validation fully implemented

**Protection Against:**

- Sybil attacks (multiple connections from same IP)
- Inventory spam attacks (memory exhaustion via large INV messages)
- GetData flooding attacks
- Connection exhaustion attacks

**Code Location:** [node/src/p2p/manager.rs](node/src/p2p/manager.rs)

**Implementation Details:**

```rust
// Connection acceptance with IP check
if peer_count >= MAX_PEERS_PER_IP {
    warn!("Rejecting connection from {} - IP limit exceeded", peer_id);
    return Ok(());
}

// Message validation
if hashes.len() > MAX_INV_PER_MESSAGE {
    warn!("Excessive INV message, ignoring");
    return;
}
```

## 10. Explorer Protection

**Implementation:** `explorer/src/state.rs`, `explorer/src/db.rs`, Vue components

**Security Features:**

- Confirmation count display on all blocks and transactions
- Status indicators:
  - 0 confirmations: "Unconfirmed" (red)
  - 1-5 confirmations: "Low Confidence" (yellow)
  - 6+ confirmations: "Confirmed" (green)
- Real-time confirmation calculation based on chain height
- Reorg alert structure defined

**Protection Against:**

- User confusion during reorgs
- Premature transaction acceptance
- Low-confirmation fraud

**Code Location:**

- [explorer/src/state.rs](explorer/src/state.rs)
- [explorer/src/db.rs](explorer/src/db.rs)
- [explorer/web/src/views/BlockDetail.vue](explorer/web/src/views/BlockDetail.vue)
- [explorer/web/src/views/TransactionDetail.vue](explorer/web/src/views/TransactionDetail.vue)

## Security Constants Reference

### Blockchain Constants (`core/src/blockchain/mod.rs`)

- `max_reorg_depth: 100` - Maximum reorganization depth
- `max_future_block_time: 7200` - Max seconds block can be in future (2 hours)
- `enable_deep_reorg_alerts: true` - Enable critical reorg warnings

### Security Constants (`core/src/security.rs`)

```rust
pub const MAX_REORG_DEPTH: u64 = 100;
pub const REORG_WARNING_THRESHOLD: u64 = 50;
pub const GENESIS_TIMESTAMP: i64 = 1738800000;
```

### Node Constants (`node/src/lib.rs`)

```rust
pub const MAX_ORPHAN_BLOCKS: usize = 100;
pub const MAX_MEMORY_BLOCKS: usize = 500;
pub const ORPHAN_TIMEOUT: u64 = 1800; // 30 minutes
```

### P2P Network Constants (`node/src/p2p/manager.rs`)

```rust
pub const MAX_PEERS_PER_IP: usize = 3;
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 30;
pub const MAX_INV_PER_MESSAGE: usize = 50000;
pub const BLOCK_ANNOUNCE_RATE_LIMIT: u64 = 10; // per minute
```

## Pending Implementation

### Completed ✅

All 10 security features are now fully implemented:

1. ✅ Work calculation overflow protection
2. ✅ Deep reorg policy separation
3. ✅ Median-Time-Past timestamp validation
4. ✅ Difficulty adjustment slow start
5. ✅ Orphan block cache limits (with eviction)
6. ✅ Memory block limits (fully enforced)
7. ✅ Transaction replay protection
8. ✅ Checkpoint policy system
9. ✅ Network-level defenses (IP limiting + message validation)
10. ✅ Explorer protection (confirmation display)

### Future Enhancements

1. **Advanced Rate Limiting:**
   - Per-peer handshake timeout with tokio::time::timeout
   - Block announcement rate tracking with timestamp throttling
   - Adaptive rate limits based on network load

2. **Enhanced Reorg Detection:**
   - Reorg event logging to explorer database
   - Webhook/notification system for deep reorgs
   - Historical reorg tracking and analytics

3. **Checkpoint Evolution:**
   - Automated checkpoint proposal system
   - Community consensus mechanism for new checkpoints
   - Checkpoint verification during initial sync

4. **Advanced Monitoring:**
   - Real-time memory usage tracking
   - Network bandwidth monitoring per peer
   - Anomaly detection for attack patterns

## Testing Recommendations

1. **Deep Reorg Testing:**
   - Test reorg at 49 blocks (warning threshold)
   - Test reorg at 51 blocks (critical alert)
   - Test reorg rejection at 101 blocks (exceeds MAX_REORG_DEPTH)
   - Test checkpoint-based reorg rejection

2. **Timestamp Validation:**
   - Test blocks with timestamps before median (should reject)
   - Test future blocks beyond 2-hour limit (should reject)
   - Test genesis timestamp enforcement

3. **Memory Limits:**
   - Add >500 blocks and verify oldest eviction
   - Monitor memory usage during eviction
   - Test orphan pool with >100 blocks

4. **Network Defenses:**
   - Test 4+ simultaneous connections from same IP (4th should be rejected)
   - Test INV message with 50,001 items (should be dropped)
   - Test GetData message with excessive items
   - Verify IP cleanup on peer disconnect

## Security Audit Checklist

**Core Features (v1.0 - v2.0):**

- [x] Work calculation overflow protection
- [x] Deep reorg depth limits
- [x] Median-Time-Past timestamp validation
- [x] Difficulty adjustment slow start
- [x] Orphan block cache limits with eviction
- [x] Memory block limits with enforcement
- [x] TX replay protection via UTXO
- [x] Checkpoint policy system (separated from consensus)
- [x] Network IP limiting and message validation
- [x] Explorer confirmation display

**Enhanced Features (v2.1):**

- [x] Block validation failure tracking & statistics
- [x] Mempool DoS protection with size/fee-based eviction
- [x] Peer subnet diversity enforcement (Eclipse attack protection)

**Legend:**

- [x] Fully implemented and operational
- [ ] Not implemented

**Status: 13/13 SECURITY FEATURES COMPLETE ✅**

## 11. Validation Failure Tracking & Statistics

**Implementation:** `core/src/security.rs`, `core/src/blockchain/mod.rs`

**Security Features:**

- 17 categorized failure reasons with enum codes
- Atomic counter statistics per failure type
- Real-time metrics via `/status` API endpoint
- Structured logging for debugging attacks
- Zero-overhead lazy_static global stats

**Failure Categories:**

```rust
HashMismatch, InvalidPoW, DifficultyOutOfRange, MerkleRootMismatch,
TimestampTooOld, TimestampTooFuture, PreviousNotFound, EmptyBlock,
InvalidCoinbase, SignatureFailure, UtxoNotFound, UtxoOwnershipFailure,
DuplicateInput, InsufficientFee, CheckpointViolation, SecurityConstraint
```

**Protection Against:**

- Blind attacks (visibility into attack patterns)
- Network reconnaissance (track what attackers probe)
- Performance degradation (identify bottleneck validations)

**Code Location:**

- [core/src/security.rs](core/src/security.rs#L19-L148)
- [core/src/blockchain/mod.rs](core/src/blockchain/mod.rs) - Integrated at validation points

## 12. Mempool DoS Protection

**Implementation:** `node/src/lib.rs::enforce_mempool_limit()`

**Security Features:**

- Transaction count limit: 10,000 (`MAX_MEMPOOL_SIZE`)
- Total size limit: 300MB (`MAX_MEMPOOL_BYTES`)
- Time-based expiry: 24 hours (`MEMPOOL_EXPIRY_TIME`)
- Fee-rate prioritization: Low-fee transactions evicted first
- Minimum relay fee: 1 Gwei/byte (`MIN_RELAY_FEE_PER_BYTE`)
- Automatic enforcement on every transaction addition

**Eviction Strategy:**

1. Remove transactions older than 24 hours
2. Sort remaining by fee-per-byte (ascending)
3. Evict lowest-fee txs until under count limit
4. Continue eviction until under byte-size limit

**Protection Against:**

- Memory exhaustion via transaction spam
- Low-fee transaction flooding
- State bloat attacks
- Resource starvation

**Code Location:**

- [node/src/lib.rs](node/src/lib.rs#L77-L168) - Enforcement logic
- [node/src/main.rs](node/src/main.rs) - Applied after block failures and mining errors
- [node/src/p2p/service.rs](node/src/p2p/service.rs) - Applied on P2P transaction receipt

## 13. Peer Diversity & Eclipse Attack Protection

**Implementation:** `node/src/p2p/manager.rs`

**Security Features:**

- **Subnet diversity limits:**
  - Max 2 peers per /24 subnet (`MAX_PEERS_PER_SUBNET_24`)
  - Max 4 peers per /16 subnet (`MAX_PEERS_PER_SUBNET_16`)
  - Minimum 3 different /16 subnets for outbound (`MIN_OUTBOUND_SUBNET_DIVERSITY`)
- **Automatic enforcement:**
  - Connection rejection on subnet limit violation
  - Real-time diversity metrics tracking
  - Logged warnings with subnet information

**Attack Prevention:**

- **Eclipse attacks:** Prevents attacker from monopolizing connections
- **Sybil attacks:** Limits multiple identities from same network
- **Network partitioning:** Ensures geographic/ISP diversity
- **Targeted isolation:** Stops subnet-based node isolation

**Metrics Available:**

- Unique /24 subnets connected
- Unique /16 subnets connected
- Exposed via `/status` API endpoint

**Code Location:** [node/src/p2p/manager.rs](node/src/p2p/manager.rs#L151-L213)

## Enhanced Security Constants

### Mempool Protection (`node/src/lib.rs`)

```rust
pub const MAX_MEMPOOL_SIZE: usize = 10000;
pub const MAX_MEMPOOL_BYTES: usize = 300_000_000; // 300MB
pub const MEMPOOL_EXPIRY_TIME: i64 = 86400; // 24 hours
pub const MIN_RELAY_FEE_PER_BYTE: u64 = 1_000_000; // 1 Gwei
```

### Subnet Diversity (`node/src/p2p/manager.rs`)

```rust
pub const MAX_PEERS_PER_SUBNET_24: usize = 2;
pub const MAX_PEERS_PER_SUBNET_16: usize = 4;
pub const MIN_OUTBOUND_SUBNET_DIVERSITY: usize = 3;
```

## Testing Recommendations (Extended)

5. **Validation Statistics:**
   - Send blocks with invalid PoW, check stats increment
   - Send blocks with wrong timestamps, verify categorization
   - Verify `/status` API shows failure breakdown

6. **Mempool Stress Testing:**
   - Send 10,001 transactions, verify eviction
   - Send 500MB of low-fee txs, check byte-limit enforcement
   - Wait 24+ hours, verify expired tx removal

7. **Subnet Diversity:**
   - Connect 3+ peers from same /24, verify 3rd rejected
   - Connect 5+ peers from same /16, verify 5th rejected
   - Verify diversity metrics in `/status` endpoint

## Security Contact

For security issues or vulnerabilities, please contact the development team through the appropriate channels before public disclosure.

## Version History

- **v1.0 (2025-01-31):** Initial comprehensive security implementation
  - Added 8 core security features
  - Partial implementation of 2 additional features
  - Explorer confirmation tracking

- **v2.0 (2026-02-05):** Complete security hardening ✅
  - **Memory block limits:** Full enforcement with automatic eviction
  - **Network defenses:** IP-based connection limiting (max 3 per IP)
  - **Network defenses:** INV/GetData message size validation (max 50K items)
  - **Checkpoint system:** Refactored as policy-level (not consensus)
  - **Checkpoint integration:** Applied to block validation and reorganization
  - All 10 security features now fully operational
  - Production-ready security posture achieved

- **v2.1 (2025-02-05):** Advanced monitoring and Eclipse protection ✅
  - **Validation failure tracking:** 17 categorized failure reasons with atomic statistics
  - **Mempool DoS protection:** Triple-layer defense (10K count, 300MB size, 24h expiry)
  - **Peer subnet diversity:** /24 and /16 limits prevent Eclipse attacks

  **New security constants:**
  - `MAX_MEMPOOL_SIZE = 10000`
  - `MAX_MEMPOOL_BYTES = 300_000_000`
  - `MAX_PEERS_PER_SUBNET_24 = 2`
  - `MAX_PEERS_PER_SUBNET_16 = 4`
  - `MIN_OUTBOUND_SUBNET_DIVERSITY = 3`

  **New API metrics in /status endpoint:**
  - `validation_failures_total`: Total failed validations
  - `validation_failures`: Array with per-category breakdown
  - `mempool.max_size` and `mempool.max_bytes`: Mempool limits
  - `network.subnet_diversity`: Unique subnet counts (IPv4 /24 and /16)

  **Status:** 13/13 features complete ✅
