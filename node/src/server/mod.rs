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

    // GET /blockchain
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

    // POST /tx
    let post_tx = warp::path("tx")
        .and(warp::post())
        .and(warp::body::bytes()) // raw body
        .and(node_filter.clone())
        .and_then(|body: bytes::Bytes, node: NodeHandle| async move {
            // try decode as SignedTx first, fall back to Transaction
            let mut create_tx: Transaction;
            // fallback: try decoding raw Transaction (wallet-cli may send this)
            match bincode::decode_from_slice::<Transaction, _>(&body, *BINCODE_CONFIG) {
                Ok((tx, _)) => {
                    log::info!("✅ Received raw Transaction");
                    create_tx = tx;
                }
                Err(e) => {
                    log::debug!("❌ error decoding bincode: {}", e);
                    return Ok::<_, warp::Rejection>(
                        with_status(
                            warp::reply::json(&serde_json::json!({"status":"error","message":"invalid bincode"})),
                            StatusCode::BAD_REQUEST
                        )
                    );
                }
            }

            {
                let mut state = node.lock().unwrap();

                log::debug!("Created Transaction: {:?}", create_tx);
                log::info!("Queuing transaction with {} inputs and {} outputs", create_tx.inputs.len(), create_tx.outputs.len());
                log::info!("Transaction ID: {}", create_tx.txid);
                if !create_tx.inputs.is_empty() {
                    log::info!("From pubkey: {}", create_tx.inputs[0].pubkey);
                }
                if !create_tx.outputs.is_empty() {
                    log::info!("To address: {}", create_tx.outputs[0].to);
                    log::info!("Amount: {}", create_tx.outputs[0].amount);
                }
                log::info!("Timestamp: {}", create_tx.timestamp);
                log::info!("Signature: {}", create_tx.inputs.get(0).and_then(|i| i.signature.as_deref()).unwrap_or("no signature"));
                log::info!("Verifying signature...");

                if !create_tx.inputs.is_empty() {
                    match create_tx.verify_signatures() {
                        Ok(true) => state.pending.push(create_tx),
                        _ => return Ok::<_, warp::Rejection>(
                            with_status(
                                warp::reply::json(&serde_json::json!({"status":"error","message":"invalid signature"})),
                                StatusCode::BAD_REQUEST
                            )
                        ),
                    }
                } else {
                    return Ok::<_, warp::Rejection>(
                        with_status(
                            warp::reply::json(&serde_json::json!({"status":"error","message":"coinbase tx not allowed"})),
                            StatusCode::BAD_REQUEST
                        )
                    );
                }
            }

            Ok::<_, warp::Rejection>(
                with_status(
                    warp::reply::json(&serde_json::json!({"status": "ok", "message": "tx queued"})),
                    StatusCode::OK,
                )
            )
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

    let routes = get_chain
        .or(post_tx)
        .or(status)
        .or(get_balance)
        .or(get_utxos)
        .with(warp::log("netcoin::http"))
        .boxed();

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    let addr = ([127, 0, 0, 1], 8333);
    warp::serve(routes).run(addr).await;
}
