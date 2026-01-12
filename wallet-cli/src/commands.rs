use crate::wallet::Wallet;
use netcoin_config::config::Config;
use netcoin_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use primitive_types::U256;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
// NTC unit constants (18 decimal places, same as Ethereum)
const NATOSHI_PER_NTC: u128 = 1_000_000_000_000_000_000; // 1 NTC = 10^18 natoshi

/// Convert NTC to natoshi (smallest unit) as U256
pub fn ntc_to_natoshi(ntc: f64) -> U256 {
    let natoshi = (ntc * NATOSHI_PER_NTC as f64) as u128;
    U256::from(natoshi)
}

/// Convert natoshi (U256) to NTC for display
pub fn natoshi_to_ntc(natoshi: U256) -> f64 {
    // Convert U256 to u128 (safe for reasonable amounts)
    let natoshi_u128 = natoshi.low_u128();
    natoshi_u128 as f64 / NATOSHI_PER_NTC as f64
}

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Create a new wallet (Ed25519)
    Generate,

    /// Create a new Ethereum-compatible wallet (secp256k1) for MetaMask
    GenerateEth,

    /// Check the balance of a specific address
    Balance { address: String },

    /// Create, sign, and broadcast a transaction to the network
    /// Amount should be specified in NTC (e.g., 1.5 for 1.5 NTC)
    Send {
        to: String,
        #[arg(help = "Amount in NTC (e.g., 1.5)")]
        amount: f64,
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

fn save_wallet_json(wallet: &Wallet, path: &str) -> std::io::Result<()> {
    // Create parent directories if they don't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

    let wallet_json = WalletJson {
        secret_key: wallet.secret_hex(),
        address: wallet.address.clone(),
    };
    let data = serde_json::to_string_pretty(&wallet_json).unwrap();
    fs::write(path, data)
}

pub fn generate_wallet() {
    let wallet = Wallet::new();
    println!("✅ New wallet created successfully!");
    println!("📍 Address: {}", wallet.address);
    println!("🔑 Private Key: {}", wallet.secret_hex());
    println!("📋 Public Key: {}", wallet.public_hex());
    println!("✨ Checksum Address: {}", wallet.checksummed_address());
    println!();
    println!("⚠️  IMPORTANT: Save your private key securely!");
    println!("   You can import this into MetaMask using the private key.");

    let path = get_wallet_path();
    save_wallet_json(&wallet, path.to_str().unwrap()).expect("Failed to save wallet");
}

pub fn generate_eth_wallet() {
    // Same as generate_wallet now, since all wallets are Ethereum-compatible
    generate_wallet();
    println!();
    println!("📖 To add NetCoin to MetaMask:");
    println!("   Network Name: NetCoin Localhost");
    println!("   RPC URL: http://127.0.0.1:8545");
    println!("   Chain ID: 8888");
    println!("   Currency Symbol: NTC");
}

fn load_wallet() -> Wallet {
    let path = get_wallet_path();
    let data = fs::read_to_string(&path).expect("Failed to read wallet file");
    let wallet_json: WalletJson = serde_json::from_str(&data).expect("Failed to parse wallet JSON");

    println!("✅ Wallet loaded: {}", wallet_json.address);
    println!("🔑 Private key: {}", wallet_json.secret_key);

    Wallet::from_hex(&wallet_json.secret_key)
}

pub fn get_balance(address: &str) {
    let cfg = Config::load();
    let url = format!("{}/address/{}/balance", cfg.node_rpc_url, address);
    match Client::new().get(&url).send() {
        Ok(res) => {
            let json: Value = res.json().unwrap();
            // Parse balance as hex string (0x...) or number
            let balance_natoshi = if let Some(s) = json["balance"].as_str() {
                if let Some(hex_str) = s.strip_prefix("0x") {
                    U256::from_str_radix(hex_str, 16).unwrap_or_else(|_| U256::zero())
                } else {
                    U256::from_dec_str(s).unwrap_or_else(|_| U256::zero())
                }
            } else {
                json["balance"]
                    .as_u64()
                    .map(U256::from)
                    .unwrap_or_else(U256::zero)
            };
            let balance_ntc = natoshi_to_ntc(balance_natoshi);
            println!("💰 Balance: {} NTC", balance_ntc);
        }
        Err(e) => println!("❌ Query failed: {}", e),
    }
}

pub fn send_transaction(to: &str, amount_natoshi: U256) {
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
    let mut input_sum = U256::zero();

    for u in &utxos {
        let txid = u["txid"].as_str().unwrap().to_string();
        let vout = u["vout"].as_u64().unwrap() as u32;
        // Parse amount as hex string (0x...) or number
        let amt = if let Some(s) = u["amount"].as_str() {
            if let Some(hex_str) = s.strip_prefix("0x") {
                U256::from_str_radix(hex_str, 16).unwrap_or_else(|_| U256::zero())
            } else {
                U256::from_dec_str(s).unwrap_or_else(|_| U256::zero())
            }
        } else {
            u["amount"]
                .as_u64()
                .map(U256::from)
                .unwrap_or_else(U256::zero)
        };
        selected_inputs.push(TransactionInput {
            txid,
            vout,
            pubkey: wallet.address.clone(),
            signature: None,
        });
        input_sum = input_sum + amt;
        if input_sum >= amount_natoshi {
            break;
        }
    }

    if input_sum < amount_natoshi {
        println!(
            "❌ Insufficient balance: have {} NTC, need {} NTC",
            natoshi_to_ntc(input_sum),
            natoshi_to_ntc(amount_natoshi)
        );
        return;
    }

    let mut outputs = vec![TransactionOutput::new(to.to_string(), amount_natoshi)];

    let change = input_sum - amount_natoshi;
    if change > U256::zero() {
        outputs.push(TransactionOutput::new(wallet.address.clone(), change));
    }

    let mut tx = Transaction {
        txid: "".to_string(),
        inputs: selected_inputs,
        outputs,
        timestamp: chrono::Utc::now().timestamp(),
    };

    // 5️⃣ 서명 (secp256k1)
    use netcoin_core::crypto::WalletKeypair;
    use secp256k1::SecretKey;

    let secret_bytes = hex::decode(wallet.secret_hex()).expect("Invalid secret key");
    let secret_key = SecretKey::from_slice(&secret_bytes).expect("Invalid secret key");
    let secp = secp256k1::Secp256k1::new();
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

    let keypair = WalletKeypair {
        secret_key,
        public_key,
    };

    if let Err(e) = tx.sign(&keypair) {
        println!("❌ Failed to sign transaction: {}", e);
        return;
    }

    tx.verify_signatures()
        .expect("Signature verification failed after signing");

    // 6️⃣ txid 채우기
    tx = tx.with_txid();

    println!("✅ Transaction created successfully!");
    println!("   TXID: {}", tx.txid);
    println!("   Amount: {} NTC", natoshi_to_ntc(amount_natoshi));
    if change > U256::zero() {
        println!("   Change: {} NTC", natoshi_to_ntc(change));
    }
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
