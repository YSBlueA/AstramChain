use chrono::{DateTime, Utc};
use primitive_types::U256;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: String,
    pub timestamp: DateTime<Utc>,
    pub transactions: usize,
    pub miner: String,
    pub difficulty: u32,
    pub nonce: u64,
    pub previous_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionInfo {
    pub hash: String,
    pub from: String,
    pub to: String,
    pub amount: U256, // 송금 금액
    pub fee: U256,    // 수수료
    pub total: U256,  // 총액 (amount + fee)
    pub timestamp: DateTime<Utc>,
    pub block_height: Option<u64>,
    pub status: String, // "confirmed", "pending"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressInfo {
    pub address: String,
    pub balance: U256,
    pub sent: U256,
    pub received: U256,
    pub transaction_count: usize,
    pub last_transaction: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainStats {
    pub total_blocks: u64,
    pub total_transactions: u64,
    pub total_volume: U256,
    pub average_block_time: f64,
    pub average_block_size: usize,
    pub current_difficulty: u32,
    pub network_hashrate: String,
}

pub struct AppState {
    pub cached_blocks: Vec<BlockInfo>,
    pub cached_transactions: Vec<TransactionInfo>,
    pub last_update: chrono::DateTime<Utc>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            cached_blocks: Vec::new(),
            cached_transactions: Vec::new(),
            last_update: Utc::now(),
        }
    }
}
