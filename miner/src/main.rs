/// Astram Standalone Miner
///
/// Modes:
///   solo  – polls node HTTP API directly, builds block template, mines, submits block
///   pool  – connects to astram-stratum pool via Stratum protocol, finds nonces, submits shares
///
/// Config file: config/minerSettings.conf  (next to the binary, or in cwd/config/)
///
///   MINING_MODE=solo          # solo | pool  (default: solo)
///   NODE_RPC_URL=http://127.0.0.1:19533
///   POOL_HOST=127.0.0.1
///   POOL_PORT=3333
///   WORKER_NAME=worker1

#[cfg(not(feature = "cuda-miner"))]
compile_error!("Astram-miner requires CUDA. Build with `--features cuda-miner`.");

use anyhow::{Result, anyhow};
use astram_config::config::Config;
use base64::{Engine as _, engine::general_purpose};
use Astram_core::block::{Block, BlockHeader};
use Astram_core::config::calculate_block_reward;
use Astram_core::consensus;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction};
use futures::{SinkExt, StreamExt};
use primitive_types::U256;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep};
use tokio_util::codec::{Framed, LinesCodec};

// ─── Settings ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum MiningMode {
    Solo,
    Pool,
}

#[derive(Debug, Clone)]
struct MinerSettings {
    mode: MiningMode,
    node_rpc_url: String,
    pool_host: String,
    pool_port: u16,
    worker_name: String,
    miner_address: String,
}

impl MinerSettings {
    fn load(cfg: &Config) -> Result<Self> {
        let mut mode = MiningMode::Solo;
        let mut node_rpc_url = cfg.node_rpc_url.clone();
        let mut pool_host = "127.0.0.1".to_string();
        let mut pool_port: u16 = 3333;
        let mut worker_name = "worker1".to_string();

        let conf_path = resolve_conf_path();
        if let Ok(contents) = std::fs::read_to_string(&conf_path) {
            for raw_line in contents.lines() {
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    match key.trim() {
                        "MINING_MODE" => {
                            mode = match value.trim().to_lowercase().as_str() {
                                "pool" => MiningMode::Pool,
                                _ => MiningMode::Solo,
                            };
                        }
                        "NODE_RPC_URL" => node_rpc_url = value.trim().to_string(),
                        "POOL_HOST" => pool_host = value.trim().to_string(),
                        "POOL_PORT" => pool_port = value.trim().parse().unwrap_or(pool_port),
                        "WORKER_NAME" => worker_name = value.trim().to_string(),
                        _ => {}
                    }
                }
            }
        } else {
            println!("[WARN] minerSettings.conf not found at {:?}, using defaults", conf_path);
        }

        let wallet_path = cfg.wallet_path_resolved();
        let wallet_file = std::fs::read_to_string(wallet_path.as_path())
            .map_err(|e| anyhow!("Failed to read wallet file {:?}: {}", wallet_path, e))?;
        let wallet: Value = serde_json::from_str(&wallet_file)?;
        let miner_address = wallet["address"]
            .as_str()
            .ok_or_else(|| anyhow!("No 'address' field in wallet file"))?
            .to_string();

        Ok(Self { mode, node_rpc_url, pool_host, pool_port, worker_name, miner_address })
    }
}

fn resolve_conf_path() -> PathBuf {
    let exe_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config/minerSettings.conf")));

    if let Some(ref p) = exe_path {
        if p.exists() {
            return p.clone();
        }
    }

    let cwd = PathBuf::from("config/minerSettings.conf");
    if cwd.exists() {
        return cwd;
    }

    exe_path.unwrap_or(cwd)
}

// ─── Node HTTP client ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ChainStatus {
    height: u64,
    difficulty: u32,
    tip_hash: String,
}

#[derive(Deserialize)]
struct MempoolResponse {
    transactions_b64: String,
    total_fees: String,
}

fn parse_u256(s: &str) -> Option<U256> {
    if let Some(hex) = s.strip_prefix("0x") {
        return U256::from_str_radix(hex, 16).ok();
    }
    U256::from_dec_str(s).ok()
}

