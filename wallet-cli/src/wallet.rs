use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use hex;
use sha2::{Digest, Sha256};
use bip39::{Mnemonic, Language};

pub struct Wallet {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub address: String,
    pub mnemonic: Option<String>,
}

impl Wallet {
    /// 새 지갑 생성 (Ed25519 + BIP39 24-word mnemonic)
    pub fn new() -> Self {
        let mut rng = OsRng;
        
        // Generate 24-word mnemonic (256 bits entropy)
        let mut entropy = [0u8; 32]; // 32 bytes = 256 bits for 24 words
        rand::Rng::fill(&mut rng, &mut entropy);
        
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .expect("Failed to generate mnemonic");
        
        Self::from_mnemonic_str(mnemonic.to_string().as_str())
    }
    
    /// Mnemonic 문자열로부터 지갑 생성 (크롬 지갑과 호환)
    pub fn from_mnemonic_str(mnemonic_str: &str) -> Self {
        let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
            .expect("Invalid mnemonic");
        
        // BIP39 seed 파생 (512 bits)
        let seed = mnemonic.to_seed("");
        
        // First 32 bytes as Ed25519 seed (크롬 지갑과 동일)
        let mut seed_bytes = [0u8; 32];
        seed_bytes.copy_from_slice(&seed[0..32]);
        
        let signing_key = SigningKey::from_bytes(&seed_bytes);
        let verifying_key = signing_key.verifying_key();
        let address = Self::address_from_public(&verifying_key);
        
        Self {
            signing_key,
            verifying_key,
            address,
            mnemonic: Some(mnemonic_str.to_string()),
        }
    }

    fn address_from_public(pubkey: &VerifyingKey) -> String {
        let pubkey_bytes = pubkey.to_bytes();
        let hash = Sha256::digest(&pubkey_bytes);
        // Use first 20 bytes for address (similar to Ethereum format)
        format!("0x{}", hex::encode(&hash[..20]))
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
    }
    
    /// 체크섬 주소 (0x 접두사 포함)
    pub fn checksummed_address(&self) -> String {
        to_checksum_address(&self.address)
    }

    /// 16진수 개인키로부터 복원 (기존 wallet.json 호환용)
    pub fn from_hex(hex_str: &str) -> Self {
        let secret_bytes = hex::decode(hex_str).expect("Invalid hex string");
        if secret_bytes.len() != 32 {
            panic!("Secret key must be 32 bytes");
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&secret_bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        let address = Self::address_from_public(&verifying_key);

        Self {
            signing_key,
            verifying_key,
            address,
            mnemonic: None, // hex로부터 복구 시 mnemonic 없음
        }
    }
}

fn to_checksum_address(address: &str) -> String {
    let address = address.trim_start_matches("0x").to_lowercase();
    let hash = hex::encode(Sha256::digest(address.as_bytes()));
    
    let mut result = String::from("0x");
    for (i, ch) in address.chars().enumerate() {
        if ch.is_numeric() {
            result.push(ch);
        } else {
            let hash_char = hash.chars().nth(i).unwrap();
            if hash_char >= '8' {
                result.push(ch.to_uppercase().next().unwrap());
            } else {
                result.push(ch);
            }
        }
    }
    result
}

