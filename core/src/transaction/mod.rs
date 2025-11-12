use anyhow::Result;
use bincode::error::EncodeError;
use bincode::{Decode, Encode, config};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hex;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

pub static BINCODE_CONFIG: Lazy<config::Configuration> = Lazy::new(|| config::standard());

/// Input: previous txid and vout index
#[derive(Encode, Decode, Debug, Clone)]
pub struct TransactionInput {
    pub txid: String, // hex
    pub vout: u32,
    pub pubkey: String,            // hex of public key (ed25519)
    pub signature: Option<String>, // hex of signature
}

/// Output: recipient address (assumed to be a simple pubkey hash) + amount
#[derive(Encode, Decode, Debug, Clone)]
pub struct TransactionOutput {
    pub to: String,
    pub amount: u64,
}

/// Transaction: inputs / outputs / timestamp / txid
#[derive(Encode, Decode, Debug, Clone)]
pub struct Transaction {
    pub txid: String, // hex
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub timestamp: i64,
}

impl Transaction {
    pub fn coinbase(to: &str, amount: u64) -> Self {
        let outputs = vec![TransactionOutput {
            to: to.to_string(),
            amount,
        }];
        let tx = Transaction {
            txid: "".to_string(),
            inputs: vec![],
            outputs,
            timestamp: chrono::Utc::now().timestamp(),
        };
        tx.with_txid()
    }

    pub fn serialize_for_hash(&self) -> Result<Vec<u8>, EncodeError> {
        Ok(bincode::encode_to_vec(
            &(&self.inputs, &self.outputs, &self.timestamp),
            *BINCODE_CONFIG,
        )?)
    }

    pub fn compute_txid(&self) -> Result<String, anyhow::Error> {
        let bytes = self.serialize_for_hash()?;
        let h1 = Sha256::digest(&bytes);
        let h2 = Sha256::digest(&h1);
        Ok(hex::encode(h2))
    }

    pub fn with_txid(mut self) -> Self {
        if let Ok(txid) = self.compute_txid() {
            self.txid = txid;
        }
        self
    }

    /// sign inputs (v2 style: SigningKey)
    pub fn sign(&mut self, signing_key: &SigningKey) -> Result<(), anyhow::Error> {
        let msg = self.serialize_for_hash()?;
        let sig: Signature = signing_key.sign(&msg);
        let sig_hex = hex::encode(sig.to_bytes());
        let pk_hex = hex::encode(signing_key.verifying_key().to_bytes());

        for inp in &mut self.inputs {
            inp.signature = Some(sig_hex.clone());
            inp.pubkey = pk_hex.clone();
        }
        Ok(())
    }

    /// verify signatures (v2 style: VerifyingKey)
    pub fn verify_signatures(&self) -> Result<bool, anyhow::Error> {
        if self.inputs.is_empty() {
            return Ok(true);
        }
        let msg = self.serialize_for_hash()?;
        for inp in &self.inputs {
            let sig_hex = match &inp.signature {
                Some(s) => s,
                None => return Ok(false),
            };
            let sig_bytes = hex::decode(sig_hex)?;
            let sig: Signature = Signature::try_from(&sig_bytes[..])
                .map_err(|e| anyhow::anyhow!("invalid signature: {}", e))?;

            let pk_bytes = hex::decode(&inp.pubkey)?;
            let pk: VerifyingKey = VerifyingKey::try_from(&pk_bytes[..])
                .map_err(|e| anyhow::anyhow!("invalid public key: {}", e))?;

            pk.verify(&msg, &sig)?;
        }
        Ok(true)
    }
}

#[test]
fn sign_and_verify() {
    use ed25519_dalek::{SECRET_KEY_LENGTH, SigningKey};
    use rand::TryRngCore;
    use rand::rngs::OsRng;
    use std::convert::TryFrom;

    let mut csprng = OsRng {};
    let mut secret_bytes = [0u8; SECRET_KEY_LENGTH];

    // ✅ try_fill_bytes 사용, 반드시 Result 처리
    csprng.try_fill_bytes(&mut secret_bytes).unwrap();

    let signing_key = SigningKey::try_from(&secret_bytes[..]).unwrap();

    let mut tx = Transaction::coinbase("addr", 50);
    assert!(tx.verify_signatures().unwrap());

    let inp = TransactionInput {
        txid: "00".repeat(32),
        vout: 0,
        pubkey: "".to_string(),
        signature: None,
    };
    let out = TransactionOutput {
        to: "alice".to_string(),
        amount: 10,
    };
    let mut tx2 = Transaction {
        txid: "".to_string(),
        inputs: vec![inp],
        outputs: vec![out],
        timestamp: chrono::Utc::now().timestamp(),
    };
    tx2.sign(&signing_key).unwrap();
    assert!(tx2.verify_signatures().unwrap());
}
