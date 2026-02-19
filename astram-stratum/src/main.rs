use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use futures::{SinkExt, StreamExt};
use astram_config::config::Config;
use Astram_core::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use Astram_core::config::initial_block_reward;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction};
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};
use tokio_util::codec::{Framed, LinesCodec};
use warp::Filter;

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
    difficulty: u32,
    tip_hash: String,
}

#[derive(Debug, Clone)]
struct MiningTemplate {
    job_id: String,
    height: u64,
    prev_hash: String,
    difficulty: u32,
    timestamp: i64,
    merkle_root: String,
    transactions: Vec<Transaction>,
    coinbase_value: U256,
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
        Self {
            jsonrpc: "1.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "1.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
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

        let height = value
            .get("blockchain")
            .and_then(|v| v.get("height"))
            .and_then(|v| v.as_u64())
            .or_else(|| value.get("height").and_then(|v| v.as_u64()))
            .unwrap_or(0);

        let difficulty = value
            .get("blockchain")
            .and_then(|v| v.get("difficulty"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        let tip_hash = value
            .get("blockchain")
            .and_then(|v| v.get("chain_tip"))
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();

        Ok(ChainStatus {
            height,
            difficulty,
            tip_hash,
        })
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
            Err(anyhow!(
                resp.message.unwrap_or_else(|| "submit failed".to_string())
            ))
        }
    }
}

fn parse_u256(value: &str) -> Option<U256> {
    if let Some(hex) = value.strip_prefix("0x") {
        return U256::from_str_radix(hex, 16).ok();
    }
    U256::from_dec_str(value).ok()
}

fn target_from_difficulty(difficulty: u32) -> String {
    let zeros = difficulty.min(64) as usize;
    let rest = 64usize.saturating_sub(zeros);
    format!("{}{}", "0".repeat(zeros), "f".repeat(rest))
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

async fn build_template(
    client: &NodeClient,
    pool_address: &str,
    job_id: String,
) -> Result<MiningTemplate> {
    let status = client.fetch_status().await?;
    let mempool = client.fetch_mempool().await?;

    let height = status.height as u64;
    let prev_hash = if status.tip_hash == "none" {
        "0".repeat(64)
    } else {
        status.tip_hash.clone()
    };

    let base_reward = initial_block_reward();
    let coinbase_value = base_reward + mempool.total_fees;

    let coinbase = Transaction::coinbase(pool_address, coinbase_value).with_hashes();
    let mut all_txs = vec![coinbase];
    all_txs.extend(mempool.txs);

    let txids: Vec<String> = all_txs.iter().map(|t| t.txid.clone()).collect();
    let merkle_root = compute_merkle_root(&txids);

    Ok(MiningTemplate {
        job_id,
        height,
        prev_hash,
        difficulty: status.difficulty,
        timestamp: chrono::Utc::now().timestamp(),
        merkle_root,
        transactions: all_txs,
        coinbase_value,
    })
}

fn build_block_from_template(template: &MiningTemplate, nonce: u64) -> Result<Block> {
    let header = BlockHeader {
        index: template.height,
        previous_hash: template.prev_hash.clone(),
        merkle_root: template.merkle_root.clone(),
        timestamp: template.timestamp,
        nonce,
        difficulty: template.difficulty,
    };

    let hash = compute_header_hash(&header)?;
    if !hash.starts_with(&"0".repeat(template.difficulty as usize)) {
        return Err(anyhow!("nonce does not meet difficulty"));
    }

    Ok(Block {
        header,
        transactions: template.transactions.clone(),
        hash,
    })
}

async fn handle_stratum_connection(
    stream: TcpStream,
    template_store: Arc<Mutex<HashMap<String, MiningTemplate>>>,
    mut job_rx: broadcast::Receiver<MiningTemplate>,
    pool_address: String,
    client: NodeClient,
) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new());
    let mut subscribed = false;

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
                    "mining.subscribe" => {
                        subscribed = true;
                        let extranonce1 = hex::encode(rand::random::<u32>().to_be_bytes());
                        let extranonce2_size = 4;
                        let result = serde_json::json!([
                            [["mining.set_difficulty", "1"], ["mining.notify", "1"]],
                            extranonce1,
                            extranonce2_size
                        ]);
                        let resp = serde_json::json!({"id": id, "result": result, "error": null});
                        framed.send(resp.to_string()).await?;

                        let new_job_id = format!("{}", chrono::Utc::now().timestamp_millis());
                        let template = build_template(&client, &pool_address, new_job_id.clone()).await?;
                        template_store.lock().unwrap().insert(new_job_id.clone(), template.clone());
                        let notify = serde_json::json!({
                            "id": null,
                            "method": "mining.notify",
                            "params": [
                                template.job_id,
                                template.prev_hash,
                                template.merkle_root,
                                template.timestamp,
                                template.difficulty,
                                target_from_difficulty(template.difficulty)
                            ]
                        });
                        framed.send(notify.to_string()).await?;
                    }
                    "mining.authorize" => {
                        let resp = serde_json::json!({"id": id, "result": true, "error": null});
                        framed.send(resp.to_string()).await?;
                    }
                    "mining.submit" => {
                        let params = params.and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let job_id = params.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let nonce_str = params.get(2).and_then(|v| v.as_str()).unwrap_or("");

                        let nonce = parse_nonce(nonce_str)?;
                        let template = {
                            let guard = template_store.lock().unwrap();
                            guard.get(&job_id).cloned()
                        };

                        if let Some(template) = template {
                            let block = build_block_from_template(&template, nonce)?;
                            match client.submit_block(&block).await {
                                Ok(_) => {
                                    let resp = serde_json::json!({"id": id, "result": true, "error": null});
                                    framed.send(resp.to_string()).await?;
                                }
                                Err(e) => {
                                    let resp = serde_json::json!({"id": id, "result": false, "error": e.to_string()});
                                    framed.send(resp.to_string()).await?;
                                }
                            }
                        } else {
                            let resp = serde_json::json!({"id": id, "result": false, "error": "unknown job"});
                            framed.send(resp.to_string()).await?;
                        }
                    }
                    _ => {
                        let resp = serde_json::json!({"id": id, "result": null, "error": "unsupported method"});
                        framed.send(resp.to_string()).await?;
                    }
                }
            }
            Ok(template) = job_rx.recv() => {
                if subscribed {
                    template_store.lock().unwrap().insert(template.job_id.clone(), template.clone());
                    let notify = serde_json::json!({
                        "id": null,
                        "method": "mining.set_difficulty",
                        "params": [template.difficulty]
                    });
                    framed.send(notify.to_string()).await?;

                    let notify = serde_json::json!({
                        "id": null,
                        "method": "mining.notify",
                        "params": [
                            template.job_id,
                            template.prev_hash,
                            template.merkle_root,
                            template.timestamp,
                            template.difficulty,
                            target_from_difficulty(template.difficulty)
                        ]
                    });
                    framed.send(notify.to_string()).await?;
                }
            }
        }
    }
}

