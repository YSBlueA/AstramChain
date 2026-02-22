use crate::ChainState;
use crate::NodeHandle;
use crate::NodeMeta;
use crate::PeerManager;
use base64::{Engine as _, engine::general_purpose};
use Astram_core::block::Block;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction};
use Astram_core::utxo::Utxo;
use log::info;
use primitive_types::U256;
use serde::Deserialize;
use warp::Filter;
use warp::{http::StatusCode, reply::with_status}; // bincode v2
use std::collections::HashMap;
use std::net::SocketAddr;
/// run_server expects NodeHandle (Arc<NodeHandles>)
pub async fn run_server(
    node: NodeHandle,
    p2p: std::sync::Arc<PeerManager>,
    chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>,
    node_meta: std::sync::Arc<NodeMeta>,
    bind_addr: SocketAddr,
) {
    let node_filter = {
        let node = node.clone();
        warp::any().map(move || node.clone())
    };
    let p2p_filter = {
        let p2p = p2p.clone();
        warp::any().map(move || p2p.clone())
    };
    let chain_filter = {
        let chain_state = chain_state.clone();
        warp::any().map(move || chain_state.clone())
    };
    let meta_filter = {
        let node_meta = node_meta.clone();
        warp::any().map(move || node_meta.clone())
    };

    // -------------------------------
    // GET /blockchain/memory - In-memory blockchain state
    let get_chain_memory = warp::path!("blockchain" / "memory")
        .and(warp::get())
        .and(chain_filter.clone())
        .and_then(|chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>| async move {
            let chain = chain_state.lock().unwrap();
            let bincode_bytes = bincode::encode_to_vec(&chain.blockchain, *BINCODE_CONFIG).unwrap();
            let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
            log::info!("[INFO] Returning {} blocks from memory", chain.blockchain.len());
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blockchain": encoded,
                "count": chain.blockchain.len(),
                "source": "memory"
            })))
        });

    // GET /blockchain/db - Blocks from database
    let get_chain_db = warp::path!("blockchain" / "db")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.clone();
            let bc = state.bc.lock().unwrap();
            match bc.get_all_blocks() {
                Ok(all_blocks) => {
                    let bincode_bytes =
                        bincode::encode_to_vec(&all_blocks, *BINCODE_CONFIG).unwrap();
                    let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
                    log::info!("[INFO] Returning {} blocks from DB", all_blocks.len());
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "blockchain": encoded,
                        "count": all_blocks.len(),
                        "source": "database"
                    })))
                }
                Err(e) => {
                    log::error!("[ERROR] Failed to fetch blocks from DB: {}", e);
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "error": format!("Failed to fetch blockchain from DB: {}", e),
                        "count": 0,
                        "source": "database"
                    })))
                }
            }
        });

    // GET /blockchain/range?from=0&to=10 - Blocks from specific height range
    let get_chain_range = warp::path!("blockchain" / "range")
        .and(warp::get())
        .and(warp::query::<std::collections::HashMap<String, String>>())
        .and(node_filter.clone())
        .and_then(|params: std::collections::HashMap<String, String>, node: NodeHandle| async move {
            let from_height = params.get("from").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let to_height = params.get("to").and_then(|s| s.parse::<u64>().ok());
            
            let state = node.clone();
            let bc = state.bc.lock().unwrap();
            match bc.get_blocks_range(from_height, to_height) {
                Ok(blocks) => {
                    let bincode_bytes = bincode::encode_to_vec(&blocks, *BINCODE_CONFIG).unwrap();
                    let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
                    
                    log::info!("[INFO] Returning {} blocks from DB (height {} to {:?})", 
                        blocks.len(), from_height, to_height);
                    
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "blockchain": encoded,
                        "count": blocks.len(),
                        "from": from_height,
                        "to": to_height,
                        "source": "database"
                    })))
                }
                Err(e) => {
                    log::error!("[ERROR] Failed to fetch blocks from DB: {}", e);
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "error": format!("Failed to fetch blockchain from DB: {}", e),
                        "count": 0
                    })))
                }
            }
        });

    // GET /debug/block-counts - Simple debug endpoint
    let debug_counts = warp::path!("debug" / "block-counts")
        .and(warp::get())
        .and(node_filter.clone())
        .and(chain_filter.clone())
        .and_then(|node: NodeHandle, chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>| async move {
            let state = node.clone();
            let chain = chain_state.lock().unwrap();
            let memory_count = chain.blockchain.len();
            let db_count = state
                .bc
                .lock()
                .unwrap()
                .get_all_blocks()
                .map(|b| b.len())
                .unwrap_or(0);

            log::info!(
                "[INFO] Block counts - Memory: {}, DB: {}",
                memory_count,
                db_count
            );

            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "memory": memory_count,
                "database": db_count,
                "match": memory_count == db_count
            })))
        });

    // GET /health - Health check endpoint for DNS server
    let health_check = warp::path!("health")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.clone();
            let bc = state.bc.lock().unwrap();
            let height = if let Some(tip_hash) = &bc.chain_tip {
                if let Ok(Some(header)) = bc.load_header(tip_hash) {
                    header.index + 1
                } else {
                    0
                }
            } else {
                0
            };

            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "status": "ok",
                "height": height,
                "timestamp": chrono::Utc::now().timestamp()
            })))
        });

    // GET /counts - lightweight counts for blocks and transactions (DB)
    let get_counts = warp::path("counts")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.clone();
            let bc = state.bc.lock().unwrap();
            let blocks = bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);
            let transactions = bc.count_transactions().unwrap_or(0);
            let volume = bc.calculate_total_volume().unwrap_or(U256::zero());
            log::info!(
                "Counts endpoint - blocks: {}, transactions: {}, volume: {}",
                blocks,
                transactions,
                volume
            );
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blocks": blocks,
                "transactions": transactions,
                "total_volume": format!("0x{:x}", volume)
            })))
        });

    // GET /status - Node status information (real-time monitoring)
    let get_status = warp::path("status")
        .and(warp::get())
        .and(node_filter.clone())
        .and(chain_filter.clone())
        .and(meta_filter.clone())
        .and(p2p_filter.clone())
        .and_then(|node: NodeHandle, chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>, node_meta: std::sync::Arc<NodeMeta>, p2p: std::sync::Arc<PeerManager>| async move {
            let request_start = std::time::Instant::now();
            info!("[DASHBOARD] 📊 /status request START");
            
            // Get P2P data FIRST to avoid nested lock contention
            // Use a non-blocking snapshot to keep /status responsive
            info!("[DASHBOARD] 🌐 P2P snapshot attempt...");
            let p2p_start = std::time::Instant::now();
            let (peer_heights, my_height, subnet_24_count, subnet_16_count) =
                match p2p.try_get_status_snapshot() {
                    Some(data) => {
                        info!("[DASHBOARD] ✅ P2P snapshot OK (took {:?})", p2p_start.elapsed());
                        data
                    }
                    None => {
                        log::warn!(
                            "[DASHBOARD] ⚠️ P2P snapshot contended (took {:?}), using defaults",
                            p2p_start.elapsed()
                        );
                        (HashMap::new(), 0, 0, 0)
                    }
                };
            
            // Quick snapshot of blockchain data with ONE read lock - no nested locks!
            info!("[DASHBOARD] 🔄 Collecting node state...");
            let (
                memory_blocks,
                pending_tx,
                seen_tx,
                chain_tip,
                is_mining,
                current_difficulty,
                hashrate,
                blocks_mined_count,
                uptime_secs,
                miner_address,
            ) = {
                let state = node.clone();

                info!("[DASHBOARD] 🔒 Acquiring blockchain lock...");
                let bc_lock_start = std::time::Instant::now();
                let chain_tip = {
                    let bc = state.bc.lock().unwrap();
                    info!("[DASHBOARD] ✅ Blockchain lock acquired (took {:?})", bc_lock_start.elapsed());
                    bc.chain_tip
                        .as_ref()
                        .map(|h| hex::encode(h))
                        .unwrap_or_else(|| "none".to_string())
                };
                info!("[DASHBOARD] ✅ Blockchain lock released (held {:?})", bc_lock_start.elapsed());

                info!("[DASHBOARD] 🔒 Acquiring chain_state lock...");
                let chain_lock_start = std::time::Instant::now();
                let memory_count = {
                    let chain = chain_state.lock().unwrap();
                    info!("[DASHBOARD] ✅ Chain_state lock acquired (took {:?})", chain_lock_start.elapsed());
                    chain.blockchain.len()
                };
                info!("[DASHBOARD] ✅ Chain_state lock released (held {:?})", chain_lock_start.elapsed());

                info!("[DASHBOARD] 🔒 Acquiring mempool lock...");
                let mempool_lock_start = std::time::Instant::now();
                let (pending_count, seen_count) = {
                    let mempool = state.mempool.lock().unwrap();
                    info!("[DASHBOARD] ✅ Mempool lock acquired (took {:?})", mempool_lock_start.elapsed());
                    (mempool.pending.len(), mempool.seen_tx.len())
                };
                info!("[DASHBOARD] ✅ Mempool lock released (held {:?})", mempool_lock_start.elapsed());

                info!("[DASHBOARD] 🔒 Acquiring mining locks...");
                let mining_start = std::time::Instant::now();
                let diff = *state.mining.current_difficulty.lock().unwrap();
                let hash = *state.mining.current_hashrate.lock().unwrap();
                let blocks_mined = state
                    .mining
                    .blocks_mined
                    .load(std::sync::atomic::Ordering::Relaxed);
                info!("[DASHBOARD] ✅ Mining state collected (took {:?})", mining_start.elapsed());

                info!("[DASHBOARD] 🔒 Acquiring miner_address lock...");
                let wallet_start = std::time::Instant::now();
                let wallet_addr = node_meta.miner_address.lock().unwrap().clone();
                info!("[DASHBOARD] ✅ Miner_address acquired (took {:?})", wallet_start.elapsed());

                (
                    memory_count,
                    pending_count,
                    seen_count,
                    chain_tip,
                    state.mining.active.load(std::sync::atomic::Ordering::Relaxed),
                    diff,
                    hash,
                    blocks_mined,
                    node_meta.node_start_time.elapsed().as_secs(),
                    wallet_addr,
                )
            };
            info!("[DASHBOARD] ✅ All state collected");
            
            // Get wallet balance OUTSIDE the lock (DB operation)
            info!("[DASHBOARD] 💰 Fetching wallet balance from DB...");
            let balance_start = std::time::Instant::now();
            let wallet_balance = {
                node
                    .bc
                    .lock()
                    .unwrap()
                    .get_address_balance_from_db(&miner_address)
                    .unwrap_or(U256::zero())
            };
            info!("[DASHBOARD] ✅ Wallet balance fetched (took {:?})", balance_start.elapsed());

            let connected_peers = peer_heights.len();
            let block_height = my_height;

            info!("[DASHBOARD] 📈 Fetching validation statistics...");
            // Get validation statistics (lock-free)
            let validation_stats = Astram_core::security::VALIDATION_STATS.get_stats();
            let total_failures: u64 = validation_stats.iter().map(|(_, count)| count).sum();
            info!("[DASHBOARD] ✅ Validation stats collected");

            info!("[DASHBOARD] 📦 Building response JSON...");
            let response = serde_json::json!({
                "node": {
                    "version": "0.1.0",
                    "uptime_seconds": uptime_secs,
                },
                "blockchain": {
                    "height": block_height,
                    "memory_blocks": memory_blocks,
                    "chain_tip": chain_tip,
                    "my_height": my_height,
                    "difficulty": current_difficulty,
                },
                "mempool": {
                    "pending_transactions": pending_tx,
                    "seen_transactions": seen_tx,
                    "max_size": crate::MAX_MEMPOOL_SIZE,
                    "max_bytes": crate::MAX_MEMPOOL_BYTES,
                },
                "network": {
                    "connected_peers": connected_peers,
                    "peer_heights": peer_heights,
                    "subnet_diversity": {
                        "unique_24_subnets": subnet_24_count,
                        "unique_16_subnets": subnet_16_count,
                    }
                },
                "mining": {
                    "active": is_mining,
                    "hashrate": hashrate,
                    "difficulty": current_difficulty,
                    "blocks_mined": blocks_mined_count,
                },
                "wallet": {
                    "address": miner_address,
                    "balance": format!("0x{:x}", wallet_balance),
                },
                "security": {
                    "validation_failures_total": total_failures,
                    "validation_failures": validation_stats.into_iter()
                        .filter(|(_, count)| *count > 0)
                        .collect::<Vec<_>>(),
                },
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            info!("[DASHBOARD] ✅ /status request COMPLETED in {:?}", request_start.elapsed());
            Ok::<_, warp::Rejection>(warp::reply::json(&response))
        });

    // GET /blockchain - Default endpoint (use memory for now)
    let get_chain = warp::path("blockchain")
        .and(warp::get())
        .and(chain_filter.clone())
        .and_then(|chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>| async move {
            let chain = chain_state.lock().unwrap();
            let bincode_bytes = bincode::encode_to_vec(&chain.blockchain, *BINCODE_CONFIG).unwrap();
            let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blockchain": encoded
            })))
        });

    // -------------------------------
    // POST /tx  (client -> node)
    // -------------------------------
    let post_tx = warp::path("tx")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(node_filter.clone())
        .and(p2p_filter.clone())
        .and_then(|body: bytes::Bytes, node: NodeHandle, p2p: std::sync::Arc<PeerManager>| async move {
            let tx: Transaction;

            match bincode::decode_from_slice::<Transaction, _>(&body, *BINCODE_CONFIG) {
                Ok((decoded, _)) => {
                    log::info!("Received Transaction {}", decoded.txid);
                    tx = decoded;
                }
                Err(e) => {
                    log::warn!("Invalid tx bincode: {}", e);
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": "invalid bincode"
                        })),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            }

            let state = node.clone();

            // Signature check
            match tx.verify_signatures() {
                Ok(true) => {
                    log::info!("TX {} signature OK", tx.txid);
                    
                    // Security: Validate fee before accepting to mempool
                    // Calculate input/output sums to verify fee
                    let mut input_sum = U256::zero();
                    let mut output_sum = U256::zero();
                    
                    // Get UTXOs from blockchain to calculate input sum
                    {
                        let bc = state.bc.lock().unwrap();
                        for inp in &tx.inputs {
                            let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                            if let Ok(Some(blob)) = bc.db.get(ukey.as_bytes()) {
                                if let Ok((utxo, _)) =
                                    bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG)
                                {
                                    input_sum = input_sum + utxo.amount();
                                }
                            }
                        }
                    }
                    
                    for out in &tx.outputs {
                        output_sum = output_sum + out.amount();
                    }
                    
                    let fee = if input_sum >= output_sum {
                        input_sum - output_sum
                    } else {
                        U256::zero()
                    };
                    
                    // Check minimum fee
                    let tx_blob = bincode::encode_to_vec(&tx, *BINCODE_CONFIG).unwrap();
                    let min_fee = Astram_core::config::calculate_min_fee(tx_blob.len());
                    
                    if fee < min_fee {
                        log::warn!("TX {} fee too low: got {}, need {}", tx.txid, fee, min_fee);
                        return Ok::<_, warp::Rejection>(with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("fee too low: got {} ram, need {} ram", fee, min_fee)
                            })),
                            StatusCode::BAD_REQUEST,
                        ));
                    }

                    let mut mempool = state.mempool.lock().unwrap();

                    // Duplicate protection
                    if mempool.seen_tx.contains_key(&tx.txid) {
                        log::info!("Duplicate TX {}", tx.txid);
                        return Ok::<_, warp::Rejection>(with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "duplicate"
                            })),
                            StatusCode::OK,
                        ));
                    }

                    // Security: Check for double-spending in mempool
                    // Collect all UTXOs used by this transaction
                    let mut tx_utxos = std::collections::HashSet::new();
                    for inp in &tx.inputs {
                        tx_utxos.insert(format!("{}:{}", inp.txid, inp.vout));
                    }

                    // Check if any pending transaction uses the same UTXOs
                    for pending_tx in &mempool.pending {
                        for pending_inp in &pending_tx.inputs {
                            let pending_utxo = format!("{}:{}", pending_inp.txid, pending_inp.vout);
                            if tx_utxos.contains(&pending_utxo) {
                                log::warn!(
                                    "Double-spend attempt: TX {} tries to use UTXO {} already used by pending TX {}",
                                    tx.txid, pending_utxo, pending_tx.txid
                                );
                                return Ok::<_, warp::Rejection>(with_status(
                                    warp::reply::json(&serde_json::json!({
                                        "status": "error",
                                        "message": format!("Double-spend: UTXO {} already used in mempool", pending_utxo)
                                    })),
                                    StatusCode::BAD_REQUEST,
                                ));
                            }
                        }
                    }

                    let now = chrono::Utc::now().timestamp();
                    mempool.seen_tx.insert(tx.txid.clone(), now);
                    mempool.pending.push(tx.clone());

                    // ---- broadcast to peers (async) ----
                    let p2p_clone = p2p.clone();
                    let tx_clone = tx.clone();

                    tokio::spawn(async move {
                        p2p_clone.broadcast_tx(&tx_clone).await;
                    });
                }
                _ => {
                    log::warn!("TX {} signature invalid", tx.txid);
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": "invalid signature"
                        })),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            }

            Ok::<_, warp::Rejection>(with_status(
                warp::reply::json(&serde_json::json!({
                    "status": "ok",
                    "message": "tx queued"
                })),
                StatusCode::OK,
            ))
        });

    // -------------------------------
    // POST /tx/relay  (node -> node)
    // -------------------------------
    let relay_tx = warp::path!("tx" / "relay")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(node_filter.clone())
        .and_then(|body: bytes::Bytes, node: NodeHandle| async move {
            let (tx, _) = match bincode::decode_from_slice::<Transaction, _>(&body, *BINCODE_CONFIG)
            {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("relay invalid bincode: {}", e);
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({"status":"error"})),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            };

            let state = node.clone();

            // Verify signature + fee
            if !tx.verify_signatures().unwrap_or(false) {
                log::warn!("relay invalid signature");
                return Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({"status":"invalid_signature"})),
                    StatusCode::OK,
                ));
            }
            
            // Security: Validate fee for relayed transactions
            let mut input_sum = U256::zero();
            let mut output_sum = U256::zero();
            
            {
                let bc = state.bc.lock().unwrap();
                for inp in &tx.inputs {
                    let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                    if let Ok(Some(blob)) = bc.db.get(ukey.as_bytes()) {
                        if let Ok((utxo, _)) =
                            bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG)
                        {
                            input_sum = input_sum + utxo.amount();
                        }
                    }
                }
            }
            
            for out in &tx.outputs {
                output_sum = output_sum + out.amount();
            }
            
            let fee = if input_sum >= output_sum { input_sum - output_sum } else { U256::zero() };
            let tx_blob = bincode::encode_to_vec(&tx, *BINCODE_CONFIG).unwrap();
            let min_fee = Astram_core::config::calculate_min_fee(tx_blob.len());
            
            if fee >= min_fee {
                let mut mempool = state.mempool.lock().unwrap();

                // Duplicate check
                if mempool.seen_tx.contains_key(&tx.txid) {
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({"status":"duplicate"})),
                        StatusCode::OK,
                    ));
                }

                // Record seen tx
                let now = chrono::Utc::now().timestamp();
                mempool.seen_tx.insert(tx.txid.clone(), now);

                log::info!("relay accepted tx {} (fee: {} >= {})", tx.txid, fee, min_fee);
                mempool.pending.push(tx);
            } else {
                log::warn!("relay rejected tx {}: fee too low ({} < {})", tx.txid, fee, min_fee);
            }

            Ok::<_, warp::Rejection>(with_status(
                warp::reply::json(&serde_json::json!({"status":"ok"})),
                StatusCode::OK,
            ))
        });

    // -------------------------------
    // GET /mempool - Pending transactions + fee summary
    // -------------------------------
    let get_mempool = warp::path("mempool")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.clone();
            let txs = {
                let mempool = state.mempool.lock().unwrap();
                mempool.pending.clone()
            };

            let mut input_keys = std::collections::HashSet::new();
            for tx in &txs {
                for inp in &tx.inputs {
                    input_keys.insert(format!("u:{}:{}", inp.txid, inp.vout));
                }
            }

            let utxo_amounts = {
                let bc = state.bc.lock().unwrap();
                let mut map = std::collections::HashMap::new();
                for ukey in input_keys {
                    if let Ok(Some(blob)) = bc.db.get(ukey.as_bytes()) {
                        if let Ok((utxo, _)) =
                            bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG)
                        {
                            map.insert(ukey, utxo.amount());
                        }
                    }
                }
                map
            };

            let mut total_fees = U256::zero();
            for tx in &txs {
                let mut input_sum = U256::zero();
                let mut output_sum = U256::zero();

                for inp in &tx.inputs {
                    let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                    if let Some(amount) = utxo_amounts.get(&ukey) {
                        input_sum = input_sum + *amount;
                    }
                }

                for out in &tx.outputs {
                    output_sum = output_sum + out.amount();
                }

                if input_sum >= output_sum {
                    total_fees = total_fees + (input_sum - output_sum);
                }
            }

            let bincode_bytes = bincode::encode_to_vec(&txs, *BINCODE_CONFIG).unwrap();
            let encoded = general_purpose::STANDARD.encode(&bincode_bytes);

            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "count": txs.len(),
                "transactions_b64": encoded,
                "total_fees": format!("0x{:x}", total_fees)
            })))
        });

    // -------------------------------
    // POST /mining/submit - Submit a mined block
    // -------------------------------
    #[derive(Deserialize)]
    struct SubmitBlockRequest {
        block_b64: String,
    }

    let submit_block = warp::path!("mining" / "submit")
        .and(warp::post())
        .and(warp::body::json())
        .and(node_filter.clone())
        .and(p2p_filter.clone())
        .and(chain_filter.clone())
        .and_then(|req: SubmitBlockRequest, node: NodeHandle, p2p: std::sync::Arc<PeerManager>, chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>| async move {
            let bytes = match general_purpose::STANDARD.decode(req.block_b64.as_bytes()) {
                Ok(b) => b,
                Err(e) => {
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": format!("invalid base64: {}", e)
                        })),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            };

            let (block, _) = match bincode::decode_from_slice::<Block, _>(&bytes, *BINCODE_CONFIG)
            {
                Ok(v) => v,
                Err(e) => {
                    return Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": format!("invalid block bincode: {}", e)
                        })),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            };

            let state = node.clone();
            match state.bc.lock().unwrap().validate_and_insert_block(&block) {
                Ok(_) => {
                    {
                        let mut chain = chain_state.lock().unwrap();
                        chain.blockchain.push(block.clone());
                        chain.enforce_memory_limit();
                    }
                    p2p.set_my_height(block.header.index + 1);

                    let now = chrono::Utc::now().timestamp();
                    {
                        let mut chain = chain_state.lock().unwrap();
                        chain.recently_mined_blocks.insert(block.hash.clone(), now);
                        chain
                            .recently_mined_blocks
                            .retain(|_, &mut timestamp| now - timestamp < 300);
                    }

                    let block_to_broadcast = block.clone();
                    tokio::spawn(async move {
                        p2p.broadcast_block(&block_to_broadcast).await;
                    });

                    Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "ok",
                            "hash": block.hash,
                            "height": block.header.index
                        })),
                        StatusCode::OK,
                    ))
                }
                Err(e) => Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({
                        "status": "error",
                        "message": format!("block rejected: {}", e)
                    })),
                    StatusCode::BAD_REQUEST,
                )),
            }
        });

    // GET /status
    let status = warp::path("status")
        .and(warp::get())
        .and(node_filter.clone())
        .and(chain_filter.clone())
        .and_then(|node: NodeHandle, chain_state: std::sync::Arc<std::sync::Mutex<ChainState>>| async move {
            let chain = chain_state.lock().unwrap();
            let height = chain
                .blockchain
                .last()
                .map(|b| b.header.index as usize)
                .unwrap_or(0);
            let pending = node.mempool.lock().unwrap().pending.len();
            let s = serde_json::json!({
                "height": height,
                "pending": pending
            });
            Ok::<_, warp::Rejection>(warp::reply::json(&s))
        });

    // GET /address/{address}/balance
    let get_balance = warp::path!("address" / String / "balance")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|address: String, node: NodeHandle| async move {
            match node.bc.lock().unwrap().get_address_balance_from_db(&address) {
                Ok(bal) => {
                    log::info!("[INFO] Balance lookup success: {} -> {}", address, bal);
                    Ok::<_, warp::Rejection>(warp::reply::json(
                        &serde_json::json!({"address": address, "balance": bal}),
                    ))
                }
                Err(e) => {
                    log::warn!("[WARN] Balance lookup failed for {}: {:?}", address, e);
                    Ok::<_, warp::Rejection>(warp::reply::json(
                        &serde_json::json!({"address": address, "balance": 0}),
                    ))
                }
            }
        });

    let get_utxos = warp::path!("address" / String / "utxos")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|address: String, node: NodeHandle| async move {
            match node.bc.lock().unwrap().get_utxos(&address) {
                Ok(list) => Ok::<_, warp::Rejection>(warp::reply::json(&list)),
                Err(e) => {
                    log::warn!("UTXO lookup failed {}: {:?}", address, e);
                    Ok::<_, warp::Rejection>(warp::reply::json(&Vec::<Utxo>::new()))
                }
            }
        });

    // GET /address/{address}/info - Address statistics from DB
    let get_address_info = warp::path!("address" / String / "info")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|address: String, node: NodeHandle| async move {
            // Normalize address to lowercase for consistent lookup
            let address = address.to_lowercase();

            let bc = node.bc.lock().unwrap();
            let balance = bc
                .get_address_balance_from_db(&address)
                .unwrap_or(U256::zero());
            let received = bc
                .get_address_received_from_db(&address)
                .unwrap_or(U256::zero());
            let sent = bc
                .get_address_sent_from_db(&address)
                .unwrap_or(U256::zero());
            let tx_count = bc
                .get_address_transaction_count_from_db(&address)
                .unwrap_or(0);

            log::info!(
                "Address info for {}: balance={}, received={}, sent={}, tx_count={}",
                address,
                balance,
                received,
                sent,
                tx_count
            );

            // Convert U256 to hex strings for JSON (to avoid precision loss in JavaScript)
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "address": address,
                "balance": format!("0x{:x}", balance),
                "received": format!("0x{:x}", received),
                "sent": format!("0x{:x}", sent),
                "transaction_count": tx_count
            })))
        });

    // GET /tx/{txid}
    let get_tx = warp::path!("tx" / String)
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|txid: String, node: NodeHandle| async move {
            match node.bc.lock().unwrap().get_transaction(&txid) {
                Ok(Some((tx, height))) => {
                    let bincode_bytes = bincode::encode_to_vec(&tx, *BINCODE_CONFIG).unwrap();
                    let encoded = general_purpose::STANDARD.encode(&bincode_bytes);

                    Ok::<_, warp::Rejection>(with_status(
                        warp::reply::json(&serde_json::json!({
                            "txid": txid,
                            "block_height": height,
                            "transaction": encoded,
                            "encoding": "bincode+base64"
                        })),
                        StatusCode::OK,
                    ))
                }

                Ok(None) => Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({
                        "error": "tx not found"
                    })),
                    StatusCode::NOT_FOUND,
                )),

                Err(e) => Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({
                        "error": format!("db error: {}", e)
                    })),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )),
            }
        });



    // -------------------------------
    // GET / - Dashboard HTML
    let dashboard = warp::path::end()
        .and(warp::get())
        .map(|| {
            warp::reply::html(include_str!("../../web/dashboard.html"))
        });

    // -------------------------------
    // combine routes
    // NOTE: Order matters! More specific routes must come before general ones
    let routes = dashboard
        .or(get_chain_db)          // /blockchain/db - specific
        .or(get_chain_memory)      // /blockchain/memory - specific
        .or(get_chain_range)       // /blockchain/range - specific
        .or(get_chain)             // /blockchain - general (must be last)
        .or(get_counts)
        .or(get_status)
        .or(debug_counts)
        .or(health_check)
        .or(post_tx)
        .or(relay_tx)
        .or(get_mempool)
        .or(submit_block)
        .or(status)
        .or(get_balance)
        .or(get_address_info)
        .or(get_utxos)
        .or(get_tx)
        .with(warp::log("Astram::http"))
        .boxed();

    println!("HTTP server running at http://{}", bind_addr);
    warp::serve(routes).run(bind_addr).await;
}

