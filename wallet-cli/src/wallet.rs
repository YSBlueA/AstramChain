use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use rand::rngs::OsRng;
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use hex;
use rand::RngCore;
use bs58;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub nonce: u64,
}

impl Transaction {
    pub fn new(from: String, to: String, amount: u64, nonce: u64) -> Self {
        Self { from, to, amount, nonce }
    }

    pub fn hash(&self) -> Vec<u8> {
        let data = serde_json::to_vec(self).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().to_vec()
    }
}

pub struct Wallet {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub address: String,
}

impl Wallet {
    /// 새 지갑 생성
    pub fn new() -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::from_bytes(&{
            let mut sk_bytes = [0u8; 32];
            csprng.fill_bytes(&mut sk_bytes);
            sk_bytes
        });
        let verifying_key = signing_key.verifying_key();
        let address = Self::address_from_public(&verifying_key);
        Self { signing_key, verifying_key, address }
    }

    /// 개인키(hex)로 복원
    pub fn from_private_key_hex(hex_str: &str) -> Self {
        let secret_bytes = hex::decode(hex_str).unwrap();
        let signing_key = SigningKey::from_bytes(&secret_bytes.try_into().unwrap());
        let verifying_key = signing_key.verifying_key();
        let address = Self::address_from_public(&verifying_key);
        Self { signing_key, verifying_key, address }
    }

    fn address_from_public(pubkey: &VerifyingKey) -> String {
        let mut hasher = Sha256::new();
        hasher.update(pubkey.as_bytes());
        let result = hasher.finalize();
        hex::encode(&result[0..20])
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    pub fn sign_transaction(&self, tx: &Transaction) -> Signature {
        let hash = tx.hash();
        self.signing_key.sign(&hash)
    }

    /// 서명 검증
    pub fn verify_signature(&self, tx: &Transaction, sig: &Signature) -> bool {
        let hash = tx.hash();
        self.verifying_key.verify(&hash, sig).is_ok()
    }

    /// 개인키를 Base58로 변환 (Solana/Phantom 호환)
    pub fn secret_base58(&self) -> String {
        let secret_bytes = self.signing_key.to_bytes(); // 32바이트
        // Solana 형식: 32바이트 개인키 + 32바이트 공개키 = 64바이트
        let mut full_bytes = [0u8; 64];
        full_bytes[..32].copy_from_slice(&secret_bytes);
        full_bytes[32..].copy_from_slice(self.verifying_key.as_bytes());
        bs58::encode(full_bytes).into_string()
    }

    /// Base58 문자열로 Wallet 복원
    pub fn from_base58(base58_str: &str) -> Self {
        let full_bytes = bs58::decode(base58_str).into_vec().expect("Invalid Base58");
        assert_eq!(full_bytes.len(), 64, "Invalid key length");

        let signing_bytes: [u8; 32] = full_bytes[..32].try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&signing_bytes);
        let verifying_key = signing_key.verifying_key();
        let address = Self::address_from_public(&verifying_key);

        Self { signing_key, verifying_key, address }
    }
}
