use crate::NodeHandle;
use base64::{Engine as _, engine::general_purpose};
use netcoin_core::transaction::{BINCODE_CONFIG, Transaction};
use netcoin_core::utxo::Utxo;
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

    // GET /counts - lightweight counts for blocks and transactions (DB)
    let get_counts = warp::path("counts")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            let blocks = state.bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);
            let transactions = state.bc.count_transactions().unwrap_or(0);
            let volume = state.bc.calculate_total_volume().unwrap_or(0);
            log::info!(
                "📈 Counts endpoint - blocks: {}, transactions: {}, volume: {}",
                blocks,
                transactions,
                volume
            );
            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "blocks": blocks,
                "transactions": transactions,
                "total_volume": volume
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
            let mut tx: Transaction;

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

            // 검증
            if tx.verify_signatures().unwrap_or(false) {
                log::info!("relay accepted tx {}", tx.txid);
                state.pending.push(tx);
            } else {
                log::warn!("relay invalid signature");
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
            match state.bc.get_balance(&address) {
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
            let state = node.lock().unwrap();

            let balance = state.bc.get_address_balance_from_db(&address).unwrap_or(0);
            let received = state.bc.get_address_received_from_db(&address).unwrap_or(0);
            let sent = state.bc.get_address_sent_from_db(&address).unwrap_or(0);
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

            Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                "address": address,
                "balance": balance,
                "received": received,
                "sent": sent,
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
    // combine routes
    // combine routes
    let routes = get_chain
        .or(get_chain_memory)
        .or(get_chain_db)
        .or(get_counts)
        .or(get_status)
        .or(debug_counts)
        .or(post_tx)
        .or(relay_tx)
        .or(status)
        .or(get_balance)
        .or(get_address_info)
        .or(get_utxos)
        .or(get_tx)
        .with(warp::log("netcoin::http"))
        .boxed();

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    let addr = ([127, 0, 0, 1], 8333);
    warp::serve(routes).run(addr).await;
}
