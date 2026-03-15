mod payout;
mod session;
mod share_validator;
mod shares;
mod vardiff;

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use futures::{SinkExt, StreamExt};
use astram_config::config::Config;
use Astram_core::block::{Block, BlockHeader, compute_merkle_root};
use Astram_core::config::calculate_block_reward;
use Astram_core::crypto::WalletKeypair;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction};
use payout::{PayoutDb, run_balance_sync, run_payout_loop};
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session::MinerSession;
use share_validator::{ShareResult, initial_pool_difficulty, pool_diff_to_target, validate_share};
use shares::{FoundBlock, Share, ShareTracker};
use vardiff::{VarDiffConfig, check_vardiff};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::time::{Duration, sleep};
use tokio_util::codec::{Framed, LinesCodec};
use warp::Filter;

// ─── Pool configuration ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PoolConfig {
    node_rpc_url: String,
    stratum_bind: String,
    gbt_bind: String,
    stats_bind: String,
    pool_address: String,
    pool_fee_percent: f64,
    pplns_window: usize,
    vardiff: VarDiffConfig,
    // Payout settings
    payout_threshold_ram: U256,
    payout_interval_secs: u64,
    payout_db_path: String,
}

impl PoolConfig {
    fn from_env(cfg: &Config) -> Result<Self> {
        let node_rpc_url =
            std::env::var("NODE_RPC_URL").unwrap_or_else(|_| cfg.node_rpc_url.clone());

        let pool_address = std::env::var("POOL_ADDRESS")
            .ok()
            .or_else(|| load_pool_address(cfg).ok())
            .ok_or_else(|| anyhow!("POOL_ADDRESS not set and wallet missing"))?;

        Ok(Self {
            node_rpc_url,
            stratum_bind: std::env::var("STRATUM_BIND")
                .unwrap_or_else(|_| "0.0.0.0:3333".to_string()),
            gbt_bind: std::env::var("GBT_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8332".to_string()),
            stats_bind: std::env::var("STATS_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8081".to_string()),
            pool_address,
            pool_fee_percent: std::env::var("POOL_FEE_PERCENT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0),
            pplns_window: std::env::var("PPLNS_WINDOW")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10_000),
            vardiff: VarDiffConfig {
                min_diff: std::env::var("VARDIFF_MIN")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1),
                max_diff: std::env::var("VARDIFF_MAX")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(32),
                target_share_time: std::env::var("VARDIFF_TARGET_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(15.0),
                ..VarDiffConfig::default()
            },
            payout_threshold_ram: {
                let asrm: f64 = std::env::var("PAYOUT_THRESHOLD_ASRM")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.5_f64);
                U256::from((asrm * 1_000_000_000_000_000_000_f64) as u128)
            },
            payout_interval_secs: std::env::var("PAYOUT_INTERVAL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(600u64),
            payout_db_path: std::env::var("POOL_DB_PATH")
                .unwrap_or_else(|_| "pool_data".to_string()),
        })
    }
}

// ─── Node RPC client ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct NodeClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct MempoolSnapshot {
    txs: Vec<Transaction>,
    total_fees: U256,
}

#[derive(Debug, Clone)]
struct ChainStatus {
    height: u64,
    difficulty: u32,      // current chain tip compact bits
    next_difficulty: u32, // DWG3-adjusted difficulty for the next block
    tip_hash: String,
}

#[derive(Deserialize)]
struct MempoolResponse {
    transactions_b64: String,
    total_fees: String,
}

#[derive(Deserialize)]
struct SubmitBlockResponse {
    status: String,
    #[serde(rename = "hash")]
    _hash: Option<String>,
    message: Option<String>,
}

impl NodeClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    async fn fetch_status(&self) -> Result<ChainStatus> {
        let url = format!("{}/status", self.base_url);
        let value: Value = self.client.get(&url).send().await?.json().await?;

        let bc = value.get("blockchain");

        let height = bc
            .and_then(|v| v.get("height"))
            .and_then(|v| v.as_u64())
            .or_else(|| value.get("height").and_then(|v| v.as_u64()))
            .unwrap_or(0);