fn parse_nonce(nonce_str: &str) -> Result<u64> {
    if let Some(hex) = nonce_str.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).map_err(|e| anyhow!("invalid nonce: {}", e));
    }

    if nonce_str.chars().all(|c| c.is_ascii_digit()) {
        return nonce_str
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid nonce: {}", e));
    }

    u64::from_str_radix(nonce_str, 16).map_err(|e| anyhow!("invalid nonce: {}", e))
}

async fn run_stratum_server(
    bind_addr: &str,
    client: NodeClient,
    pool_address: String,
) -> Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    let templates: Arc<Mutex<HashMap<String, MiningTemplate>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (job_tx, _) = broadcast::channel(16);

    let client_for_jobs = client.clone();
    let pool_for_jobs = pool_address.clone();
    let templates_for_jobs = templates.clone();
    let job_tx_for_task = job_tx.clone();
    tokio::spawn(async move {
        loop {
            let job_id = format!("{}", chrono::Utc::now().timestamp_millis());
            match build_template(&client_for_jobs, &pool_for_jobs, job_id.clone()).await {
                Ok(template) => {
                    templates_for_jobs
                        .lock()
                        .unwrap()
                        .insert(job_id, template.clone());
                    let _ = job_tx_for_task.send(template);
                }
                Err(e) => {
                    log::warn!("failed to build template: {}", e);
                }
            }
            sleep(Duration::from_secs(15)).await;
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let job_rx = job_tx.subscribe();
        let templates = templates.clone();
        let client = client.clone();
        let pool_address = pool_address.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_stratum_connection(stream, templates, job_rx, pool_address, client).await
            {
                log::warn!("stratum connection closed: {}", e);
            }
        });
    }
}

