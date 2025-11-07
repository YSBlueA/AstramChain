use serde::{Serialize, Deserialize};
use ed25519_dalek::{Keypair, Signature, PublicKey, Verifier, Signer};
use rand::rngs::OsRng;
use bincode;
use sha2::{Sha256, Digest};
use hex;
use anyhow::Result;

/// Input: previous txid and vout index
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionInput {
    pub txid: String, // hex
    pub vout: u32,
    pub pubkey: String, // hex of public key (ed25519)
    pub signature: Option<String>, // hex of signature
}

/// Output: recipient address (assumed to be a simple pubkey hash) + amount
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionOutput {
    pub to: String,
    pub amount: u64,
}

/// Transaction: inputs / outputs / timestamp / txid
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub txid: String, // hex
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub timestamp: i64,
}

impl Transaction {
    /// Create coinbase transaction (inputs are empty)
    pub fn coinbase(to: &str, amount: u64) -> Self {
        let outputs = vec![TransactionOutput{ to: to.to_string(), amount }];
        let tx = Transaction {
            txid: "".to_string(),
            inputs: vec![],
            outputs,
            timestamp: chrono::Utc::now().timestamp(),
        };
        tx.with_txid()
    }

    /// Deterministic serialization (for signing / hashing)
    pub fn serialize_for_hash(&self) -> Result<Vec<u8>, bincode::Error> {
        Ok(bincode::serialize(&(
            &self.inputs,
            &self.outputs,
            &self.timestamp
        ))?)
    }

    /// compute txid: sha256d(serialized_for_hash)
    pub fn compute_txid(&self) -> Result<String, anyhow::Error> {
        let bytes = self.serialize_for_hash()?;
        let h1 = Sha256::digest(&bytes);
        let h2 = Sha256::digest(&h1);
        Ok(hex::encode(h2))
    }

    /// returns a copy with txid set based on contents
    pub fn with_txid(mut self) -> Self {
        if let Ok(txid) = self.compute_txid() {
            self.txid = txid;
        }
        self
    }

    /// sign inputs (for simplicity: same key signs all inputs)
    pub fn sign(&mut self, keypair: &Keypair) -> Result<(), anyhow::Error> {
        // sign serialized_for_hash and attach signature + pubkey to each input
        let msg = self.serialize_for_hash()?;
        let sig: Signature = keypair.sign(&msg);
        let sig_hex = hex::encode(sig.to_bytes());
        let pk_hex = hex::encode(keypair.public.to_bytes());

        for inp in &mut self.inputs {
            inp.signature = Some(sig_hex.clone());
            inp.pubkey = pk_hex.clone();
        }
        // recompute txid after signing? ideally txid includes signature? Many designs exclude sig from txid.
        // We'll keep txid computed from inputs+outputs+timestamp (no signature) for simplicity.
        Ok(())
    }

    /// verify signatures for all inputs
    pub fn verify_signatures(&self) -> Result<bool, anyhow::Error> {
        // coinbase tx has no inputs/signatures
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
            let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes)?;
            let pk_bytes = hex::decode(&inp.pubkey)?;
            let pk = ed25519_dalek::PublicKey::from_bytes(&pk_bytes)?;
            pk.verify(&msg, &sig)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Keypair;
    use rand::rngs::OsRng;

    #[test]
    fn sign_and_verify() {
        let mut csprng = OsRng{};
        let kp: Keypair = Keypair::generate(&mut csprng);

        let mut tx = Transaction::coinbase("addr", 50);
        // coinbase has no inputs -> verify true
        assert!(tx.verify_signatures().unwrap());

        // make a simple tx with one input
        let inp = TransactionInput { txid: "00".repeat(32), vout: 0, pubkey: "".to_string(), signature: None };
        let out = TransactionOutput { to: "alice".to_string(), amount: 10 };
        let mut tx2 = Transaction { txid: "".to_string(), inputs: vec![inp], outputs: vec![out], timestamp: chrono::Utc::now().timestamp() };
        tx2.sign(&kp).unwrap();
        assert!(tx2.verify_signatures().unwrap());
    }
}
