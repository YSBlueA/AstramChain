/// Pool payout module
///
/// Responsibilities:
///   1. Persist miner balances to RocksDB so they survive pool restarts.
///   2. Periodically build signed payout transactions and place them in a
///      pool-managed queue (PendingPayouts).
///   3. The stratum block-submission handler includes queued TXs in the next
///      block it mines, then clears the queue on success.
///
/// Payout flow:
///   1. Every interval: fetch pool UTXOs, build + sign payout TXs → PendingPayouts queue
///   2. build_template includes those TXs directly in the block (no node mempool)
///   3. On successful block: zero miner balances in tracker + DB, clear queue
///   4. On failed block: clear queue → rebuilt fresh next interval

use anyhow::{Result, anyhow};
use Astram_core::config::calculate_default_fee;
use Astram_core::crypto::WalletKeypair;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use primitive_types::U256;
use rocksdb::{DB, Options, WriteBatch};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

use crate::shares::ShareTracker;

type SharedTracker = Arc<std::sync::Mutex<ShareTracker>>;

// ─── Pending payout queue ─────────────────────────────────────────────────────

/// A signed payout transaction waiting to be included in the next mined block.
#[derive(Clone)]
pub struct PendingPayout {
    pub miner_addr: String,
    pub amount: U256,
    pub tx: Transaction,
}

/// Shared queue of built-but-not-yet-confirmed payout transactions.
/// Populated by run_payout_loop; drained by the block-submission handler.
pub type PendingPayouts = Arc<std::sync::Mutex<Vec<PendingPayout>>>;

// ─── Payout database ─────────────────────────────────────────────────────────

/// Thin RocksDB wrapper that persists miner pending balances.
///
/// Key space:
///   bal:<address>  → U256 decimal string
pub struct PayoutDb {
    db: Arc<DB>,
}