async fn run_gbt_server(bind_addr: &str, client: NodeClient, pool_address: String) -> Result<()> {
    let route = warp::post()
        .and(warp::body::json())
        .and_then(move |request: JsonRpcRequest| {
            let client = client.clone();
            let pool_address = pool_address.clone();
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
                                        let bytes = bincode::encode_to_vec(tx, *BINCODE_CONFIG)
                                            .unwrap_or_default();
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
                                    "target": target_from_difficulty(template.difficulty),
                                    "mintime": template.timestamp,
                                    "curtime": template.timestamp,
                                    "height": template.height,
                                    "mutable": ["time", "transactions", "prevblock"],
                                    "noncerange": "00000000ffffffff",
                                    "capabilities": ["proposal"],
                                    "longpollid": format!("{}:{}", template.height, template.merkle_root)
                                });

                                Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::success(id, result)))
                            }
                            Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::error(
                                id,
                                -32000,
                                format!("template error: {}", e),
                            ))),
                        }
                    }
                    "submitblock" => {
                        let params = request.params.and_then(|v| v.as_array().cloned()).unwrap_or_default();
                        let data = params.get(0).and_then(|v| v.as_str()).unwrap_or("");
                        match decode_block_payload(data) {
                            Ok(block) => {
                                match client.submit_block(&block).await {
                                    Ok(_) => Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::success(id, Value::Null))),
                                    Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::error(
                                        id,
                                        -32001,
                                        format!("submit failed: {}", e),
                                    ))),
                                }
                            }
                            Err(e) => Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::error(
                                id,
                                -32602,
                                format!("invalid block: {}", e),
                            ))),
                        }
                    }
                    _ => Ok::<_, warp::Rejection>(warp::reply::json(&JsonRpcResponse::error(
                        id,
                        -32601,
                        "method not found",
                    ))),
                }
            }
        })
        .with(warp::log("Astram::gbt"));

    let addr: std::net::SocketAddr = bind_addr.parse()?;
    warp::serve(route).run(addr).await;
    Ok(())
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cfg = Config::load();
    let node_url = std::env::var("NODE_RPC_URL").unwrap_or(cfg.node_rpc_url.clone());

    let pool_address = std::env::var("POOL_ADDRESS")
        .ok()
        .or_else(|| load_pool_address(&cfg).ok())
        .ok_or_else(|| anyhow!("POOL_ADDRESS not set and wallet missing"))?;

    let stratum_bind = std::env::var("STRATUM_BIND").unwrap_or_else(|_| "0.0.0.0:3333".to_string());
    let gbt_bind = std::env::var("GBT_BIND").unwrap_or_else(|_| "0.0.0.0:8332".to_string());

    let client = NodeClient::new(node_url.clone());

    let gbt_client = client.clone();
    let gbt_pool = pool_address.clone();
    let gbt_bind_for_task = gbt_bind.clone();
    tokio::spawn(async move {
        if let Err(e) = run_gbt_server(&gbt_bind_for_task, gbt_client, gbt_pool).await {
            log::error!("GBT server failed: {}", e);
        }
    });

    log::info!("Stratum server listening on {}", stratum_bind);
    log::info!("GBT server listening on {}", gbt_bind);
    log::info!("Using node RPC at {}", node_url);

    run_stratum_server(&stratum_bind, client, pool_address).await
}

