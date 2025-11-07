use crate::wallet::{Wallet, Transaction};
use reqwest::blocking::Client;
use serde_json::Value;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use netcoin_config::config::Config;


#[derive(clap::Subcommand)]
pub enum Commands {
    /// Create a new wallet
    Generate,

    /// Check the balance of a specific address
    Balance { address: String },

    /// Create, sign, and broadcast a transaction to the network
    Send {
        from: String,
        to: String,
        amount: u64,
        private_key: String,
    },

    /// Manage CLI configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },
}

#[derive(clap::Subcommand)]
pub enum ConfigCommands {
    View,
    Set {
        key: String,
        value: String,
    },
    Init,
}

#[derive(Serialize)]
struct WalletJson {
    secret_key: String,
    address: String,
}

fn get_wallet_path() -> PathBuf {
    let cfg = Config::load();
    let expanded = shellexpand::tilde(&cfg.wallet_path);
    PathBuf::from(expanded.to_string())
}

fn save_wallet_base58(wallet: Wallet, path: &str) -> std::io::Result<()> {
    let wallet_json = WalletJson {
        secret_key: wallet.secret_base58(),
        address: wallet.address.clone(),
    };
    let data = serde_json::to_string_pretty(&wallet_json).unwrap();
    fs::write(path, data)
}

pub fn generate_wallet() {
    let wallet = Wallet::new();
    println!("✅ New wallet created successfully!");
    println!("address: {}", wallet.address);
    println!("Private key(hex): {}", wallet.secret_hex());

    let path = get_wallet_path();
    save_wallet_base58(wallet, path.to_str().unwrap()).expect("Failed to save wallet");
}

pub fn get_balance(address: &str) {
    let cfg = Config::load();
    let url = format!("{}/address/{}/balance", cfg.node_rpc_url, address);
    match Client::new().get(&url).send() {
        Ok(res) => {
            let json: Value = res.json().unwrap();
            println!("💰 balance: {}", json["balance"]);
        }
        Err(e) => println!("❌ Query failed: {}", e),
    }
}

pub fn send_transaction(from: &str, to: &str, amount: u64, private_key: &str) {
    let cfg = Config::load();
    let wallet = Wallet::from_private_key_hex(private_key);
    let tx = Transaction::new(from.to_string(), to.to_string(), amount, 0);
    let signature = wallet.sign_transaction(&tx);

    let client = Client::new();
    let res = client
        .post(format!("{}/tx/send", cfg.node_rpc_url))
        .json(&serde_json::json!({
            "from": from,
            "to": to,
            "amount": amount,
            "signature": hex::encode(signature.to_bytes())
        }))
        .send();

    match res {
        Ok(_) => println!("🚀 Transaction broadcast completed!"),
        Err(e) => println!("❌ Transaction failed: {}", e),
    }
}
