use crate::block::Block;
use crate::transaction::Transaction;
/// Security validation utilities for blockchain operations
use anyhow::{Result, anyhow};
use primitive_types::U256;

/// Security constants
pub const MAX_TX_SIZE: usize = 100_000; // 100KB max transaction size
pub const MIN_OUTPUT_VALUE: u64 = 100; // Minimum output to prevent dust
pub const MAX_TX_INPUTS: usize = 1000; // Prevent huge transactions
pub const MAX_TX_OUTPUTS: usize = 1000;
pub const MAX_FUTURE_TIMESTAMP: i64 = 7200; // 2 hours tolerance

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
            inputs,
            outputs: vec![TransactionOutput::new("addr".to_string(), U256::from(100))],
            timestamp: 0,
        };

        let result = validate_transaction_security(&tx, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }
}
