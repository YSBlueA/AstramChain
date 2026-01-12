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
/// Minimum relay fee: 1 natoshi per byte
pub const MIN_RELAY_FEE_NAT_PER_BYTE: U256 = U256([1, 0, 0, 0]);

/// Default wallet fee: 2 natoshi per byte
pub const DEFAULT_WALLET_FEE_NAT_PER_BYTE: U256 = U256([2, 0, 0, 0]);

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
pub fn calculate_min_fee(tx_size_bytes: usize) -> U256 {
    MIN_RELAY_FEE_NAT_PER_BYTE * U256::from(tx_size_bytes)
}

/// Calculate default wallet fee for transaction in natoshi based on transaction size
pub fn calculate_default_fee(tx_size_bytes: usize) -> U256 {
    DEFAULT_WALLET_FEE_NAT_PER_BYTE * U256::from(tx_size_bytes)
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
        let fee = calculate_default_fee(250); // standard tx size
        assert_eq!(fee, U256::from(500)); // 2 nat/byte * 250 bytes
    }
}