impl PayoutDb {
    pub fn open(path: &str) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)?;
        log::info!("💾 PayoutDB opened at {}", path);
        Ok(Self { db: Arc::new(db) })
    }

    /// Atomically overwrite the entire balance table.
    pub fn save_all(&self, balances: &HashMap<String, U256>) -> Result<()> {
        let mut batch = WriteBatch::default();

        // Delete all existing entries
        let mut iter = self.db.raw_iterator();
        iter.seek(b"bal:");
        while iter.valid() {
            match iter.key() {
                Some(k) if k.starts_with(b"bal:") => { batch.delete(k); }
                _ => break,
            }
            iter.next();
        }

        // Write new non-zero balances
        for (addr, bal) in balances {
            if *bal > U256::zero() {
                let key = format!("bal:{}", addr);
                batch.put(key.as_bytes(), bal.to_string().as_bytes());
            }
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// Load all persisted non-zero balances.
    pub fn load_all(&self) -> Result<HashMap<String, U256>> {
        let mut map = HashMap::new();
        let mut iter = self.db.raw_iterator();
        iter.seek(b"bal:");
        while iter.valid() {
            if let Some(key) = iter.key() {
                if !key.starts_with(b"bal:") { break; }
                let key_str = String::from_utf8_lossy(key);
                if let Some(addr) = key_str.strip_prefix("bal:") {
                    if let Some(val) = iter.value() {
                        let s = String::from_utf8_lossy(val);
                        if let Ok(bal) = U256::from_dec_str(&s) {
                            if bal > U256::zero() {
                                map.insert(addr.to_string(), bal);
                            }
                        }
                    }
                }
            }
            iter.next();
        }
        Ok(map)
    }

    /// Update or delete a single address balance.
    pub fn set(&self, address: &str, balance: U256) -> Result<()> {
        let key = format!("bal:{}", address);
        if balance == U256::zero() {
            self.db.delete(key.as_bytes())?;
        } else {
            self.db.put(key.as_bytes(), balance.to_string().as_bytes())?;
        }
        Ok(())
    }
}

// ─── Node helpers ─────────────────────────────────────────────────────────────

/// Parse a UTXO amount field from the node's JSON response.
/// Supports: amount_raw ([u64;4]), amount ([u64;4] or "0x..." hex or decimal).
fn parse_utxo_amount(u: &Value) -> U256 {
    // amount_raw: [u64; 4] (canonical)
    if let Some(arr) = u["amount_raw"].as_array() {
        let p: Vec<u64> = arr.iter().filter_map(|v| v.as_u64()).collect();
        if p.len() == 4 { return U256([p[0], p[1], p[2], p[3]]); }
    }
    // amount: [u64; 4] array
    if let Some(arr) = u["amount"].as_array() {
        let p: Vec<u64> = arr.iter().filter_map(|v| v.as_u64()).collect();
        if p.len() == 4 { return U256([p[0], p[1], p[2], p[3]]); }
    }
    // amount: "0x..." hex string
    if let Some(s) = u["amount"].as_str() {
        if let Some(hex) = s.strip_prefix("0x") {
            return U256::from_str_radix(hex, 16).unwrap_or(U256::zero());
        }
        return U256::from_dec_str(s).unwrap_or(U256::zero());
    }
    U256::zero()
}

/// Fetch spendable UTXOs for `address` from the node.
async fn fetch_utxos(
    client: &reqwest::Client,
    base_url: &str,
    address: &str,
) -> Result<Vec<(TransactionInput, U256)>> {
    let url = format!("{}/address/{}/utxos", base_url, address);
    let utxos: Vec<Value> = client.get(&url).send().await?.json().await?;

    let mut result = Vec::new();
    for u in &utxos {
        let txid = match u["txid"].as_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let vout = u["vout"].as_u64().unwrap_or(0) as u32;
        let amt = parse_utxo_amount(u);
        if amt > U256::zero() {
            result.push((
                TransactionInput { txid, vout, pubkey: address.to_string(), signature: None },
                amt,
            ));
        }
    }
    Ok(result)
}

// ─── Payout transaction builder ───────────────────────────────────────────────

/// Build and sign a payout transaction without submitting it to the node.
/// The returned TX is included directly in the next mined block.
pub async fn build_payout_tx(
    http: &reqwest::Client,
    base_url: &str,
    keypair: &WalletKeypair,
    pool_address: &str,
    to: &str,
    amount: U256,
) -> Result<Transaction> {
    let input_pool = fetch_utxos(http, base_url, pool_address).await?;
    if input_pool.is_empty() {
        return Err(anyhow!("pool wallet has no UTXOs"));
    }

    let mut selected: Vec<TransactionInput> = Vec::new();
    let mut input_sum = U256::zero();
    let mut cursor = 0usize;

    while cursor < input_pool.len() && input_sum < amount {
        let (inp, amt) = input_pool[cursor].clone();
        selected.push(inp);
        input_sum += amt;
        cursor += 1;
    }

    if input_sum < amount {
        return Err(anyhow!(
            "pool balance {} is less than payout amount {}",
            input_sum, amount
        ));
    }

    // Fee convergence loop
    let mut fee = U256::zero();
    for _ in 0..16 {
        while input_sum < amount + fee {
            if cursor >= input_pool.len() {
                return Err(anyhow!("pool balance insufficient to cover amount + fee"));
            }
            let (inp, amt) = input_pool[cursor].clone();
            selected.push(inp);
            input_sum += amt;
            cursor += 1;
        }

        let change = input_sum - amount - fee;
        let mut outputs = vec![TransactionOutput::new(to.to_string(), amount)];
        if change > U256::zero() {
            outputs.push(TransactionOutput::new(pool_address.to_string(), change));
        }

        let mut tx = Transaction {
            txid: String::new(),
            inputs: selected.clone(),
            outputs,
            timestamp: chrono::Utc::now().timestamp(),
        };

        tx.sign(keypair).map_err(|e| anyhow!("sign error: {}", e))?;
        tx = tx.with_hashes();

        let body = bincode::encode_to_vec(&tx, *BINCODE_CONFIG)
            .map_err(|e| anyhow!("serialize error: {}", e))?;
        let new_fee = calculate_default_fee(body.len());

        if new_fee <= fee {
            return Ok(tx);
        }
        fee = new_fee;
    }

    Err(anyhow!("fee convergence failed after 16 iterations"))
}

// ─── Background tasks ─────────────────────────────────────────────────────────

/// Periodically saves the full balance map to RocksDB (every 30 s).
/// This ensures balances survive a pool restart even between payouts.
pub async fn run_balance_sync(tracker: SharedTracker, payout_db: Arc<PayoutDb>) {
    loop {
        sleep(Duration::from_secs(30)).await;
        let balances = {
            let t = tracker.lock().unwrap();
            t.balances.clone()
        };
        if let Err(e) = payout_db.save_all(&balances) {
            log::warn!("⚠️  Balance sync to DB failed: {}", e);
        }
    }
}

/// Payout loop: every `interval_secs`, build signed payout TXs for eligible miners
/// and place them in `pending_payouts`. The block-submission handler will include
/// them in the next mined block and zero out balances on confirmation.
pub async fn run_payout_loop(
    http: reqwest::Client,
    base_url: String,
    tracker: SharedTracker,
    _payout_db: Arc<PayoutDb>,
    keypair: Arc<WalletKeypair>,
    pool_address: String,
    threshold: U256,
    interval_secs: u64,
    pending_payouts: PendingPayouts,
) {
    // Stagger the first run so the pool has time to sync with the node
    sleep(Duration::from_secs(60)).await;

    loop {
        let candidates: Vec<(String, U256)> = {
            let t = tracker.lock().unwrap();
            t.balances
                .iter()
                .filter(|(addr, bal)| {
                    **bal >= threshold && addr.as_str() != pool_address.as_str()
                })
                .map(|(a, b)| (a.clone(), *b))
                .collect()
        };

        if !candidates.is_empty() {
            log::info!(
                "💸 Payout: {} miner(s) eligible, building TXs for next block",
                candidates.len()
            );

            // Rebuild the queue fresh each interval (UTXOs may have changed).
            let mut new_pending: Vec<PendingPayout> = Vec::new();
            for (miner_addr, amount) in candidates {
                match build_payout_tx(&http, &base_url, &keypair, &pool_address, &miner_addr, amount).await {
                    Ok(tx) => {
                        log::info!("💸 Queued payout {} → {} ram", miner_addr, amount);
                        new_pending.push(PendingPayout { miner_addr, amount, tx });
                    }
                    Err(e) => {
                        log::warn!("❌ Failed to build payout TX for {}: {}", miner_addr, e);
                    }
                }
            }

            *pending_payouts.lock().unwrap() = new_pending;
        }

        sleep(Duration::from_secs(interval_secs)).await;
    }
}