async fn fetch_status(client: &reqwest::Client, base_url: &str) -> Result<ChainStatus> {
    let url = format!("{}/status", base_url);
    let v: Value = client.get(&url).send().await?.json().await?;

    let height = v.get("blockchain")
        .and_then(|b| b.get("height"))
        .and_then(|h| h.as_u64())
        .unwrap_or(0);
    let difficulty = v.get("blockchain")
        .and_then(|b| b.get("difficulty"))
        .and_then(|d| d.as_u64())
        .unwrap_or(1) as u32;
    let tip_hash = v.get("blockchain")
        .and_then(|b| b.get("chain_tip"))
        .and_then(|t| t.as_str())
        .unwrap_or("none")
        .to_string();

    Ok(ChainStatus { height, difficulty, tip_hash })
}

async fn fetch_mempool(client: &reqwest::Client, base_url: &str) -> Result<(Vec<Transaction>, U256)> {
    let url = format!("{}/mempool", base_url);
    let resp: MempoolResponse = client.get(&url).send().await?.json().await?;

    let bytes = general_purpose::STANDARD.decode(resp.transactions_b64.as_bytes())?;
    let (txs, _) = bincode::decode_from_slice::<Vec<Transaction>, _>(&bytes, *BINCODE_CONFIG)
        .map_err(|e| anyhow!("mempool decode: {}", e))?;
    let total_fees = parse_u256(&resp.total_fees).unwrap_or_else(U256::zero);

    Ok((txs, total_fees))
}

async fn submit_block(client: &reqwest::Client, base_url: &str, block: &Block) -> Result<()> {
    let bytes = bincode::encode_to_vec(block, *BINCODE_CONFIG)?;
    let payload = serde_json::json!({ "block_b64": general_purpose::STANDARD.encode(&bytes) });
    let url = format!("{}/mining/submit", base_url);
    let resp: Value = client.post(&url).json(&payload).send().await?.json().await?;

    if resp.get("status").and_then(|s| s.as_str()) == Some("ok") {
        Ok(())
    } else {
        let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        Err(anyhow!("submit rejected: {}", msg))
    }
}

// ─── Solo mining ──────────────────────────────────────────────────────────────

