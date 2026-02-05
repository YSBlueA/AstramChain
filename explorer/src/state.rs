use chrono::{DateTime, Utc};
use primitive_types::U256;
use serde::{Deserialize, Serialize, Serializer};

// U256을 hex 문자열로 직렬화하는 헬퍼
fn serialize_u256_as_hex<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let hex_string = format!("0x{:x}", value);
    serializer.serialize_str(&hex_string)
}

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
    /// Number of confirmations (blocks built on top of this block)
    /// 0 = unconfirmed, 1-5 = low confidence, 6+ = confirmed
    pub confirmations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionInfo {
    pub hash: String, // EVM hash (0x...) - 외부 노출용
    pub txid: String, // UTXO txid - 내부 추적용 (필요시에만 사용)
    pub from: String,
    pub to: String,
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub amount: U256, // 송금 금액
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub fee: U256, // 수수료
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub total: U256, // 총액 (amount + fee)
    pub timestamp: DateTime<Utc>,
    pub block_height: Option<u64>,
    pub status: String, // "confirmed", "pending"
    /// Number of confirmations (None if pending, Some(n) if in a block)
    /// 0 = just mined, 6+ = safe/confirmed
    pub confirmations: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressInfo {
    pub address: String,
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub balance: U256,
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub sent: U256,
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub received: U256,
    pub transaction_count: usize,
    pub last_transaction: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainStats {
    pub total_blocks: u64,
    pub total_transactions: u64,
    #[serde(serialize_with = "serialize_u256_as_hex")]
    pub total_volume: U256,
    pub average_block_time: f64,
    pub average_block_size: usize,
    pub current_difficulty: u32,
    pub network_hashrate: String,
}
