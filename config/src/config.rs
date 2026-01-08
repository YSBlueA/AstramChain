use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub wallet_path: String,
    pub node_rpc_url: String,
    pub data_dir: String,
}

impl Config {
    fn expand_path(path: &str) -> PathBuf {
        let expanded = shellexpand::tilde(path);
        PathBuf::from(expanded.into_owned())
    }

    /// Compute the default wallet path depending on the target OS.
    fn default_wallet_path() -> String {
        let home = dirs::home_dir().expect("Cannot find home directory");

        // Use a Windows-friendly folder when building on Windows to avoid tilde expansion issues.
        if cfg!(target_os = "windows") {
            let base = dirs::data_dir().unwrap_or(home).join("Netcoin");
            return base.join("wallet.json").to_string_lossy().into_owned();
        }

        home.join(".netcoin")
            .join("wallet.json")
            .to_string_lossy()
            .into_owned()
    }

    /// Compute the default data directory depending on the target OS.
    fn default_data_dir() -> String {
        let home = dirs::home_dir().expect("Cannot find home directory");

        if cfg!(target_os = "windows") {
            let base = dirs::data_dir().unwrap_or(home).join("Netcoin");
            return base.join("data").to_string_lossy().into_owned();
        }

        home.join(".netcoin")
            .join("data")
            .to_string_lossy()
            .into_owned()
    }

    pub fn default_path() -> PathBuf {
        let home = dirs::home_dir().expect("Cannot find home directory");
        home.join(".netcoin/config.json")
    }

    /// Wallet path with tilde expansion applied.
    pub fn wallet_path_resolved(&self) -> PathBuf {
        Self::expand_path(&self.wallet_path)
    }

    /// Data directory with tilde expansion applied.
    pub fn data_dir_resolved(&self) -> PathBuf {
        Self::expand_path(&self.data_dir)
    }

    pub fn load() -> Self {
        let path = Self::default_path();
        if !path.exists() {
            println!(
                "⚠️ Configuration file not found. Creating default configuration.: {:?}",
                path
            );
            let cfg = Self::default();
            cfg.save();
            return cfg;
        }
        let data = fs::read_to_string(&path).expect("Failed to read configuration file");
        serde_json::from_str(&data).expect("Configuration file format error")
    }

    pub fn save(&self) {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let json = serde_json::to_string_pretty(self).unwrap();
        fs::write(&path, json).unwrap();
    }

    pub fn set_value(&mut self, key: &str, value: &str) {
        match key {
            "wallet_path" => self.wallet_path = value.to_string(),
            "node_rpc_url" => self.node_rpc_url = value.to_string(),
            "data_dir" => self.data_dir = value.to_string(),
            _ => {
                println!("❌ Unknown configuration key: {}", key);
                return;
            }
        }
        self.save();
        println!("✅ {} = {} Set successfully.", key, value);
    }

    pub fn view(&self) {
        println!("{}", serde_json::to_string_pretty(self).unwrap());
    }

    pub fn init_default() {
        let cfg = Self::default();
        cfg.save();
        println!(
            "✅ Default configuration file has been created: {:?}",
            Self::default_path()
        );
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wallet_path: Self::default_wallet_path(),
            node_rpc_url: "http://127.0.0.1:8333".to_string(),
            data_dir: Self::default_data_dir(),
        }
    }
}
