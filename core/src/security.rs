use crate::block::Block;
use crate::transaction::Transaction;
/// Security validation utilities for blockchain operations
use anyhow::{Result, anyhow};
use primitive_types::U256;
use std::sync::atomic::{AtomicU64, Ordering};

/// Security constants
pub const MAX_TX_SIZE: usize = 100_000; // 100KB max transaction size
pub const MIN_OUTPUT_VALUE: u64 = 1_000_000_000_000; // 1 Twei (0.000001 NTC) minimum to prevent dust
pub const MAX_TX_INPUTS: usize = 1000; // Prevent huge transactions
pub const MAX_TX_OUTPUTS: usize = 1000;
pub const MAX_FUTURE_TIMESTAMP: i64 = 7200; // 2 hours tolerance
pub const MAX_REORG_DEPTH: u64 = 100; // Maximum blocks to reorganize (51% attack protection)
pub const GENESIS_TIMESTAMP: i64 = 1738800000; // ~Feb 6, 2026 - blocks before this are invalid
pub const REORG_WARNING_THRESHOLD: u64 = 50;

/// Block validation failure reasons (for statistics and debugging)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockFailureReason {
    HashMismatch,         // Computed hash doesn't match
    InvalidPoW,           // Proof of work doesn't meet difficulty
    DifficultyOutOfRange, // Difficulty changed too much
    MerkleRootMismatch,   // Merkle root doesn't match transactions
    TimestampTooOld,      // Timestamp before median-time-past
    TimestampTooFuture,   // Timestamp too far in future
    PreviousNotFound,     // Parent block doesn't exist
    EmptyBlock,           // No transactions
    InvalidCoinbase,      // Coinbase transaction is invalid
    SignatureFailure,     // Transaction signature verification failed
    UtxoNotFound,         // Referenced UTXO doesn't exist
    UtxoOwnershipFailure, // UTXO ownership verification failed
    DuplicateInput,       // Same input used twice in transaction
    InsufficientFee,      // Output sum > input sum
    CheckpointViolation,  // Conflicts with checkpoint policy
    SecurityConstraint,   // Generic security constraint violation
    Other,                // Other/unknown reason
}

impl BlockFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HashMismatch => "hash_mismatch",
            Self::InvalidPoW => "invalid_pow",
            Self::DifficultyOutOfRange => "difficulty_out_of_range",
            Self::MerkleRootMismatch => "merkle_root_mismatch",
            Self::TimestampTooOld => "timestamp_too_old",
            Self::TimestampTooFuture => "timestamp_too_future",
            Self::PreviousNotFound => "previous_not_found",
            Self::EmptyBlock => "empty_block",
            Self::InvalidCoinbase => "invalid_coinbase",
            Self::SignatureFailure => "signature_failure",
            Self::UtxoNotFound => "utxo_not_found",
            Self::UtxoOwnershipFailure => "utxo_ownership_failure",
            Self::DuplicateInput => "duplicate_input",
            Self::InsufficientFee => "insufficient_fee",
            Self::CheckpointViolation => "checkpoint_violation",
            Self::SecurityConstraint => "security_constraint",
            Self::Other => "other",
        }
    }
}

/// Global statistics for block validation failures
pub struct ValidationStats {
    pub hash_mismatch: AtomicU64,
    pub invalid_pow: AtomicU64,
    pub difficulty_out_of_range: AtomicU64,
    pub merkle_root_mismatch: AtomicU64,
    pub timestamp_too_old: AtomicU64,
    pub timestamp_too_future: AtomicU64,
    pub previous_not_found: AtomicU64,
    pub empty_block: AtomicU64,
    pub invalid_coinbase: AtomicU64,
    pub signature_failure: AtomicU64,
    pub utxo_not_found: AtomicU64,
    pub utxo_ownership_failure: AtomicU64,
    pub duplicate_input: AtomicU64,
    pub insufficient_fee: AtomicU64,
    pub checkpoint_violation: AtomicU64,
    pub security_constraint: AtomicU64,
    pub other: AtomicU64,
}

