use crate::wallet::Wallet;
use Astram_config::config::Config;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use primitive_types::U256;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
// ASRM unit constants (18 decimal places, same as Ethereum)
const RAM_PER_ASRM: u128 = 1_000_000_000_000_000_000; // 1 ASRM = 10^18 ram

/// Convert ASRM to ram (smallest unit) as U256
pub fn ASRM_to_ram(ASRM: f64) -> U256 {
    let ram = (ASRM * RAM_PER_ASRM as f64) as u128;
    U256::from(ram)
}

/// Convert ram (U256) to ASRM for display
pub fn ram_to_ASRM(ram: U256) -> f64 {
    // Convert U256 to u128 (safe for reasonable amounts)
    let ram_u128 = ram.low_u128();
    ram_u128 as f64 / RAM_PER_ASRM as f64
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
    /// Amount should be specified in ASRM (e.g., 1.5 for 1.5 ASRM)
    Send {
        to: String,
        #[arg(help = "Amount in ASRM (e.g., 1.5)")]
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
    println!("[OK] New wallet created successfully!");
    println!("Address: {}", wallet.address);
    println!("Private Key: {}", wallet.secret_hex());
    println!("Public Key: {}", wallet.public_hex());
    println!("Checksum Address: {}", wallet.checksummed_address());
    println!();
    println!("[WARN] IMPORTANT: Save your private key securely!");
    println!("   You can import this into MetaMask using the private key.");

    let path = get_wallet_path();
    save_wallet_json(&wallet, path.to_str().unwrap()).expect("Failed to save wallet");
}

pub fn generate_eth_wallet() {
    // Same as generate_wallet now, since all wallets are Ethereum-compatible
    generate_wallet();
    println!();
    println!("To add Astram to MetaMask:");
    println!("   Network Name: Astram Localhost");
    println!("   RPC URL: http://127.0.0.1:8545");
    println!("   Chain ID: 8888");
    println!("   Currency Symbol: ASRM");
}

fn load_wallet() -> Wallet {
    let path = get_wallet_path();
    let data = fs::read_to_string(&path).expect("Failed to read wallet file");
    let wallet_json: WalletJson = serde_json::from_str(&data).expect("Failed to parse wallet JSON");

    println!("[INFO] Wallet loaded: {}", wallet_json.address);
    println!("Private key: {}", wallet_json.secret_key);

    Wallet::from_hex(&wallet_json.secret_key)
}

pub fn get_balance(address: &str) {
    let cfg = Config::load();
    let url = format!("{}/address/{}/balance", cfg.node_rpc_url, address);
    match Client::new().get(&url).send() {
        Ok(res) => {
            let json: Value = res.json().unwrap();
            // Parse balance as hex string (0x...) or number
            let balance_ram = if let Some(s) = json["balance"].as_str() {
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
            let balance_ASRM = ram_to_ASRM(balance_ram);
            println!("Balance: {} ASRM", balance_ASRM);
        }
        Err(e) => println!("[ERROR] Query failed: {}", e),
    }
}

pub fn send_transaction(to: &str, amount_ram: U256) {
    let cfg = Config::load();
    let wallet = load_wallet();
    let client = Client::new();

    let url = format!("{}/address/{}/utxos", cfg.node_rpc_url, wallet.address);
    let utxos: Vec<Value> = match client.get(&url).send() {
        Ok(res) => match res.json() {
            Ok(v) => v,
            Err(e) => {
                println!("[ERROR] Failed to parse UTXOs JSON: {}", e);
                return;
            }
        },
        Err(e) => {
            println!("[ERROR] Query failed: {}", e);
            return;
        }
    };

    if utxos.is_empty() {
        println!("[WARN] No UTXOs available for address {}", wallet.address);
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
        if input_sum >= amount_ram {
            break;
        }
    }

    if input_sum < amount_ram {
        println!(
            "[WARN] Insufficient balance: have {} ASRM, need {} ASRM",
            ram_to_ASRM(input_sum),
            ram_to_ASRM(amount_ram)
        );
        return;
    }

    // Calculate estimated transaction size and fee
    // Typical transaction structure:
    // - Base: ~100 bytes
    // - Per input: ~100 bytes (txid, vout, pubkey, signature)
    // - Per output: ~50 bytes (address, amount)
    let estimated_tx_size = 100 + (selected_inputs.len() * 100) + (2 * 50); // Assume 2 outputs max
    let fee = Astram_core::config::calculate_default_fee(estimated_tx_size);

    println!("Transaction Details:");
    println!("   Inputs: {} UTXO(s)", selected_inputs.len());
    println!("   Estimated size: {} bytes", estimated_tx_size);
    println!("   Fee: {} ASRM ({} ram)", ram_to_ASRM(fee), fee);

    // Check if we have enough for amount + fee
    if input_sum < amount_ram + fee {
        println!(
            "[WARN] Insufficient balance for amount + fee: have {} ASRM, need {} ASRM",
            ram_to_ASRM(input_sum),
            ram_to_ASRM(amount_ram + fee)
        );
        return;
    }

    let mut outputs = vec![TransactionOutput::new(to.to_string(), amount_ram)];

    let change = input_sum - amount_ram - fee;
    if change > U256::zero() {
        outputs.push(TransactionOutput::new(wallet.address.clone(), change));
    } else {
        println!("   No change (exact amount + fee)");
    }

    let mut tx = Transaction {
        txid: "".to_string(),
        eth_hash: "".to_string(),
        inputs: selected_inputs,
        outputs,
        timestamp: chrono::Utc::now().timestamp(),
    };

    // Step 5: Sign transaction (secp256k1)
    use Astram_core::crypto::WalletKeypair;
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
        println!("??Failed to sign transaction: {}", e);
        return;
    }

    tx.verify_signatures()
        .expect("Signature verification failed after signing");

    // Step 6: Populate txid and eth_hash
    tx = tx.with_hashes();

    println!("[OK] Transaction created successfully!");
    println!("   TXID (internal): {}", tx.txid);
    println!("   ETH Hash (external): {}", tx.eth_hash);
    println!("   Amount: {} ASRM", ram_to_ASRM(amount_ram));
    println!("   Fee: {} ASRM ({} ram)", ram_to_ASRM(fee), fee);
    if change > U256::zero() {
        println!("   Change: {} ASRM", ram_to_ASRM(change));
    }
    println!(
        "Signature: {}",
        tx.inputs
            .get(0)
            .and_then(|i| i.signature.as_deref())
            .unwrap_or("no signature")
    );

    // Step 7: Serialize
    let body = match bincode::encode_to_vec(&tx, *BINCODE_CONFIG) {
        Ok(b) => b,
        Err(e) => {
            println!("[ERROR] Failed to serialize transaction: {}", e);
            return;
        }
    };

    // Step 8: POST /tx
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
                println!("[OK] Transaction broadcast completed!");
            } else {
                println!("[ERROR] Transaction failed!");
                println!("Status: {}", status);
                println!("Response body: {}", text);
            }
        }
        Err(e) => println!("[ERROR] Transaction failed (network/reqwest error): {}", e),
    }
}
