use crate::storage::block::Transaction;
use warp::Filter;

pub async fn start_rpc() {
    let tx_send = warp::path("tx")
        .and(warp::path("send"))
        .and(warp::post())
        .and(warp::body::json())
        .map(|tx: Transaction| {
            println!("Received transaction: {:?}", tx);
            warp::reply::json(&serde_json::json!({"status": "ok"}))
        });

    warp::serve(tx_send).run(([127, 0, 0, 1], 3030)).await;
}
