use bincode::{Decode, Encode, config::standard};
use std::fs;

#[derive(Encode, Decode, Debug)]
pub struct Wallet {
    pub address: String,
    pub private_key: String,
}

impl Wallet {
    pub fn load_from_file(path: &str) -> Option<Self> {
        let data = fs::read(path).ok()?;

        let (wallet, _): (Wallet, usize) = bincode::decode_from_slice(&data, standard()).ok()?;
        Some(wallet)
    }

    pub fn save_to_file(&self, path: &str) -> std::io::Result<()> {
        let bytes = bincode::encode_to_vec(self, standard()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("bincode encode error: {}", e),
            )
        })?;
        fs::write(path, bytes)
    }
}
