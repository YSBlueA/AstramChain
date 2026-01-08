use crate::wallet::Wallet;
use chrono::Utc;
use netcoin_config::config::Config;
use netcoin_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize, de};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Create a new wallet
    Generate,

    /// Check the balance of a specific address
    Balance { address: String },

    /// Create, sign, and broadcast a transaction to the network
    Send { to: String, amount: u64 },

    /// Manage CLI configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },
}

#[derive(clap::Subcommand)]
pub enum ConfigCommands {
    View,
    Set { key: String, value: String },
    Init,
}

#[derive(Serialize, Deserialize)]
struct WalletJson {
    secret_key: String,
    address: String,
}

fn get_wallet_path() -> PathBuf {
    let cfg = Config::load();
    cfg.wallet_path_resolved()
}

fn save_wallet_base58(wallet: Wallet, path: &str) -> std::io::Result<()> {
    // Create parent directories if they don't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

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

fn load_wallet() -> Wallet {
    let path = get_wallet_path();
    let data = fs::read_to_string(&path).expect("Failed to read wallet file");
    let wallet_json: WalletJson = serde_json::from_str(&data).expect("Failed to parse wallet JSON");

    println!("✅ Wallet loaded: {}", wallet_json.address);
    println!("✅ Private key: {}", wallet_json.secret_key);

    Wallet::from_base58(&wallet_json.secret_key)
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

pub fn send_transaction(to: &str, amount: u64) {
    let cfg = Config::load();
    let wallet = load_wallet();
    let client = Client::new();

    let url = format!("{}/address/{}/utxos", cfg.node_rpc_url, wallet.address);
    let utxos: Vec<Value> = match client.get(&url).send() {
        Ok(res) => match res.json() {
            Ok(v) => v,
            Err(e) => {
                println!("❌ Failed to parse UTXOs JSON: {}", e);
                return;
            }
        },
        Err(e) => {
            println!("❌ Query failed: {}", e);
            return;
        }
    };

    if utxos.is_empty() {
        println!("❌ No UTXOs available for address {}", wallet.address);
        return;
    }

    let mut selected_inputs = vec![];
    let mut input_sum: u64 = 0;

    for u in &utxos {
        let txid = u["txid"].as_str().unwrap().to_string();
        let vout = u["vout"].as_u64().unwrap() as u32;
        let amt = u["amount"].as_u64().unwrap();
        selected_inputs.push(TransactionInput {
            txid,
            vout,
            pubkey: wallet.address.clone(),
            signature: None,
        });
        input_sum += amt;
        if input_sum >= amount {
            break;
        }
    }

    if input_sum < amount {
        println!(
            "❌ Insufficient balance: have {}, need {}",
            input_sum, amount
        );
        return;
    }

    let mut outputs = vec![TransactionOutput {
        to: to.to_string(),
        amount,
    }];

    let change = input_sum - amount;
    if change > 0 {
        outputs.push(TransactionOutput {
            to: wallet.address.clone(),
            amount: change,
        });
    }

    let mut tx = Transaction {
        txid: "".to_string(),
        inputs: selected_inputs,
        outputs,
        timestamp: chrono::Utc::now().timestamp(),
    };

    // 5️⃣ 서명
    if let Err(e) = tx.sign(&wallet.signing_key) {
        println!("❌ Failed to sign transaction: {}", e);
        return;
    }

    tx.verify_signatures()
        .expect("Signature verification failed after signing");

    // 6️⃣ txid 채우기
    tx = tx.with_txid();

    println!("✅ Transaction created. txid: {}", tx.txid);
    println!(
        "Signature: {}",
        tx.inputs
            .get(0)
            .and_then(|i| i.signature.as_deref())
            .unwrap_or("no signature")
    );

    // 7️⃣ Serialize
    let body = match bincode::encode_to_vec(&tx, *BINCODE_CONFIG) {
        Ok(b) => b,
        Err(e) => {
            println!("❌ Failed to serialize transaction: {}", e);
            return;
        }
    };

    // 8️⃣ POST /tx
    match client
        .post(format!("{}/tx", cfg.node_rpc_url))
        .body(body)
        .header("Content-Type", "application/octet-stream")
        .send()
    {
        Ok(mut response) => {
            let status = response.status();
            let mut text = String::new();
            response.read_to_string(&mut text).unwrap_or_default();
            if status.is_success() {
                println!("🚀 Transaction broadcast completed!");
            } else {
                println!("❌ Transaction failed!");
                println!("Status: {}", status);
                println!("Response body: {}", text);
            }
        }
        Err(e) => println!("❌ Transaction failed (network/reqwest error): {}", e),
    }
}
