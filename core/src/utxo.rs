use bincode::{Decode, Encode};

#[derive(Encode, Decode, Debug, Clone)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub to: String,
    pub amount: u64,
}
