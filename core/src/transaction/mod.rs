use anyhow::Result;
use bincode::error::EncodeError;
use bincode::{Decode, Encode, config};
use hex;
use once_cell::sync::Lazy;
use primitive_types::U256;
use sha2::{Digest, Sha256};

pub static BINCODE_CONFIG: Lazy<config::Configuration> = Lazy::new(|| config::standard());

/// Input: previous txid and vout index
#[derive(Encode, Decode, Debug, Clone)]
pub struct TransactionInput {
    pub txid: String, // hex
    pub vout: u32,
    pub pubkey: String,            // hex of public key (secp256k1, uncompressed)
    pub signature: Option<String>, // hex of signature (64 bytes compact)
}

/// Output: recipient address (assumed to be a simple pubkey hash) + amount
/// Amount is stored as [u64; 4] for bincode compatibility, represents U256
#[derive(Encode, Decode, Debug, Clone)]
pub struct TransactionOutput {
    pub to: String,
    amount_raw: [u64; 4], // U256 internal representation
}

impl TransactionOutput {
    pub fn new(to: String, amount: U256) -> Self {
        TransactionOutput {
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

/// Transaction: inputs / outputs / timestamp / txid
#[derive(Encode, Decode, Debug, Clone)]
pub struct Transaction {
    pub txid: String,     // UTXO transaction tracking (SHA256 double hash)
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub timestamp: i64,
}

impl Transaction {
    pub fn coinbase(to: &str, amount: U256) -> Self {
        let output = TransactionOutput::new(to.to_string(), amount);
        let tx = Transaction {
            txid: "".to_string(),
            inputs: vec![],
            outputs: vec![output],
            timestamp: chrono::Utc::now().timestamp(),
        };
        tx.with_hashes()
    }

    pub fn serialize_for_hash(&self) -> Result<Vec<u8>, EncodeError> {
        let inputs_for_hash: Vec<_> = self
            .inputs
            .iter()
            .map(|i| (i.txid.clone(), i.vout)) // omit pubkey
            .collect();

        Ok(bincode::encode_to_vec(
            &(&inputs_for_hash, &self.outputs, &self.timestamp),
            *BINCODE_CONFIG,
        )?)
    }

    /// Calculate txid for UTXO transaction tracking (Bitcoin style: SHA256 double hash)
    pub fn compute_txid(&self) -> Result<String, anyhow::Error> {
        let bytes = self.serialize_for_hash()?;
        let h1 = Sha256::digest(&bytes);
        let h2 = Sha256::digest(&h1);
        Ok(hex::encode(h2))
    }

    /// Set txid (recommended)
    pub fn with_hashes(mut self) -> Self {
        if let Ok(txid) = self.compute_txid() {
            self.txid = txid;
        }
        self
    }

    /// Legacy wrapper method (deprecated)
    #[deprecated(note = "Use with_hashes() instead")]
    pub fn with_txid(self) -> Self {
        self.with_hashes()
    }

    /// sign inputs using secp256k1
    pub fn sign(&mut self, secret_key: &crate::crypto::WalletKeypair) -> Result<(), anyhow::Error> {
        let tx_bytes = self.serialize_for_hash()?;
        let sig_bytes = secret_key.sign(&tx_bytes);

        let sig_hex = hex::encode(sig_bytes);
        let pk_hex = secret_key.public_hex();

        for inp in &mut self.inputs {
            inp.signature = Some(sig_hex.clone());
            inp.pubkey = pk_hex.clone();
        }
        Ok(())
    }

    /// verify signatures using secp256k1
    pub fn verify_signatures(&self) -> Result<bool, anyhow::Error> {
        if self.inputs.is_empty() {
            return Ok(true);
        }

        let tx_bytes = self.serialize_for_hash()?;

        for inp in &self.inputs {
            let sig_hex = inp
                .signature
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing signature"))?;

            // Standard Astram signature verification
            let sig_bytes = hex::decode(sig_hex)?;

            if !crate::crypto::verify_signature(&inp.pubkey, &tx_bytes, &sig_bytes) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[test]
fn sign_and_verify() {
    use crate::crypto::WalletKeypair;

    let keypair = WalletKeypair::new();

    let tx = Transaction::coinbase("addr", U256::from(50));
    assert!(tx.verify_signatures().unwrap());

    let inp = TransactionInput {
        txid: "00".repeat(32),
        vout: 0,
        pubkey: "".to_string(),
        signature: None,
    };
    let out = TransactionOutput::new("alice".to_string(), U256::from(10));
    let mut tx2 = Transaction {
        txid: "".to_string(),
        inputs: vec![inp],
        outputs: vec![out],
        timestamp: chrono::Utc::now().timestamp(),
    };
    tx2.sign(&keypair).unwrap();
    assert!(tx2.verify_signatures().unwrap());
}
