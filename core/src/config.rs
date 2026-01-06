// Token economics and fee configuration for NTC blockchain

// ========== Token Definition ==========
/// 1 NTC in natoshi (smallest unit)
pub const NATOSHI_PER_NTC: u64 = 100_000_000;

/// Initial block reward: 8 NTC in natoshi
pub const INITIAL_BLOCK_REWARD: u64 = 8 * NATOSHI_PER_NTC;

/// Halving occurs every 210,000 blocks (~4 years at 10 min block time)
pub const HALVING_INTERVAL: u64 = 210_000;

/// Max supply: 42,000,000 NTC in natoshi
pub const MAX_SUPPLY: u64 = 42_000_000 * NATOSHI_PER_NTC;

// ========== Fee Model ==========
/// Minimum relay fee: 1 natoshi per byte
pub const MIN_RELAY_FEE_NAT_PER_BYTE: u64 = 1;

/// Default wallet fee: 2 natoshi per byte
pub const DEFAULT_WALLET_FEE_NAT_PER_BYTE: u64 = 2;

// ========== Helper Functions ==========

/// Calculate block reward for given height based on halving schedule
pub fn calculate_block_reward(block_height: u64) -> u64 {
    let halvings = (block_height / HALVING_INTERVAL) as u32;
    
    if halvings >= 33 {
        // After 33 halvings, reward reaches effectively 0 (supply cap reached)
        return 0;
    }
    
    INITIAL_BLOCK_REWARD >> halvings
}

/// Calculate minimum fee for transaction in natoshi based on transaction size
pub fn calculate_min_fee(tx_size_bytes: usize) -> u64 {
    (tx_size_bytes as u64) * MIN_RELAY_FEE_NAT_PER_BYTE
}

/// Calculate default wallet fee for transaction in natoshi based on transaction size
pub fn calculate_default_fee(tx_size_bytes: usize) -> u64 {
    (tx_size_bytes as u64) * DEFAULT_WALLET_FEE_NAT_PER_BYTE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_supply() {
        let reward = calculate_block_reward(0);
        assert_eq!(reward, 8 * NATOSHI_PER_NTC);
    }

    #[test]
    fn test_first_halving() {
        let reward_before = calculate_block_reward(HALVING_INTERVAL - 1);
        let reward_after = calculate_block_reward(HALVING_INTERVAL);
        
        assert_eq!(reward_before, 8 * NATOSHI_PER_NTC);
        assert_eq!(reward_after, 4 * NATOSHI_PER_NTC);
    }

    #[test]
    fn test_fee_calculation() {
        let fee = calculate_default_fee(250); // standard tx size
        assert_eq!(fee, 250 * 2); // 2 nat/byte
    }
}
