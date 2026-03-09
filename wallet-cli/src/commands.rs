use crate::wallet::Wallet;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use astram_config::config::Config;
use primitive_types::U256;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
// ASRM unit constants (18 decimal places)
const RAM_PER_ASRM: u128 = 1_000_000_000_000_000_000; // 1 ASRM = 10^18 ram

/// Convert ASRM to ram (smallest unit) as U256
pub fn asrm_to_ram(asrm: f64) -> U256 {
    let ram = (asrm * RAM_PER_ASRM as f64) as u128;
    U256::from(ram)
}

/// Convert ram (U256) to ASRM for display
pub fn ram_to_asrm(ram: U256) -> f64 {
    // Convert U256 to u128 (safe for reasonable amounts)
    let ram_u128 = ram.low_u128();
    ram_u128 as f64 / RAM_PER_ASRM as f64
}

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Create a new wallet (Ed25519 + BIP39 24-word mnemonic)
    Generate,

    /// Create a new Ed25519 wallet (alias for Generate)
    GenerateEth,
    
    /// Import wallet from 24-word recovery phrase (compatible with Chrome wallet)
    Import {
        #[arg(help = "24-word recovery phrase")]
        mnemonic: String,
    },

    /// Check the balance of a specific address (or current wallet if not specified)
    Balance { 
        #[arg(help = "Address to check balance (defaults to current wallet)")]
        address: Option<String> 
    },

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
    #[serde(skip_serializing_if = "Option::is_none")]
    mnemonic: Option<String>,
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
        mnemonic: wallet.mnemonic.clone(),
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
    
    if let Some(ref mnemonic) = wallet.mnemonic {
        println!();
        println!("=== 24-Word Recovery Phrase (BIP39) ===");
        println!("{}", mnemonic);
        println!("========================================");
        println!();
        println!("[INFO] This mnemonic is compatible with Chrome wallet!");
    }
    
    println!();
    println!("[WARN] IMPORTANT: Save your recovery phrase and private key securely!");

    let path = get_wallet_path();
    save_wallet_json(&wallet, path.to_str().unwrap()).expect("Failed to save wallet");
}

pub fn generate_eth_wallet() {
    // Same as generate_wallet now, since all wallets are Ed25519
    generate_wallet();
}

pub fn import_wallet(mnemonic: &str) {
    let wallet = Wallet::from_mnemonic_str(mnemonic);
    println!("[OK] Wallet imported successfully from recovery phrase!");
    println!("Address: {}", wallet.address);
    println!("Private Key: {}", wallet.secret_hex());
    println!("Public Key: {}", wallet.public_hex());
    println!("Checksum Address: {}", wallet.checksummed_address());
    println!();
    println!("[INFO] This wallet is compatible with Chrome wallet!");
    println!();
    println!("[WARN] Wallet will be saved to disk.");

    let path = get_wallet_path();
    save_wallet_json(&wallet, path.to_str().unwrap()).expect("Failed to save wallet");
    println!("[OK] Wallet saved to: {}", path.display());
}