impl ValidationStats {
    pub fn new() -> Self {
        Self {
            hash_mismatch: AtomicU64::new(0),
            invalid_pow: AtomicU64::new(0),
            difficulty_out_of_range: AtomicU64::new(0),
            merkle_root_mismatch: AtomicU64::new(0),
            timestamp_too_old: AtomicU64::new(0),
            timestamp_too_future: AtomicU64::new(0),
            previous_not_found: AtomicU64::new(0),
            empty_block: AtomicU64::new(0),
            invalid_coinbase: AtomicU64::new(0),
            signature_failure: AtomicU64::new(0),
            utxo_not_found: AtomicU64::new(0),
            utxo_ownership_failure: AtomicU64::new(0),
            duplicate_input: AtomicU64::new(0),
            insufficient_fee: AtomicU64::new(0),
            checkpoint_violation: AtomicU64::new(0),
            security_constraint: AtomicU64::new(0),
            other: AtomicU64::new(0),
        }
    }

    pub fn increment(&self, reason: BlockFailureReason) {
        let counter = match reason {
            BlockFailureReason::HashMismatch => &self.hash_mismatch,
            BlockFailureReason::InvalidPoW => &self.invalid_pow,
            BlockFailureReason::DifficultyOutOfRange => &self.difficulty_out_of_range,
            BlockFailureReason::MerkleRootMismatch => &self.merkle_root_mismatch,
            BlockFailureReason::TimestampTooOld => &self.timestamp_too_old,
            BlockFailureReason::TimestampTooFuture => &self.timestamp_too_future,
            BlockFailureReason::PreviousNotFound => &self.previous_not_found,
            BlockFailureReason::EmptyBlock => &self.empty_block,
            BlockFailureReason::InvalidCoinbase => &self.invalid_coinbase,
            BlockFailureReason::SignatureFailure => &self.signature_failure,
            BlockFailureReason::UtxoNotFound => &self.utxo_not_found,
            BlockFailureReason::UtxoOwnershipFailure => &self.utxo_ownership_failure,
            BlockFailureReason::DuplicateInput => &self.duplicate_input,
            BlockFailureReason::InsufficientFee => &self.insufficient_fee,
            BlockFailureReason::CheckpointViolation => &self.checkpoint_violation,
            BlockFailureReason::SecurityConstraint => &self.security_constraint,
            BlockFailureReason::Other => &self.other,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> Vec<(String, u64)> {
        vec![
            (
                "hash_mismatch".to_string(),
                self.hash_mismatch.load(Ordering::Relaxed),
            ),
            (
                "invalid_pow".to_string(),
                self.invalid_pow.load(Ordering::Relaxed),
            ),
            (
                "difficulty_out_of_range".to_string(),
                self.difficulty_out_of_range.load(Ordering::Relaxed),
            ),
            (
                "merkle_root_mismatch".to_string(),
                self.merkle_root_mismatch.load(Ordering::Relaxed),
            ),
            (
                "timestamp_too_old".to_string(),
                self.timestamp_too_old.load(Ordering::Relaxed),
            ),
            (
                "timestamp_too_future".to_string(),
                self.timestamp_too_future.load(Ordering::Relaxed),
            ),
            (
                "previous_not_found".to_string(),
                self.previous_not_found.load(Ordering::Relaxed),
            ),
            (
                "empty_block".to_string(),
                self.empty_block.load(Ordering::Relaxed),
            ),
            (
                "invalid_coinbase".to_string(),
                self.invalid_coinbase.load(Ordering::Relaxed),
            ),
            (
                "signature_failure".to_string(),
                self.signature_failure.load(Ordering::Relaxed),
            ),
            (
                "utxo_not_found".to_string(),
                self.utxo_not_found.load(Ordering::Relaxed),
            ),
            (
                "utxo_ownership_failure".to_string(),
                self.utxo_ownership_failure.load(Ordering::Relaxed),
            ),
            (
                "duplicate_input".to_string(),
                self.duplicate_input.load(Ordering::Relaxed),
            ),
            (
                "insufficient_fee".to_string(),
                self.insufficient_fee.load(Ordering::Relaxed),
            ),
            (
                "checkpoint_violation".to_string(),
                self.checkpoint_violation.load(Ordering::Relaxed),
            ),
            (
                "security_constraint".to_string(),
                self.security_constraint.load(Ordering::Relaxed),
            ),
            ("other".to_string(), self.other.load(Ordering::Relaxed)),
        ]
    }
}

lazy_static::lazy_static! {
    pub static ref VALIDATION_STATS: ValidationStats = ValidationStats::new();
}

/// Validate transaction security constraints
pub fn validate_transaction_security(tx: &Transaction, block_timestamp: i64) -> Result<()> {
    // 1. Check transaction size (prevent DoS)
    let tx_bytes = bincode::encode_to_vec(tx, crate::blockchain::BINCODE_CONFIG.clone())
        .map_err(|e| anyhow!("failed to serialize tx: {}", e))?;

    if tx_bytes.len() > MAX_TX_SIZE {
        return Err(anyhow!(
            "transaction too large: {} bytes (max {})",
            tx_bytes.len(),
            MAX_TX_SIZE
        ));
    }

    // 2. Check input/output count (prevent resource exhaustion)
    if tx.inputs.len() > MAX_TX_INPUTS {
        return Err(anyhow!(
            "too many inputs: {} (max {})",
            tx.inputs.len(),
            MAX_TX_INPUTS
        ));
    }

    if tx.outputs.len() > MAX_TX_OUTPUTS {
        return Err(anyhow!(
            "too many outputs: {} (max {})",
            tx.outputs.len(),
            MAX_TX_OUTPUTS
        ));
    }

    // 3. Timestamp validation (prevent future/old transactions)
    let current_time = chrono::Utc::now().timestamp();

    if tx.timestamp > current_time + MAX_FUTURE_TIMESTAMP {
        return Err(anyhow!(
            "transaction timestamp too far in future: {} > {}",
            tx.timestamp,
            current_time + MAX_FUTURE_TIMESTAMP
        ));
    }

    // Transaction shouldn't be newer than containing block
    if tx.timestamp > block_timestamp {
        return Err(anyhow!(
            "transaction timestamp ({}) exceeds block timestamp ({})",
            tx.timestamp,
            block_timestamp
        ));
    }

    // 4. Validate outputs are not dust (except coinbase)
    if !tx.inputs.is_empty() {
        for (idx, out) in tx.outputs.iter().enumerate() {
            if out.amount() < U256::from(MIN_OUTPUT_VALUE) {
                return Err(anyhow!(
                    "output {} is dust: {} (minimum {})",
                    idx,
                    out.amount(),
                    MIN_OUTPUT_VALUE
                ));
            }
        }
    }

    // 5. Validate no empty addresses
    for (idx, out) in tx.outputs.iter().enumerate() {
        if out.to.is_empty() {
            return Err(anyhow!("output {} has empty address", idx));
        }
    }

    Ok(())
}

/// Validate block security constraints
pub fn validate_block_security(block: &Block) -> Result<()> {
    // 1. Block must have at least coinbase transaction
    if block.transactions.is_empty() {
        return Err(anyhow!("block has no transactions"));
    }

    // 2. Validate block timestamp
    let current_time = chrono::Utc::now().timestamp();

    if block.header.timestamp > current_time + MAX_FUTURE_TIMESTAMP {
        return Err(anyhow!(
            "block timestamp too far in future: {} > {}",
            block.header.timestamp,
            current_time + MAX_FUTURE_TIMESTAMP
        ));
    }

    // Prevent pre-genesis blocks
    if block.header.timestamp < GENESIS_TIMESTAMP {
        return Err(anyhow!(
            "block timestamp predates genesis: {} < {}",
            block.header.timestamp,
            GENESIS_TIMESTAMP
        ));
    }

    // 3. Coinbase must be first and only coinbase
    let coinbase = &block.transactions[0];
    if !coinbase.inputs.is_empty() {
        return Err(anyhow!("first transaction is not coinbase"));
    }

    for (idx, tx) in block.transactions.iter().enumerate().skip(1) {
        if tx.inputs.is_empty() {
            return Err(anyhow!("non-first transaction {} is coinbase-like", idx));
        }
    }

    Ok(())
}

/// Check if reorganization depth exceeds safety limit
/// This prevents deep chain reorganizations that could indicate a 51% attack
pub fn validate_reorg_depth(
    current_height: u64,
    fork_point_height: u64,
    max_depth: u64,
) -> Result<()> {
    let reorg_depth = current_height.saturating_sub(fork_point_height);

    if reorg_depth > max_depth {
        return Err(anyhow!(
            "🚨 REORG DEPTH EXCEEDED: attempted to reorganize {} blocks (max allowed: {}). This may indicate a 51% attack!",
            reorg_depth,
            max_depth
        ));
    }

    if reorg_depth > max_depth / 2 {
        log::warn!(
            "⚠️  Large reorganization detected: {} blocks (limit: {}). Monitor for potential attack.",
            reorg_depth,
            max_depth
        );
    }

    Ok(())
}

/// Rate limiter for preventing spam from single address
pub struct AddressRateLimiter {
    /// address -> (count, window_start)
    limits: std::collections::HashMap<String, (u32, i64)>,
    max_per_window: u32,
    window_seconds: i64,
}

impl AddressRateLimiter {
    pub fn new(max_per_window: u32, window_seconds: i64) -> Self {
        Self {
            limits: std::collections::HashMap::new(),
            max_per_window,
            window_seconds,
        }
    }

    /// Check if address is allowed to submit transaction
    pub fn check_and_update(&mut self, address: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let normalized = address.to_lowercase();

        let entry = self.limits.entry(normalized.clone()).or_insert((0, now));

        // Reset window if expired
        if now - entry.1 >= self.window_seconds {
            *entry = (1, now);
            return Ok(());
        }

        // Check limit
        if entry.0 >= self.max_per_window {
            return Err(anyhow!(
                "rate limit exceeded for {}: {} tx in {} seconds",
                address,
                entry.0,
                self.window_seconds
            ));
        }

        entry.0 += 1;
        Ok(())
    }

    /// Clean up old entries (call periodically)
    pub fn cleanup(&mut self) {
        let now = chrono::Utc::now().timestamp();
        self.limits
            .retain(|_, (_, window_start)| now - *window_start < self.window_seconds * 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter() {
        let mut limiter = AddressRateLimiter::new(3, 60); // 3 per minute

        let addr = "0x1234567890abcdef";

        // Should allow first 3
        assert!(limiter.check_and_update(addr).is_ok());
        assert!(limiter.check_and_update(addr).is_ok());
        assert!(limiter.check_and_update(addr).is_ok());

        // Should reject 4th
        assert!(limiter.check_and_update(addr).is_err());
    }

    #[test]
    fn test_transaction_size_limit() {
        use crate::transaction::{Transaction, TransactionInput, TransactionOutput};

        // Create huge transaction
        let mut inputs = vec![];
        for i in 0..2000 {
            inputs.push(TransactionInput {
                txid: format!("{:064x}", i),
                vout: 0,
                pubkey: "0".repeat(130),
                signature: Some("0".repeat(128)),
            });
        }

        let tx = Transaction {
            txid: "test".to_string(),
            eth_hash: "0x0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            inputs,
            outputs: vec![TransactionOutput::new("addr".to_string(), U256::from(100))],
            timestamp: 0,
        };

        let result = validate_transaction_security(&tx, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_reorg_depth_validation() {
        // Safe reorganization
        assert!(validate_reorg_depth(110, 100, 100).is_ok());

        // At limit
        assert!(validate_reorg_depth(200, 100, 100).is_ok());

        // Exceeds limit - should reject
        assert!(validate_reorg_depth(210, 100, 100).is_err());

        // No reorg needed
        assert!(validate_reorg_depth(100, 100, 100).is_ok());
    }

    #[test]
    fn test_genesis_timestamp_validation() {
        use crate::block::{Block, BlockHeader};
        use crate::transaction::Transaction;

        let mut block = Block {
            header: BlockHeader {
                index: 1,
                previous_hash: "0".repeat(64),
                merkle_root: "0".repeat(64),
                timestamp: GENESIS_TIMESTAMP - 1000, // Before genesis
                nonce: 0,
                difficulty: 1,
            },
            transactions: vec![Transaction::coinbase("addr", U256::from(50))],
            hash: "0".repeat(64),
        };

        assert!(validate_block_security(&block).is_err());

        // After genesis should be OK
        block.header.timestamp = GENESIS_TIMESTAMP + 1000;
        assert!(validate_block_security(&block).is_ok());
    }
}