        let difficulty = bc
            .and_then(|v| v.get("difficulty"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        // Use next_difficulty (DWG3-adjusted) for block templates.
        // Falls back to difficulty if the node is older and doesn't expose it.
        let next_difficulty = bc
            .and_then(|v| v.get("next_difficulty"))
            .and_then(|v| v.as_u64())
            .unwrap_or(difficulty as u64) as u32;

        let tip_hash = bc
            .and_then(|v| v.get("chain_tip"))
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();

        Ok(ChainStatus { height, difficulty, next_difficulty, tip_hash })
    }

    async fn fetch_mempool(&self) -> Result<MempoolSnapshot> {
        let url = format!("{}/mempool", self.base_url);
        let resp: MempoolResponse = self.client.get(&url).send().await?.json().await?;

        let bytes = general_purpose::STANDARD
            .decode(resp.transactions_b64.as_bytes())
            .map_err(|e| anyhow!("invalid mempool base64: {}", e))?;
        let (txs, _) = bincode::decode_from_slice::<Vec<Transaction>, _>(&bytes, *BINCODE_CONFIG)
            .map_err(|e| anyhow!("invalid mempool bincode: {}", e))?;

        let total_fees = parse_u256(&resp.total_fees).unwrap_or_else(U256::zero);
        Ok(MempoolSnapshot { txs, total_fees })
    }

    async fn submit_block(&self, block: &Block) -> Result<()> {
        let bytes = bincode::encode_to_vec(block, *BINCODE_CONFIG)?;
        let payload = serde_json::json!({
            "block_b64": general_purpose::STANDARD.encode(bytes)
        });
        let url = format!("{}/mining/submit", self.base_url);
        let resp: SubmitBlockResponse = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        if resp.status == "ok" {
            Ok(())
        } else {
            Err(anyhow!(resp.message.unwrap_or_else(|| "submit failed".to_string())))
        }
    }
}

// ─── Mining template ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MiningTemplate {
    job_id: String,
    height: u64,
    prev_hash: String,
    difficulty: u32,   // compact bits – network difficulty
    pool_diff: u32,    // leading zeros – pool share difficulty
    timestamp: i64,
    merkle_root: String,
    transactions: Vec<Transaction>,
    coinbase_value: U256,
}

fn parse_u256(value: &str) -> Option<U256> {
    if let Some(hex) = value.strip_prefix("0x") {
        return U256::from_str_radix(hex, 16).ok();
    }
    U256::from_dec_str(value).ok()
}

async fn build_template(
    client: &NodeClient,
    pool_address: &str,
    job_id: String,
) -> Result<MiningTemplate> {
    let status = client.fetch_status().await?;
    let mempool = client.fetch_mempool().await?;

    let height = status.height + 1;
    let prev_hash = if status.tip_hash == "none" {
        "0".repeat(64)
    } else {
        status.tip_hash.clone()
    };

    let base_reward = calculate_block_reward(height);
    let coinbase_value = base_reward + mempool.total_fees;
    let coinbase = Transaction::coinbase(pool_address, coinbase_value).with_hashes();

    let mut all_txs = vec![coinbase];
    all_txs.extend(mempool.txs);

    let txids: Vec<String> = all_txs.iter().map(|t| t.txid.clone()).collect();
    let merkle_root = compute_merkle_root(&txids);

    // Use next_difficulty so the block header matches what the node's DWG3
    // algorithm expects for the next block — prevents "difficulty mismatch" rejections.
    let block_difficulty = status.next_difficulty;
    let pool_diff = initial_pool_difficulty(block_difficulty);

    Ok(MiningTemplate {
        job_id,
        height,
        prev_hash,
        difficulty: block_difficulty,
        pool_diff,
        timestamp: chrono::Utc::now().timestamp(),
        merkle_root,
        transactions: all_txs,
        coinbase_value,
    })
}

fn build_header_for_nonce(template: &MiningTemplate, nonce: u64) -> BlockHeader {
    BlockHeader {
        index: template.height,
        previous_hash: template.prev_hash.clone(),
        merkle_root: template.merkle_root.clone(),
        timestamp: template.timestamp,
        nonce,
        difficulty: template.difficulty,
    }
}

fn parse_nonce(nonce_str: &str) -> Result<u64> {
    if let Some(hex) = nonce_str.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).map_err(|e| anyhow!("invalid nonce: {}", e));
    }
    if nonce_str.chars().all(|c| c.is_ascii_digit()) {
        return nonce_str.parse::<u64>().map_err(|e| anyhow!("invalid nonce: {}", e));
    }
    u64::from_str_radix(nonce_str, 16).map_err(|e| anyhow!("invalid nonce: {}", e))
}

