// Token economics and fee configuration for NTC blockchain
use primitive_types::U256;

// ========== Token Definition ==========
/// 1 NTC in natoshi (smallest unit) - 18 decimals like Ethereum
pub const NATOSHI_PER_NTC: U256 = U256([1_000_000_000_000_000_000, 0, 0, 0]);

/// Initial block reward: 8 NTC in natoshi
pub fn initial_block_reward() -> U256 {
    NATOSHI_PER_NTC * U256::from(8)
}

/// Halving occurs every 210,000 blocks (~4 years at 10 min block time)
pub const HALVING_INTERVAL: u64 = 210_000;

/// Max supply: 42,000,000 NTC in natoshi
pub fn max_supply() -> U256 {
    NATOSHI_PER_NTC * U256::from(42_000_000)
}

// ========== Fee Model ==========
// 🛡️ Anti-DDoS Fee Policy (Learned from Solana's $0.00025 fee DDoS attacks in 2021-2022)
// Low fees enable spam attacks: attackers sent 400k+ tx/sec, causing 17hr network downtime

/// Base minimum fee: 100,000 natoshi (0.0000000000001 NTC )
/// This prevents spam while remaining essentially free for users
pub const BASE_MIN_FEE: U256 = U256([100_000, 0, 0, 0]);

/// Additional fee per byte: 100 natoshi/byte
/// Penalizes large/complex transactions to prevent resource abuse
pub const MIN_RELAY_FEE_NAT_PER_BYTE: U256 = U256([100, 0, 0, 0]);

/// Default wallet fee per byte: 200 natoshi/byte (2x minimum for faster confirmation)
pub const DEFAULT_WALLET_FEE_NAT_PER_BYTE: U256 = U256([200, 0, 0, 0]);

// ========== Helper Functions ==========

/// Calculate block reward for given height based on halving schedule
pub fn calculate_block_reward(block_height: u64) -> U256 {
    let halvings = (block_height / HALVING_INTERVAL) as u32;

    if halvings >= 33 {
        // After 33 halvings, reward reaches effectively 0 (supply cap reached)
        return U256::zero();
    }

    initial_block_reward() >> halvings
}

/// Calculate minimum fee for transaction in natoshi based on transaction size
/// Formula: BASE_MIN_FEE + (size × MIN_RELAY_FEE_NAT_PER_BYTE)
/// Example: 250 bytes → 100,000 + (250 × 100) = 125,000 natoshi
pub fn calculate_min_fee(tx_size_bytes: usize) -> U256 {
    BASE_MIN_FEE + (MIN_RELAY_FEE_NAT_PER_BYTE * U256::from(tx_size_bytes))
}

/// Calculate default wallet fee for transaction in natoshi based on transaction size
/// Formula: BASE_MIN_FEE + (size × DEFAULT_WALLET_FEE_NAT_PER_BYTE)
/// Example: 250 bytes → 100,000 + (250 × 200) = 150,000 natoshi
pub fn calculate_default_fee(tx_size_bytes: usize) -> U256 {
    BASE_MIN_FEE + (DEFAULT_WALLET_FEE_NAT_PER_BYTE * U256::from(tx_size_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_supply() {
        let reward = calculate_block_reward(0);
        assert_eq!(reward, NATOSHI_PER_NTC * U256::from(8));
    }

    #[test]
    fn test_first_halving() {
        let reward_before = calculate_block_reward(HALVING_INTERVAL - 1);
        let reward_after = calculate_block_reward(HALVING_INTERVAL);

        assert_eq!(reward_before, NATOSHI_PER_NTC * U256::from(8));
        assert_eq!(reward_after, NATOSHI_PER_NTC * U256::from(4));
    }

    #[test]
    fn test_fee_calculation() {
        // Standard transaction: 250 bytes
        let min_fee = calculate_min_fee(250);
        let expected_min = U256::from(100_000) + U256::from(250 * 100); // BASE + (size × rate)
        assert_eq!(min_fee, expected_min); // 125,000 natoshi

        let default_fee = calculate_default_fee(250);
        let expected_default = U256::from(100_000) + U256::from(250 * 200);
        assert_eq!(default_fee, expected_default); // 150,000 natoshi
    }

    #[test]
    fn test_base_fee_prevents_spam() {
        // Even tiny transactions pay base fee
        let tiny_tx_fee = calculate_min_fee(100);
        assert!(tiny_tx_fee >= BASE_MIN_FEE);

        // Spam attack cost: 1,000,000 transactions
        let spam_cost = calculate_min_fee(250) * U256::from(1_000_000);
        // = 125,000,000,000 natoshi = 0.000000125 NTC
        // At $1/NTC: $0.000000125 × 1M = $125 total cost
        // This makes spam attacks expensive!
        assert!(spam_cost > U256::from(100_000_000_000u64));
    }
}
