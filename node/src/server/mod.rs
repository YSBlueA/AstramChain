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
    // GET /blockchain
    // -------------------------------
    let get_chain = warp::path("blockchain")
        .and(warp::get())
        .and(node_filter.clone())
        .and_then(|node: NodeHandle| async move {
            let state = node.lock().unwrap();
            // use core's BINCODE_CONFIG so client/server config matches
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

    // -------------------------------
    // combine routes
    // -------------------------------
    let routes = get_chain
        .or(post_tx)
        .or(relay_tx)
        .or(status)
        .or(get_balance)
        .or(get_utxos)
        .with(warp::log("netcoin::http"))
        .boxed();

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    let addr = ([127, 0, 0, 1], 8333);
    warp::serve(routes).run(addr).await;
}