fn decode_block_payload(input: &str) -> Result<Block> {
    let bytes = if input.chars().all(|c| c.is_ascii_hexdigit()) && input.len() % 2 == 0 {
        hex::decode(input)?
    } else {
        general_purpose::STANDARD
            .decode(input.as_bytes())
            .map_err(|e| anyhow!("invalid base64: {}", e))?
    };
    let (block, _) = bincode::decode_from_slice::<Block, _>(&bytes, *BINCODE_CONFIG)?;
    Ok(block)
}

// ─── Shared pool state ────────────────────────────────────────────────────────

type SharedTracker = Arc<Mutex<ShareTracker>>;
type TemplateStore = Arc<Mutex<HashMap<String, MiningTemplate>>>;
type ActiveConnections = Arc<AtomicU64>;

// ─── Stratum connection handler ───────────────────────────────────────────────

async fn handle_stratum_connection(
    stream: TcpStream,
    template_store: TemplateStore,
    job_rx: broadcast::Receiver<MiningTemplate>,
    pool_cfg: Arc<PoolConfig>,
    client: NodeClient,
    tracker: SharedTracker,
    force_rebuild_tx: watch::Sender<u64>,
) -> Result<()> {
    // Shared slot: inner handler writes extranonce1 here on auth so we can clean up on exit.
    let registered_e1: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let result = handle_stratum_inner(
        stream, template_store, job_rx, pool_cfg, client, tracker.clone(), registered_e1.clone(), force_rebuild_tx,
    ).await;
    if let Some(e1) = registered_e1.lock().unwrap().take() {
        tracker.lock().unwrap().unregister_worker(&e1);
    }
    result
}