async fn run_solo(settings: MinerSettings) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client");

    let hashrate: Arc<Mutex<f64>> = Arc::new(Mutex::new(0.0));

    println!("[SOLO] Starting solo miner → node: {}", settings.node_rpc_url);
    println!("[SOLO] Miner address: {}", settings.miner_address);

    loop {
        // 1. Fetch chain status
        let status = match fetch_status(&http, &settings.node_rpc_url).await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[SOLO] Failed to fetch status: {}", e);
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // 2. Fetch mempool
        let (mempool_txs, total_fees) = match fetch_mempool(&http, &settings.node_rpc_url).await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[SOLO] Failed to fetch mempool: {}", e);
                (Vec::new(), U256::zero())
            }
        };

        let next_height = status.height + 1;
        let prev_hash = if status.tip_hash == "none" { "0".repeat(64) } else { status.tip_hash.clone() };
        let reward = calculate_block_reward(next_height) + total_fees;

        println!(
            "[SOLO] Mining block #{} | diff=0x{:08x} | txs={} | reward={} wei",
            next_height, status.difficulty, mempool_txs.len(), reward
        );

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_for_poll = cancel_flag.clone();

        // 3. Poll for new block while mining (cancel if chain tip changes)
        let node_url = settings.node_rpc_url.clone();
        let current_tip = status.tip_hash.clone();
        let http_poll = http.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(3)).await;
                if cancel_for_poll.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(new_status) = fetch_status(&http_poll, &node_url).await {
                    if new_status.tip_hash != current_tip {
                        log::info!("[SOLO] New block detected, cancelling mining...");
                        cancel_for_poll.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        });

        // 4. Mine (blocking CUDA)
        let cancel_for_mine = cancel_flag.clone();
        let hr = hashrate.clone();
        let miner_addr = settings.miner_address.clone();
        let diff = status.difficulty;

        let mine_result: Result<Block> = tokio::task::spawn_blocking(move || {
            consensus::mine_block_with_coinbase_cuda(
                next_height,
                prev_hash,
                diff,
                mempool_txs,
                &miner_addr,
                reward,
                cancel_for_mine,
                Some(hr),
            )
        })
        .await
        .map_err(|e| anyhow!("mining task panic: {}", e))
        .and_then(|r| r);

        cancel_flag.store(true, Ordering::Relaxed); // stop poll task

        match mine_result {
            Ok(block) => {
                println!("[SOLO] ✅ Block found! #{} hash={}", block.header.index, &block.hash[..16]);
                match submit_block(&http, &settings.node_rpc_url, &block).await {
                    Ok(_) => println!("[SOLO] Block submitted successfully"),
                    Err(e) => log::warn!("[SOLO] Submit failed: {}", e),
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("cancelled") {
                    log::info!("[SOLO] Mining cancelled (new block)");
                } else {
                    log::error!("[SOLO] Mining error: {}", e);
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
}

// ─── Pool / Stratum mining ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct StratumJob {
    job_id: String,
    height: u64,
    prev_hash: String,
    merkle_root: String,
    timestamp: i64,
    difficulty: u32,
}

async fn run_pool(settings: MinerSettings) {
    println!("[POOL] Starting pool miner → {}:{}", settings.pool_host, settings.pool_port);
    println!("[POOL] Miner address: {}", settings.miner_address);

    loop {
        let pool_addr = format!("{}:{}", settings.pool_host, settings.pool_port);

        let stream = match TcpStream::connect(&pool_addr).await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[POOL] Connection failed: {}", e);
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        println!("[POOL] Connected to {}", pool_addr);

        if let Err(e) = run_stratum_session(stream, &settings).await {
            log::warn!("[POOL] Session ended: {}", e);
        }

        println!("[POOL] Reconnecting in 5 seconds...");
        sleep(Duration::from_secs(5)).await;
    }
}

async fn run_stratum_session(stream: TcpStream, settings: &MinerSettings) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new());
    let worker_login = format!("{}.{}", settings.miner_address, settings.worker_name);

    // Subscribe
    let subscribe = serde_json::json!({
        "id": 1,
        "method": "mining.subscribe",
        "params": [format!("Astram-miner/{}", env!("CARGO_PKG_VERSION"))]
    });
    framed.send(subscribe.to_string()).await?;

    // Authorize
    let authorize = serde_json::json!({
        "id": 2,
        "method": "mining.authorize",
        "params": [worker_login, "x"]
    });
    framed.send(authorize.to_string()).await?;

    let hashrate: Arc<Mutex<f64>> = Arc::new(Mutex::new(0.0));
    let mut current_job: Option<StratumJob> = None;
    let mut _pool_diff: u32 = 1; // share difficulty sent by pool (informational)
    let mut cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let mut mining_handle: Option<tokio::task::JoinHandle<Result<(u64, String)>>> = None;

    loop {
        tokio::select! {
            // ── Network message from pool ──────────────────────────────────
            maybe_line = framed.next() => {
                let line = match maybe_line {
                    Some(Ok(l)) => l,
                    Some(Err(e)) => return Err(anyhow!("stream error: {}", e)),
                    None => return Ok(()),
                };

                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

                match method {
                    "mining.set_difficulty" => {
                        if let Some(d) = msg.get("params")
                            .and_then(|p| p.as_array())
                            .and_then(|a| a.first())
                            .and_then(|v| v.as_u64())
                        {
                            _pool_diff = d as u32;
                            log::info!("[POOL] Difficulty set to {}", _pool_diff);
                        }
                    }

                    "mining.notify" => {
                        let params = match msg.get("params").and_then(|p| p.as_array()) {
                            Some(p) => p.clone(),
                            None => continue,
                        };

                        // params: [job_id, height, prev_hash, merkle_root, timestamp, difficulty, pool_target]
                        let job_id    = params.get(0).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let height    = params.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
                        let prev_hash = params.get(2).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let merkle    = params.get(3).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let timestamp = params.get(4).and_then(|v| v.as_i64()).unwrap_or(0);
                        let diff      = params.get(5).and_then(|v| v.as_u64()).unwrap_or(1) as u32;

                        log::info!("[POOL] New job {} height={} diff=0x{:08x}", job_id, height, diff);

                        // Cancel ongoing mining
                        cancel_flag.store(true, Ordering::Relaxed);
                        if let Some(h) = mining_handle.take() {
                            let _ = h.await;
                        }

                        let job = StratumJob { job_id, height, prev_hash, merkle_root: merkle, timestamp, difficulty: diff };
                        current_job = Some(job.clone());

                        // Start new mining task
                        cancel_flag = Arc::new(AtomicBool::new(false));
                        let header = BlockHeader {
                            index: job.height,
                            previous_hash: job.prev_hash.clone(),
                            merkle_root: job.merkle_root.clone(),
                            timestamp: job.timestamp,
                            nonce: 0,
                            difficulty: job.difficulty,
                        };
                        let cf = cancel_flag.clone();
                        let hr = hashrate.clone();
                        mining_handle = Some(tokio::task::spawn_blocking(move || {
                            consensus::mine_header_cuda(header, cf, Some(hr))
                        }));
                    }

                    _ => {
                        // Result messages (subscribe/authorize responses) – log errors only
                        if let Some(err) = msg.get("error") {
                            if !err.is_null() {
                                log::warn!("[POOL] Server error: {}", err);
                            }
                        }
                    }
                }
            }

            // ── Mining task completed ──────────────────────────────────────
            Some(result) = async {
                if let Some(ref mut h) = mining_handle {
                    Some(h.await)
                } else {
                    None
                }
            }, if mining_handle.is_some() => {
                mining_handle = None;

                let job = match &current_job {
                    Some(j) => j.clone(),
                    None => continue,
                };

                match result {
                    Ok(Ok((nonce, _hash))) => {
                        println!("[POOL] ✅ Share found! job={} nonce=0x{:016x}", job.job_id, nonce);

                        let submit = serde_json::json!({
                            "id": 3,
                            "method": "mining.submit",
                            "params": [
                                format!("{}.{}", settings.miner_address, settings.worker_name),
                                job.job_id,
                                format!("0x{:016x}", nonce)
                            ]
                        });
                        if let Err(e) = framed.send(submit.to_string()).await {
                            return Err(anyhow!("send submit: {}", e));
                        }

                        // Restart mining with same job (pool will send new notify on block found)
                        let new_header = BlockHeader {
                            index: job.height,
                            previous_hash: job.prev_hash.clone(),
                            merkle_root: job.merkle_root.clone(),
                            timestamp: job.timestamp,
                            nonce: nonce.wrapping_add(1),
                            difficulty: job.difficulty,
                        };
                        cancel_flag = Arc::new(AtomicBool::new(false));
                        let cf = cancel_flag.clone();
                        let hr = hashrate.clone();
                        mining_handle = Some(tokio::task::spawn_blocking(move || {
                            consensus::mine_header_cuda(new_header, cf, Some(hr))
                        }));
                    }
                    Ok(Err(e)) => {
                        let msg = e.to_string();
                        if !msg.contains("cancelled") {
                            log::error!("[POOL] Mining error: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("[POOL] Mining task panic: {}", e);
                    }
                }
            }
        }
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    println!("[INFO] Astram miner starting...");

    let cfg = Config::load();
    let settings = match MinerSettings::load(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[ERROR] Failed to load miner settings: {}", e);
            std::process::exit(1);
        }
    };

    println!("[INFO] Mode: {:?}", settings.mode);

    match settings.mode {
        MiningMode::Solo => run_solo(settings).await,
        MiningMode::Pool => run_pool(settings).await,
    }
}
