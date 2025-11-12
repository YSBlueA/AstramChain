use crate::NodeHandle;
use base64::{Engine as _, engine::general_purpose};
use bincode::config::standard;
use netcoin_core::transaction::Transaction;
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
            let bincode_bytes = bincode::encode_to_vec(&state.blockchain, standard()).unwrap();
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
            let tx: Transaction = match bincode::decode_from_slice(&body, standard()) {
                Ok((tx, _)) => tx,
                Err(_) => return Ok::<_, warp::Rejection>(
                    with_status(
                        warp::reply::json(&serde_json::json!({"status":"error","message":"invalid bincode"})),
                        StatusCode::BAD_REQUEST
                    )
                ),
            };

            {
                let mut state = node.lock().unwrap();
                if !tx.inputs.is_empty() {
                    match tx.verify_signatures() {
                        Ok(true) => state.pending.push(tx),
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

    let routes = get_chain
        .or(post_tx)
        .or(status)
        .or(get_balance)
        .with(warp::log("netcoin::http"))
        .boxed();

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    let addr = ([127, 0, 0, 1], 8333);
    warp::serve(routes).run(addr).await;
}
