pub mod eth_rpc;

pub use eth_rpc::run_eth_rpc_server;

use crate::NodeHandle;
use base64::{Engine as _, engine::general_purpose};
use netcoin_core::transaction::{BINCODE_CONFIG, Transaction};
use netcoin_core::utxo::Utxo;
use primitive_types::U256;
use warp::Filter;
use warp::{http::StatusCode, reply::with_status}; // bincode v2
/// run_server expects NodeHandle (Arc<Mutex<NodeState>>)
pub async fn run_server(node: NodeHandle) {
    let node_filter = {
        let node = node.clone();
        warp::any().map(move || node.clone())
    };

    // -------------------------------
    // GET /blockchain/memory - In-memory blockchain state
    let get_chain_memory = warp::path!("blockchain" / "memory")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            let bincode_bytes = bincode::encode_to_vec(&state.blockchain, *BINCODE_CONFIG).unwrap();
            let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
            log::info!("✅ Returning {} blocks from memory", state.blockchain.len());
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blockchain": encoded,
                "count": state.blockchain.len(),
                "source": "memory"
            })))
        });

    // GET /blockchain/db - Blocks from database
    let get_chain_db = warp::path!("blockchain" / "db")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            match state.bc.get_all_blocks() {
                Ok(all_blocks) => {
                    let bincode_bytes =
                        bincode::encode_to_vec(&all_blocks, *BINCODE_CONFIG).unwrap();
                    let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
                    log::info!("✅ Returning {} blocks from DB", all_blocks.len());
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "blockchain": encoded,
                        "count": all_blocks.len(),
                        "source": "database"
                    })))
                }
                Err(e) => {
                    log::error!("❌ Failed to fetch blocks from DB: {}", e);
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "error": format!("Failed to fetch blockchain from DB: {}", e),
                        "count": 0,
                        "source": "database"
                    })))
                }
            }
        });

    // GET /debug/block-counts - Simple debug endpoint
    let debug_counts = warp::path!("debug" / "block-counts")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            let memory_count = state.blockchain.len();
            let db_count = state.bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);

            log::info!(
                "📊 Block counts - Memory: {}, DB: {}",
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
            let state = node.lock().unwrap();
            let height = if let Some(tip_hash) = &state.bc.chain_tip {
                if let Ok(Some(header)) = state.bc.load_header(tip_hash) {
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
            let state = node.lock().unwrap();
            let blocks = state.bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);
            let transactions = state.bc.count_transactions().unwrap_or(0);
            let volume = state.bc.calculate_total_volume().unwrap_or(U256::zero());
            log::info!(
                "📈 Counts endpoint - blocks: {}, transactions: {}, volume: {}",
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
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();

            // Get blockchain info
            let block_height = state.bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);
            let memory_blocks = state.blockchain.len();
            let pending_tx = state.pending.len();
            let seen_tx = state.seen_tx.len();

            // Get P2P network info
            let peer_heights = state.p2p.get_peer_heights();
            let connected_peers = peer_heights.len();
            let my_height = state.p2p.get_my_height();

            // Get chain tip hash
            let chain_tip = state
                .bc
                .chain_tip
                .as_ref()
                .map(|hash| hex::encode(hash))
                .unwrap_or_else(|| "none".to_string());

            log::info!(
                "🔍 Status requested - Height: {}, Peers: {}, Pending TX: {}",
                block_height,
                connected_peers,
                pending_tx
            );

            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "node": {
                    "version": "0.1.0",
                    "uptime_seconds": 0, // TODO: Add start time tracking
                },
                "blockchain": {
                    "height": block_height,
                    "memory_blocks": memory_blocks,
                    "chain_tip": chain_tip,
                    "my_height": my_height,
                },
                "mempool": {
                    "pending_transactions": pending_tx,
                    "seen_transactions": seen_tx,
                },
                "network": {
                    "connected_peers": connected_peers,
                    "peer_heights": peer_heights,
                },
                "timestamp": chrono::Utc::now().to_rfc3339(),
            })))
        });

    // GET /blockchain - Default endpoint (use memory for now)
    let get_chain = warp::path("blockchain")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            let bincode_bytes = bincode::encode_to_vec(&state.blockchain, *BINCODE_CONFIG).unwrap();
            let encoded = general_purpose::STANDARD.encode(&bincode_bytes);
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blockchain": encoded
            })))
        });

    // -------------------------------
    // POST /tx  (client → node)
    // -------------------------------
    let post_tx = warp::path("tx")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(node_filter.clone())
        .and_then(|body: bytes::Bytes, node: NodeHandle| async move {
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

            // lock
            let mut state = node.lock().unwrap();

            // 중복 방지
            if state.seen_tx.contains(&tx.txid) {
                log::info!("Duplicate TX {}", tx.txid);
                return Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({
                        "status": "duplicate"
                    })),
                    StatusCode::OK,
                ));
            }

            // signature 검증
            match tx.verify_signatures() {
                Ok(true) => {
                    log::info!("TX {} signature OK", tx.txid);
                    
                    // 🔒 Security: Validate fee before accepting to mempool
                    // Calculate input/output sums to verify fee
                    let mut input_sum = U256::zero();
                    let mut output_sum = U256::zero();
                    
                    // Get UTXOs from blockchain to calculate input sum
                    for inp in &tx.inputs {
                        let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                        if let Ok(Some(blob)) = state.bc.db.get(ukey.as_bytes()) {
                            if let Ok((utxo, _)) = bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG) {
                                input_sum = input_sum + utxo.amount();
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
                    let min_fee = netcoin_core::config::calculate_min_fee(tx_blob.len());
                    
                    if fee < min_fee {
                        log::warn!("TX {} fee too low: got {}, need {}", tx.txid, fee, min_fee);
                        return Ok::<_, warp::Rejection>(with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("fee too low: got {} natoshi, need {} natoshi", fee, min_fee)
                            })),
                            StatusCode::BAD_REQUEST,
                        ));
                    }

                    state.seen_tx.insert(tx.txid.clone());
                    state.pending.push(tx.clone());

                    // ---- broadcast to peers (async) ----
                    let p2p_clone = state.p2p.clone();
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
    // POST /tx/relay  (node → node)
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

            let mut state = node.lock().unwrap();

            // 중복 체크
            if state.seen_tx.contains(&tx.txid) {
                return Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({"status":"duplicate"})),
                    StatusCode::OK,
                ));
            }

            // seen 기록
            state.seen_tx.insert(tx.txid.clone());

            // 검증: signature + fee
            if !tx.verify_signatures().unwrap_or(false) {
                log::warn!("relay invalid signature");
                return Ok::<_, warp::Rejection>(with_status(
                    warp::reply::json(&serde_json::json!({"status":"invalid_signature"})),
                    StatusCode::OK,
                ));
            }
            
            // 🔒 Security: Validate fee for relayed transactions
            let mut input_sum = U256::zero();
            let mut output_sum = U256::zero();
            
            for inp in &tx.inputs {
                let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                if let Ok(Some(blob)) = state.bc.db.get(ukey.as_bytes()) {
                    if let Ok((utxo, _)) = bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG) {
                        input_sum = input_sum + utxo.amount();
                    }
                }
            }
            
            for out in &tx.outputs {
                output_sum = output_sum + out.amount();
            }
            
            let fee = if input_sum >= output_sum { input_sum - output_sum } else { U256::zero() };
            let tx_blob = bincode::encode_to_vec(&tx, *BINCODE_CONFIG).unwrap();
            let min_fee = netcoin_core::config::calculate_min_fee(tx_blob.len());
            
            if fee >= min_fee {
                log::info!("relay accepted tx {} (fee: {} >= {})", tx.txid, fee, min_fee);
                state.pending.push(tx);
            } else {
                log::warn!("relay rejected tx {}: fee too low ({} < {})", tx.txid, fee, min_fee);
            }

            Ok::<_, warp::Rejection>(with_status(
                warp::reply::json(&serde_json::json!({"status":"ok"})),
                StatusCode::OK,
            ))
        });

    // GET /status
    let status = warp::path("status")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            let height = state
                .blockchain
                .last()
                .map(|b| b.header.index as usize)
                .unwrap_or(0);
            let s = serde_json::json!({
                "height": height,
                "pending": state.pending.len()
            });
            Ok::<_, warp::Rejection>(warp::reply::json(&s))
        });

    // GET /address/{address}/balance
    let get_balance = warp::path!("address" / String / "balance")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|address: String, node: NodeHandle| async move {
            let state = node.lock().unwrap();
            match state.bc.get_address_balance_from_db(&address) {
                Ok(bal) => {
                    log::info!("✅ balance lookup success: {} -> {}", address, bal);
                    Ok::<_, warp::Rejection>(warp::reply::json(
                        &serde_json::json!({"address": address, "balance": bal}),
                    ))
                }
                Err(e) => {
                    log::warn!("⚠️ balance lookup failed for {}: {:?}", address, e);
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
            let state = node.lock().unwrap();
            match state.bc.get_utxos(&address) {
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
            let state = node.lock().unwrap();

            let balance = state
                .bc
                .get_address_balance_from_db(&address)
                .unwrap_or(U256::zero());
            let received = state
                .bc
                .get_address_received_from_db(&address)
                .unwrap_or(U256::zero());
            let sent = state
                .bc
                .get_address_sent_from_db(&address)
                .unwrap_or(U256::zero());
            let tx_count = state
                .bc
                .get_address_transaction_count_from_db(&address)
                .unwrap_or(0);

            log::info!(
                "📍 Address info for {}: balance={}, received={}, sent={}, tx_count={}",
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
            let state = node.lock().unwrap();

            match state.bc.get_transaction(&txid) {
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
    // GET /eth_mapping/:eth_hash - Resolve Ethereum tx hash to NetCoin txid
    let get_eth_mapping = warp::path!("eth_mapping" / String)
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|eth_hash: String, node: NodeHandle| async move {
            let state = node.lock().unwrap();
            // Strip 0x prefix if present
            let eth_hash = eth_hash.strip_prefix("0x").unwrap_or(&eth_hash);
            
            match state.eth_to_netcoin_tx.get(eth_hash) {
                Some(netcoin_txid) => {
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "eth_hash": format!("0x{}", eth_hash),
                        "netcoin_txid": netcoin_txid,
                        "found": true
                    })))
                }
                None => {
                    Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                        "eth_hash": format!("0x{}", eth_hash),
                        "netcoin_txid": null,
                        "found": false
                    })))
                }
            }
        });

    // -------------------------------
    // combine routes
    // combine routes
    let routes = get_chain
        .or(get_chain_memory)
        .or(get_chain_db)
        .or(get_counts)
        .or(get_status)
        .or(debug_counts)
        .or(health_check)
        .or(post_tx)
        .or(relay_tx)
        .or(status)
        .or(get_balance)
        .or(get_address_info)
        .or(get_utxos)
        .or(get_tx)
        .or(get_eth_mapping)
        .with(warp::log("netcoin::http"))
        .boxed();

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    let addr = ([127, 0, 0, 1], 8333);
    warp::serve(routes).run(addr).await;
}
