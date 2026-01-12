use bincode::{Decode, Encode};
use primitive_types::U256;
use serde::{Deserialize, Serialize};

/// UTXO with amount stored as [u64; 4] for bincode compatibility
#[derive(Encode, Decode, Debug, Clone, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub to: String,
    amount_raw: [u64; 4], // U256 internal representation
}

impl Utxo {
    pub fn new(txid: String, vout: u32, to: String, amount: U256) -> Self {
        Utxo {
            txid,
            vout,
            to,
            amount_raw: amount.0,
        }
    }

    pub fn amount(&self) -> U256 {
        U256(self.amount_raw)
    }

    pub fn set_amount(&mut self, amount: U256) {
        self.amount_raw = amount.0;
    }
}