pub fn load_wallet() -> Wallet {
    let path = get_wallet_path();
    let data = fs::read_to_string(&path).expect("Failed to read wallet file");
    let wallet_json: WalletJson = serde_json::from_str(&data).expect("Failed to parse wallet JSON");

    let wallet = Wallet::from_hex(&wallet_json.secret_key);
    
    // Check if stored address matches the one derived from private key
    if wallet_json.address != wallet.address {
        println!("[WARN] Address mismatch detected!");
        println!("       Stored address:  {}", wallet_json.address);
        println!("       Derived address: {}", wallet.address);
        println!("[INFO] Auto-fixing wallet.json with correct address...");
        
        // Update wallet.json with correct address
        let updated_wallet = WalletJson {
            secret_key: wallet_json.secret_key.clone(),
            address: wallet.address.clone(),
            mnemonic: wallet_json.mnemonic.clone(),
        };
        if let Ok(data) = serde_json::to_string_pretty(&updated_wallet) {
            if fs::write(&path, data).is_ok() {
                println!("[OK] Wallet file updated with correct address");
            }
        }
    }
    
    println!("[INFO] Wallet loaded: {}", wallet.address);
    println!("Private key: {}", wallet_json.secret_key);

    wallet
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
            let balance_asrm = ram_to_asrm(balance_ram);
            println!("Balance: {} ASRM", balance_asrm);
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

    let mut input_pool: Vec<(TransactionInput, U256)> = Vec::new();

    for (_i, u) in utxos.iter().enumerate() {
        let txid = u["txid"].as_str().unwrap().to_string();
        let vout = u["vout"].as_u64().unwrap() as u32;
        
        // Parse amount: try amount_raw first, then amount (for backwards compatibility)
        let amt = if let Some(arr) = u["amount_raw"].as_array() {
            // amount_raw: [u64; 4] array from Utxo struct
            let parts: Vec<u64> = arr.iter()
                .filter_map(|v| v.as_u64())
                .collect();
            if parts.len() == 4 {
                U256([parts[0], parts[1], parts[2], parts[3]])
            } else {
                U256::zero()
            }
        } else if let Some(arr) = u["amount"].as_array() {
            // Fallback to "amount" field
            let parts: Vec<u64> = arr.iter()
                .filter_map(|v| v.as_u64())
                .collect();
            if parts.len() == 4 {
                U256([parts[0], parts[1], parts[2], parts[3]])
            } else {
                U256::zero()
            }
        } else if let Some(s) = u["amount"].as_str() {
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

        input_pool.push((TransactionInput {
            txid,
            vout,
            pubkey: wallet.address.clone(),
            signature: None,
        }, amt));
    }

    let mut selected_inputs: Vec<TransactionInput> = vec![];
    let mut input_sum = U256::zero();
    let mut cursor = 0usize;

    while cursor < input_pool.len() && input_sum < amount_ram {
        let (inp, amt) = input_pool[cursor].clone();
        selected_inputs.push(inp);
        input_sum = input_sum + amt;
        cursor += 1;
    }

    if input_sum < amount_ram {
        println!(
            "[WARN] Insufficient balance: have {} ASRM, need {} ASRM",
            ram_to_asrm(input_sum),
            ram_to_asrm(amount_ram)
        );
        return;
    }
    // Step 5: Sign transaction (Ed25519)
    use Astram_core::crypto::WalletKeypair;

    let keypair = WalletKeypair::from_secret_hex(&wallet.secret_hex())
        .expect("Invalid secret key");

    // Build transaction with exact serialized size-based fee.
    // If fee grows, keep adding UTXOs until amount + fee is covered.
    let mut fee = U256::zero();
    let mut final_tx: Option<Transaction> = None;
    let mut final_tx_size: usize = 0;
    let mut final_change = U256::zero();

    for _ in 0..16 {
        while input_sum < amount_ram + fee {
            if cursor >= input_pool.len() {
                println!(
                    "[WARN] Insufficient balance for amount + fee: have {} ASRM, need {} ASRM",
                    ram_to_asrm(input_sum),
                    ram_to_asrm(amount_ram + fee)
                );
                return;
            }
            let (inp, amt) = input_pool[cursor].clone();
            selected_inputs.push(inp);
            input_sum = input_sum + amt;
            cursor += 1;
        }

        let change = input_sum - amount_ram - fee;
        let mut outputs = vec![TransactionOutput::new(to.to_string(), amount_ram)];
        if change > U256::zero() {
            outputs.push(TransactionOutput::new(wallet.address.clone(), change));
        }

        let mut candidate_tx = Transaction {
            txid: "".to_string(),
            inputs: selected_inputs.clone(),
            outputs,
            timestamp: chrono::Utc::now().timestamp(),
        };

        if let Err(e) = candidate_tx.sign(&keypair) {
            println!("[ERROR] Failed to sign transaction: {}", e);
            return;
        }

        match candidate_tx.verify_signatures() {
            Ok(true) => {}
            Ok(false) => {
                println!("[ERROR] Signature verification failed after signing");
                return;
            }
            Err(e) => {
                println!("[ERROR] Signature verification error: {}", e);
                return;
            }
        }

        candidate_tx = candidate_tx.with_hashes();

        let candidate_body = match bincode::encode_to_vec(&candidate_tx, *BINCODE_CONFIG) {
            Ok(b) => b,
            Err(e) => {
                println!("[ERROR] Failed to serialize transaction: {}", e);
                return;
            }
        };

        let candidate_size = candidate_body.len();
        let candidate_fee = Astram_core::config::calculate_default_fee(candidate_size);

        if candidate_fee > fee {
            fee = candidate_fee;
            continue;
        }

        final_tx = Some(candidate_tx);
        final_tx_size = candidate_size;
        final_change = change;
        break;
    }

    let tx = match final_tx {
        Some(t) => t,
        None => {
            println!("[ERROR] Failed to converge transaction fee calculation");
            return;
        }
    };

    println!("Transaction Details:");
    println!("   Inputs: {} UTXO(s)", tx.inputs.len());
    println!("   Serialized size: {} bytes", final_tx_size);
    println!("   Fee: {} ASRM ({} ram)", ram_to_asrm(fee), fee);
    if final_change == U256::zero() {
        println!("   No change (exact amount + fee)");
    }

    println!("[OK] Transaction created successfully!");
    println!("   TXID (internal): {}", tx.txid);
    println!("   Amount: {} ASRM", ram_to_asrm(amount_ram));
    println!("   Fee: {} ASRM ({} ram)", ram_to_asrm(fee), fee);
    if final_change > U256::zero() {
        println!("   Change: {} ASRM", ram_to_asrm(final_change));
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