async fn handle_stratum_inner(
    stream: TcpStream,
    template_store: TemplateStore,
    mut job_rx: broadcast::Receiver<MiningTemplate>,
    pool_cfg: Arc<PoolConfig>,
    client: NodeClient,
    tracker: SharedTracker,
    registered_e1: Arc<std::sync::Mutex<Option<String>>>,
    force_rebuild_tx: watch::Sender<u64>,
) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new());
    let mut session: Option<MinerSession> = None;

    loop {
        tokio::select! {
            maybe_line = framed.next() => {
                let line = match maybe_line {
                    Some(Ok(l)) => l,
                    Some(Err(e)) => return Err(anyhow!("stream error: {}", e)),
                    None => return Ok(()),
                };

                let req: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let params = req.get("params");

                match method {
                    // ── mining.subscribe ──────────────────────────────────
                    "mining.subscribe" => {
                        let extranonce1 = hex::encode(rand::random::<u32>().to_be_bytes());
                        let extranonce2_size = 4u32;

                        // Build initial template to get block difficulty for VarDiff
                        let job_id = format!("{}", chrono::Utc::now().timestamp_millis());
                        let init_diff = match build_template(&client, &pool_cfg.pool_address, job_id.clone()).await {
                            Ok(t) => {
                                template_store.lock().unwrap().insert(job_id.clone(), t.clone());
                                let pool_diff = t.pool_diff.max(pool_cfg.vardiff.min_diff);
                                // send initial difficulty
                                let diff_msg = serde_json::json!({
                                    "id": null,
                                    "method": "mining.set_difficulty",
                                    "params": [pool_diff]
                                });
                                framed.send(diff_msg.to_string()).await?;
                                // send first job
                                let notify = build_notify(&t);
                                framed.send(notify.to_string()).await?;
                                pool_diff
                            }
                            Err(e) => {
                                log::warn!("subscribe: failed to build template: {}", e);
                                pool_cfg.vardiff.min_diff
                            }
                        };

                        let mut s = MinerSession::new(extranonce1.clone(), init_diff);
                        // clamp initial difficulty to configured range
                        s.difficulty = s.difficulty.clamp(pool_cfg.vardiff.min_diff, pool_cfg.vardiff.max_diff);
                        session = Some(s);

                        let result = serde_json::json!([
                            [["mining.set_difficulty", "1"], ["mining.notify", "1"]],
                            extranonce1,
                            extranonce2_size
                        ]);
                        let resp = serde_json::json!({"id": id, "result": result, "error": null});
                        framed.send(resp.to_string()).await?;
                    }

                    // ── mining.authorize ─────────────────────────────────
                    "mining.authorize" => {
                        let login = params
                            .and_then(|v| v.as_array())
                            .and_then(|a| a.first())
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");

                        if let Some(ref mut s) = session {
                            s.authorize(login);
                            log::info!(
                                "[AUTH] worker={}.{} diff={}",
                                s.miner_address,
                                s.worker_name,
                                s.difficulty
                            );
                            // Register as connected worker so it shows in the stats UI
                            tracker.lock().unwrap().register_worker(
                                s.extranonce1.clone(),
                                s.miner_address.clone(),
                                s.worker_name.clone(),
                                s.difficulty,
                            );
                            *registered_e1.lock().unwrap() = Some(s.extranonce1.clone());
                        }

                        let resp = serde_json::json!({"id": id, "result": true, "error": null});
                        framed.send(resp.to_string()).await?;
                    }

                    // ── mining.submit ─────────────────────────────────────
                    "mining.submit" => {
                        let p = params.and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let job_id = p.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let nonce_str = p.get(2).and_then(|v| v.as_str()).unwrap_or("");

                        let s = match session.as_mut() {
                            Some(s) if s.is_authorized() => s,
                            _ => {
                                let resp = serde_json::json!({"id": id, "result": false, "error": "not authorized"});
                                framed.send(resp.to_string()).await?;
                                continue;
                            }
                        };

                        let nonce = match parse_nonce(nonce_str) {
                            Ok(n) => n,
                            Err(e) => {
                                s.record_rejected_share();
                                tracker.lock().unwrap().add_rejected(&s.miner_address);
                                let resp = serde_json::json!({"id": id, "result": false, "error": e.to_string()});
                                framed.send(resp.to_string()).await?;
                                continue;
                            }
                        };

                        let template = template_store.lock().unwrap().get(&job_id).cloned();
                        match template {
                            None => {
                                s.record_rejected_share();
                                tracker.lock().unwrap().add_rejected(&s.miner_address);
                                let resp = serde_json::json!({"id": id, "result": false, "error": "unknown job"});
                                framed.send(resp.to_string()).await?;
                            }
                            Some(tmpl) => {
                                let header = build_header_for_nonce(&tmpl, nonce);
                                match validate_share(&header, s.difficulty, tmpl.difficulty) {
                                    Ok(ShareResult::Rejected { reason }) => {
                                        s.record_rejected_share();
                                        tracker.lock().unwrap().add_rejected(&s.miner_address);
                                        log::debug!("[REJECT] {}.{}: {}", s.miner_address, s.worker_name, reason);
                                        let resp = serde_json::json!({"id": id, "result": false, "error": reason});
                                        framed.send(resp.to_string()).await?;
                                    }
                                    Ok(ShareResult::AcceptedShare { hash }) => {
                                        s.record_accepted_share();
                                        {
                                            let mut t = tracker.lock().unwrap();
                                            t.add_share(Share {
                                                miner_address: s.miner_address.clone(),
                                                worker_name: s.worker_name.clone(),
                                                difficulty: s.difficulty,
                                                timestamp: chrono::Utc::now().timestamp(),
                                                job_id: job_id.clone(),
                                            });
                                        }
                                        log::debug!("[SHARE] {}.{} hash={}", s.miner_address, s.worker_name, &hash[..8]);

                                        // VarDiff check
                                        if let Some(new_diff) = check_vardiff(s, &pool_cfg.vardiff) {
                                            s.difficulty = new_diff;
                                            let diff_msg = serde_json::json!({
                                                "id": null,
                                                "method": "mining.set_difficulty",
                                                "params": [new_diff]
                                            });
                                            framed.send(diff_msg.to_string()).await?;
                                        }

                                        let resp = serde_json::json!({"id": id, "result": true, "error": null});
                                        framed.send(resp.to_string()).await?;
                                    }
                                    Ok(ShareResult::FoundBlock { hash: _ }) => {
                                        // Build and submit the full block
                                        let block = Block {
                                            header,
                                            transactions: tmpl.transactions.clone(),
                                            hash: compute_header_hash_str(&tmpl, nonce),
                                        };
                                        match client.submit_block(&block).await {
                                            Ok(_) => {
                                                log::info!(
                                                    "🎉 [BLOCK] height={} finder={}.{}",
                                                    tmpl.height,
                                                    s.miner_address,
                                                    s.worker_name
                                                );
                                                // PPLNS distribution
                                                let reward = tmpl.coinbase_value;
                                                let finder = s.miner_address.clone();
                                                let fee_fraction = pool_cfg.pool_fee_percent / 100.0;
                                                let credits = {
                                                    let mut t = tracker.lock().unwrap();
                                                    t.add_share(Share {
                                                        miner_address: s.miner_address.clone(),
                                                        worker_name: s.worker_name.clone(),
                                                        difficulty: s.difficulty,
                                                        timestamp: chrono::Utc::now().timestamp(),
                                                        job_id: job_id.clone(),
                                                    });
                                                    t.distribute_pplns(
                                                        reward,
                                                        fee_fraction,
                                                        &finder,
                                                        tmpl.height,
                                                        block.hash.clone(),
                                                        chrono::Utc::now().timestamp(),
                                                    )
                                                };
                                                for (addr, amount) in &credits {
                                                    log::info!(
                                                        "  💰 {} credited 0x{:x} wei",
                                                        addr, amount
                                                    );
                                                }
                                                s.record_accepted_share();
                                                // Signal template loop to rebuild immediately
                                                let _ = force_rebuild_tx.send(chrono::Utc::now().timestamp_millis() as u64);
                                                let resp = serde_json::json!({"id": id, "result": true, "block_found": true, "error": null});
                                                framed.send(resp.to_string()).await?;
                                            }
                                            Err(e) => {
                                                log::warn!("[BLOCK] submit failed: {}", e);
                                                s.record_rejected_share();
                                                tracker.lock().unwrap().add_rejected(&s.miner_address);
                                                // Signal template loop to rebuild immediately (stale tip / fork)
                                                let _ = force_rebuild_tx.send(chrono::Utc::now().timestamp_millis() as u64);
                                                let resp = serde_json::json!({"id": id, "result": false, "error": e.to_string()});
                                                framed.send(resp.to_string()).await?;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        s.record_rejected_share();
                                        tracker.lock().unwrap().add_rejected(&s.miner_address);
                                        let resp = serde_json::json!({"id": id, "result": false, "error": e.to_string()});
                                        framed.send(resp.to_string()).await?;
                                    }
                                }
                            }
                        }
                    }

                    _ => {
                        let resp = serde_json::json!({"id": id, "result": null, "error": "unsupported method"});
                        framed.send(resp.to_string()).await?;
                    }
                }
            }

            // ── New job pushed from template loop ─────────────────────────
            Ok(template) = job_rx.recv() => {
                if let Some(ref s) = session {
                    if s.is_authorized() || session.is_some() {
                        template_store.lock().unwrap().insert(template.job_id.clone(), template.clone());

                        let diff_msg = serde_json::json!({
                            "id": null,
                            "method": "mining.set_difficulty",
                            "params": [s.difficulty]
                        });
                        framed.send(diff_msg.to_string()).await?;

                        let notify = build_notify(&template);
                        framed.send(notify.to_string()).await?;
                    }
                }
            }
        }
    }
}

/// Compute the header hash string (for the found-block case)
fn compute_header_hash_str(template: &MiningTemplate, nonce: u64) -> String {
    let header = build_header_for_nonce(template, nonce);
    Astram_core::block::compute_header_hash(&header).unwrap_or_default()
}

fn build_notify(t: &MiningTemplate) -> Value {
    serde_json::json!({
        "id": null,
        "method": "mining.notify",
        "params": [
            t.job_id,
            t.height,
            t.prev_hash,
            t.merkle_root,
            t.timestamp,
            t.difficulty,
            pool_diff_to_target(t.pool_diff)
        ]
    })
}

// ─── Template polling loop (detects new blocks immediately) ──────────────────

async fn run_template_loop(
    client: NodeClient,
    pool_address: String,
    templates: TemplateStore,
    job_tx: broadcast::Sender<MiningTemplate>,
    mut force_rebuild_rx: watch::Receiver<u64>,
) {
    let mut last_tip = String::new();
    let mut last_job_time = 0i64;

    loop {
        // Wait up to 500 ms OR wake immediately when force_rebuild is signalled
        // (e.g. right after a block is submitted by the stratum handler).
        tokio::select! {
            _ = sleep(Duration::from_millis(500)) => {}
            _ = force_rebuild_rx.changed() => {}
        }

        let now = chrono::Utc::now().timestamp();
        let status = client.fetch_status().await;

        let should_rebuild = match &status {
            Ok(s) => {
                // Rebuild immediately on new block OR every 15s for fresh mempool
                s.tip_hash != last_tip || (now - last_job_time) >= 15
            }
            Err(_) => false,
        };

        if should_rebuild {
            let job_id = format!("{}", chrono::Utc::now().timestamp_millis());
            match build_template(&client, &pool_address, job_id.clone()).await {
                Ok(template) => {
                    let new_tip = template.prev_hash.clone();
                    let is_new_block = new_tip != last_tip;
                    last_tip = new_tip;
                    last_job_time = now;

                    templates.lock().unwrap().insert(job_id, template.clone());
                    let _ = job_tx.send(template);

                    if is_new_block {
                        log::info!("📦 New block detected – job refreshed immediately");
                    }
                }
                Err(e) => {
                    log::warn!("failed to build template: {}", e);
                }
            }
        }
    }
}

// ─── Stratum server ───────────────────────────────────────────────────────────

async fn run_stratum_server(
    pool_cfg: Arc<PoolConfig>,
    client: NodeClient,
    tracker: SharedTracker,
    active_connections: ActiveConnections,
) -> Result<()> {
    let listener = TcpListener::bind(&pool_cfg.stratum_bind).await?;
    let templates: TemplateStore = Arc::new(Mutex::new(HashMap::new()));
    let (job_tx, _) = broadcast::channel::<MiningTemplate>(32);
    // force_rebuild: stratum handler signals this after block submission so the
    // template loop rebuilds immediately instead of waiting up to 1 s.
    let (force_rebuild_tx, force_rebuild_rx) = watch::channel::<u64>(0);

    // Spawn template polling loop
    tokio::spawn(run_template_loop(
        client.clone(),
        pool_cfg.pool_address.clone(),
        templates.clone(),
        job_tx.clone(),
        force_rebuild_rx,
    ));

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        log::info!("[STRATUM] Connection from {}", peer_addr);

        let job_rx = job_tx.subscribe();
        let templates = templates.clone();
        let client = client.clone();
        let pool_cfg = pool_cfg.clone();
        let tracker = tracker.clone();
        let ac = active_connections.clone();
        let rebuild_tx = force_rebuild_tx.clone();

        tokio::spawn(async move {
            ac.fetch_add(1, AtomicOrdering::Relaxed);
            let result = handle_stratum_connection(stream, templates, job_rx, pool_cfg, client, tracker, rebuild_tx).await;
            ac.fetch_sub(1, AtomicOrdering::Relaxed);
            if let Err(e) = result {
                log::debug!("[STRATUM] {} disconnected: {}", peer_addr, e);
            }
        });
    }
}

