use warp::Filter;
use netcoin_core::transaction::Transaction;
use std::sync::{Arc, Mutex};
use serde_json::json;
use crate::NodeHandle;
use warp::{http::StatusCode, reply::with_status};

/// run_server expects NodeHandle (Arc<Mutex<NodeState>>)
pub async fn run_server(node: NodeHandle) {
    // node filter
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
            let json = serde_json::to_string_pretty(&state.blockchain)
                .unwrap_or_else(|_| "[]".to_string());
            Ok::<_, warp::Rejection>(
                warp::reply::json(
                    &serde_json::from_str::<serde_json::Value>(&json).unwrap()
                )
            )
        });

    // POST /tx
    let post_tx = warp::path("tx")
        .and(warp::post())
        .and(warp::body::json())
        .and(node_filter.clone())
        .and_then(|tx: Transaction, node: NodeHandle| async move {
            {
                let mut state = node.lock().unwrap();
                if !tx.inputs.is_empty() {
                    match tx.verify_signatures() {
                        Ok(true) => {
                            state.pending.push(tx);
                        }
                        _ => {
                            return Ok::<_, warp::Rejection>(
                                with_status(
                                    warp::reply::json(
                                        &json!({"status":"error","message":"invalid signature"})
                                    ),
                                    StatusCode::BAD_REQUEST
                                )
                            );
                        }
                    }
                } else {
                    return Ok::<_, warp::Rejection>(
                        with_status(
                            warp::reply::json(
                                &json!({"status":"error","message":"coinbase tx not allowed"})
                            ),
                            StatusCode::BAD_REQUEST
                        )
                    );
                }
            }
            Ok::<_, warp::Rejection>(
                with_status(
                    warp::reply::json(&json!({"status": "ok", "message": "tx queued"})),
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
            let height = state.blockchain.last()
                .map(|b| b.header.index as usize)
                .unwrap_or(0);
            let s = json!({
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
                Ok(bal) => Ok::<_, warp::Rejection>(
                    warp::reply::json(&json!({"address": address, "balance": bal}))
                ),
                Err(_) => Ok::<_, warp::Rejection>(
                    warp::reply::json(&json!({"address": address, "balance": 0}))
                ),
            }
        });

    // 모든 route를 Box::leak 으로 static lifetime 으로 변환
    let routes = get_chain
        .or(post_tx)
        .or(status)
        .or(get_balance)
        .with(warp::log("netcoin::http"))
        .boxed(); // <- boxed() 추가

    println!("🌐 HTTP server running at http://127.0.0.1:8333");

    // 여기서 Arc::leak으로 lifetime 문제 해결
    let addr = ([127, 0, 0, 1], 8333);

    // run_server 내부에서 await로 실행
    warp::serve(routes) // routes.clone() 대신 boxed routes를 직접 사용
        .run(addr)
        .await;
}
