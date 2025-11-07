use serde::{Serialize, Deserialize};
use std::fs;

#[derive(Serialize, Deserialize, Debug)]
pub struct Wallet {
    pub address: String,
    pub private_key: String,
}

impl Wallet {
    pub fn load_from_file(path: &str) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }
}