// ─── GBT server ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JsonRpcRequest {
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self { jsonrpc: "1.0".to_string(), id, result: Some(result), error: None }
    }
    fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "1.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message: message.into() }),
        }
    }
}

async fn run_gbt_server(
    bind_addr: String,
    client: NodeClient,
    pool_address: String,
    tracker: SharedTracker,
) -> Result<()> {
    let route = warp::post()
        .and(warp::body::json())
        .and_then(move |request: JsonRpcRequest| {
            let client = client.clone();
            let pool_address = pool_address.clone();
            let tracker = tracker.clone();
            async move {
                let id = request.id.clone();
                match request.method.as_str() {
                    "getblocktemplate" => {
                        let job_id = format!("{}", chrono::Utc::now().timestamp_millis());
                        match build_template(&client, &pool_address, job_id).await {
                            Ok(template) => {
                                let txs = template
                                    .transactions
                                    .iter()
                                    .skip(1)
                                    .map(|tx| {
                                        let bytes =
                                            bincode::encode_to_vec(tx, *BINCODE_CONFIG).unwrap_or_default();
                                        serde_json::json!({
                                            "data": hex::encode(bytes),
                                            "txid": tx.txid,
                                            "hash": tx.txid
                                        })
                                    })
                                    .collect::<Vec<_>>();

                                let result = serde_json::json!({
                                    "version": 1,
                                    "previousblockhash": template.prev_hash,
                                    "transactions": txs,
                                    "coinbasevalue": template.coinbase_value.to_string(),
                                    "target": pool_diff_to_target(template.pool_diff),
                                    "mintime": template.timestamp,
                                    "curtime": template.timestamp,
                                    "height": template.height,
                                    "mutable": ["time", "transactions", "prevblock"],
                                    "noncerange": "00000000ffffffff",
                                    "capabilities": ["proposal"],
                                    "longpollid": format!("{}:{}", template.height, template.merkle_root)
                                });
                                Ok::<_, warp::Rejection>(warp::reply::json(
                                    &JsonRpcResponse::success(id, result),
                                ))
                            }
                            Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(
                                &JsonRpcResponse::error(id, -32000, format!("template error: {}", e)),
                            )),
                        }
                    }
                    "submitblock" => {
                        let params = request
                            .params
                            .and_then(|v| v.as_array().cloned())
                            .unwrap_or_default();
                        let data = params.get(0).and_then(|v| v.as_str()).unwrap_or("");
                        match decode_block_payload(data) {
                            Ok(block) => match client.submit_block(&block).await {
                                Ok(_) => {
                                    // GBT miner found a block – credit the pool address
                                    let _credits = {
                                        let mut t = tracker.lock().unwrap();
                                        t.distribute_pplns(
                                            U256::zero(), // reward unknown here
                                            0.0,
                                            &pool_address,
                                            block.header.index,
                                            block.hash.clone(),
                                            chrono::Utc::now().timestamp(),
                                        )
                                    };
                                    Ok::<_, warp::Rejection>(warp::reply::json(
                                        &JsonRpcResponse::success(id, Value::Null),
                                    ))
                                }
                                Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(
                                    &JsonRpcResponse::error(id, -32001, format!("submit failed: {}", e)),
                                )),
                            },
                            Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(
                                &JsonRpcResponse::error(id, -32602, format!("invalid block: {}", e)),
                            )),
                        }
                    }
                    _ => Ok::<_, warp::Rejection>(warp::reply::json(
                        &JsonRpcResponse::error(id, -32601, "method not found"),
                    )),
                }
            }
        })
        .with(warp::log("Astram::gbt"));

    let addr: std::net::SocketAddr = bind_addr.parse()?;
    warp::serve(route).run(addr).await;
    Ok(())
}

