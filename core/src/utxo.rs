use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Encode, Decode, Debug, Clone, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub to: String,
    pub amount: u64,
}