// ─── Stats HTTP API ───────────────────────────────────────────────────────────

/// HTML dashboard embedded at compile time from web/index.html
static DASHBOARD_HTML: &str = include_str!("../web/index.html");

async fn run_stats_server(bind_addr: String, tracker: SharedTracker, active_connections: ActiveConnections) -> Result<()> {
    // Serve the dashboard at /
    let index = warp::path::end()
        .and(warp::get())
        .map(|| warp::reply::html(DASHBOARD_HTML.to_string()));

    let t1 = tracker.clone();
    let ac1 = active_connections.clone();
    let stats = warp::path!("stats").and(warp::get()).map(move || {
        let t = t1.lock().unwrap();
        warp::reply::json(&serde_json::json!({
            "total_shares_accepted": t.total_shares_accepted,
            "total_shares_rejected": t.total_shares_rejected,
            "blocks_found": t.found_blocks.len(),
            "pplns_window_shares": t.recent_shares.len(),
            "active_connections": ac1.load(AtomicOrdering::Relaxed),
        }))
    });

    let t2 = tracker.clone();
    let miners = warp::path!("miners").and(warp::get()).map(move || {
        let t = t2.lock().unwrap();
        warp::reply::json(&t.miner_stats())
    });

    let t3 = tracker.clone();
    let blocks = warp::path!("blocks").and(warp::get()).map(move || {
        let t = t3.lock().unwrap();
        let recent: Vec<FoundBlock> = t.found_blocks.iter().rev().take(50).cloned().collect();
        warp::reply::json(&recent)
    });

    let t4 = tracker.clone();
    let payments = warp::path!("payments").and(warp::get()).map(move || {
        let t = t4.lock().unwrap();
        let balances: HashMap<String, String> = t
            .balances
            .iter()
            .map(|(k, v)| (k.clone(), format!("0x{:x}", v)))
            .collect();
        warp::reply::json(&balances)
    });

    let routes = index.or(stats).or(miners).or(blocks).or(payments);
    let addr: std::net::SocketAddr = bind_addr.parse()?;
    log::info!("Stats API listening on http://{}", addr);
    warp::serve(routes).run(addr).await;
    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Load the pool wallet keypair (used to sign payout transactions).
/// Priority: POOL_WALLET_SECRET env var → wallet file secret_key field.
fn load_pool_keypair(cfg: &Config) -> Result<WalletKeypair> {
    if let Ok(secret) = std::env::var("POOL_WALLET_SECRET") {
        return WalletKeypair::from_secret_hex(&secret)
            .map_err(|e| anyhow!("invalid POOL_WALLET_SECRET: {}", e));
    }
    let wallet_path = cfg.wallet_path_resolved();
    let data = std::fs::read_to_string(&wallet_path)
        .map_err(|e| anyhow!("cannot read wallet file {:?}: {}", wallet_path, e))?;
    let wallet: serde_json::Value = serde_json::from_str(&data)?;
    let secret = wallet["secret_key"]
        .as_str()
        .ok_or_else(|| anyhow!("no 'secret_key' field in wallet file"))?;
    WalletKeypair::from_secret_hex(secret)
        .map_err(|e| anyhow!("invalid secret_key in wallet file: {}", e))
}

fn load_pool_address(cfg: &Config) -> Result<String> {
    let wallet_path = cfg.wallet_path_resolved();
    let wallet_file = std::fs::read_to_string(wallet_path)
        .map_err(|e| anyhow!("failed to read wallet file: {}", e))?;
    let wallet: Value = serde_json::from_str(&wallet_file)
        .map_err(|e| anyhow!("failed to parse wallet JSON: {}", e))?;
    wallet
        .get("address")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("wallet address missing"))
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cfg = Config::load();
    let pool_cfg = Arc::new(PoolConfig::from_env(&cfg)?);
    let client = NodeClient::new(pool_cfg.node_rpc_url.clone());

    // ── Payout system setup ──────────────────────────────────────────────────
    let keypair = Arc::new(load_pool_keypair(&cfg)?);

    let payout_db = Arc::new(
        PayoutDb::open(&pool_cfg.payout_db_path)
            .map_err(|e| anyhow!("failed to open payout DB: {}", e))?,
    );

    // Initialise tracker and load persisted balances
    let tracker: SharedTracker = Arc::new(Mutex::new(ShareTracker::new(pool_cfg.pplns_window)));
    {
        let saved = payout_db.load_all().unwrap_or_default();
        if !saved.is_empty() {
            let mut t = tracker.lock().unwrap();
            for (addr, bal) in &saved {
                t.balances.insert(addr.clone(), *bal);
            }
            log::info!("💾 Loaded {} pending balances from DB", saved.len());
        }
    }

    let active_connections: ActiveConnections = Arc::new(AtomicU64::new(0));

    log::info!("🏊 Astram Mining Pool starting...");
    log::info!("  Stratum   : {}", pool_cfg.stratum_bind);
    log::info!("  GBT       : {}", pool_cfg.gbt_bind);
    log::info!("  Stats API : {}", pool_cfg.stats_bind);
    log::info!("  Node RPC  : {}", pool_cfg.node_rpc_url);
    log::info!("  Pool addr : {}", pool_cfg.pool_address);
    log::info!("  Pool fee  : {}%", pool_cfg.pool_fee_percent);
    log::info!("  PPLNS win : {} shares", pool_cfg.pplns_window);
    log::info!(
        "  VarDiff   : {}–{} leading zeros, target {}s/share",
        pool_cfg.vardiff.min_diff,
        pool_cfg.vardiff.max_diff,
        pool_cfg.vardiff.target_share_time
    );
    log::info!(
        "  Payout    : threshold={} ram, interval={}s",
        pool_cfg.payout_threshold_ram,
        pool_cfg.payout_interval_secs
    );

    // Spawn GBT server
    {
        let gbt_client = client.clone();
        let gbt_pool = pool_cfg.pool_address.clone();
        let gbt_bind = pool_cfg.gbt_bind.clone();
        let gbt_tracker = tracker.clone();
        tokio::spawn(async move {
            if let Err(e) = run_gbt_server(gbt_bind, gbt_client, gbt_pool, gbt_tracker).await {
                log::error!("GBT server failed: {}", e);
            }
        });
    }

    // Spawn stats API server
    {
        let stats_bind = pool_cfg.stats_bind.clone();
        let stats_tracker = tracker.clone();
        let stats_ac = active_connections.clone();
        tokio::spawn(async move {
            if let Err(e) = run_stats_server(stats_bind, stats_tracker, stats_ac).await {
                log::error!("Stats server failed: {}", e);
            }
        });
    }

    // Spawn balance persistence task (syncs balances to DB every 30 s)
    {
        let sync_tracker = tracker.clone();
        let sync_db = payout_db.clone();
        tokio::spawn(async move {
            run_balance_sync(sync_tracker, sync_db).await;
        });
    }

    // Spawn payout loop
    {
        let pay_http = client.client.clone();
        let pay_url = pool_cfg.node_rpc_url.clone();
        let pay_tracker = tracker.clone();
        let pay_db = payout_db.clone();
        let pay_kp = keypair.clone();
        let pay_addr = pool_cfg.pool_address.clone();
        let pay_threshold = pool_cfg.payout_threshold_ram;
        let pay_interval = pool_cfg.payout_interval_secs;
        tokio::spawn(async move {
            run_payout_loop(
                pay_http, pay_url, pay_tracker, pay_db,
                pay_kp, pay_addr, pay_threshold, pay_interval,
            ).await;
        });
    }

    // Run stratum server (blocking)
    run_stratum_server(pool_cfg, client, tracker, active_connections).await
}
