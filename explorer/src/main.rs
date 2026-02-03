mod api;
mod handlers;
mod rpc;
mod state;

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use log::{error, info};
use rpc::NodeRpcClient;
use state::AppState;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::time::{Duration, interval};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("🌐 NetCoin Explorer starting...");

    let app_state = Arc::new(Mutex::new(AppState::new()));
    let app_state_sync = app_state.clone();

    // 백그라운드에서 Node RPC와 동기화하는 태스크
    tokio::spawn(async move {
        let rpc_client = NodeRpcClient::new("http://127.0.0.1:8333");

        // 서버 시작 직후 즉시 첫 번째 동기화 수행
        let mut utxo_map = app_state_sync.lock().unwrap().utxo_map.clone();
        match rpc_client
            .fetch_blockchain_with_transactions(&mut utxo_map)
            .await
        {
            Ok((blocks, transactions)) => {
                let mut state = app_state_sync.lock().unwrap();
                state.cached_blocks = blocks.clone();
                state.cached_transactions = transactions;
                state.utxo_map = utxo_map;
                state.last_update = chrono::Utc::now();

                info!(
                    "✅ Initial blockchain data synced from Node: {} blocks, {} transactions",
                    blocks.len(),
                    state.cached_transactions.len()
                );
            }
            Err(e) => {
                error!("❌ Failed to sync blockchain on startup: {}", e);
            }
        }

        // 이후 10초마다 동기화
        let mut sync_interval = interval(Duration::from_secs(10));

        loop {
            sync_interval.tick().await;

            // Node에서 실제 블록체인 데이터 가져오기
            let mut utxo_map = app_state_sync.lock().unwrap().utxo_map.clone();
            match rpc_client
                .fetch_blockchain_with_transactions(&mut utxo_map)
                .await
            {
                Ok((blocks, transactions)) => {
                    let mut state = app_state_sync.lock().unwrap();
                    state.cached_blocks = blocks;
                    state.cached_transactions = transactions;
                    state.utxo_map = utxo_map;
                    state.last_update = chrono::Utc::now();

                    info!(
                        "✅ Blockchain data synced from Node: {} blocks",
                        state.cached_blocks.len()
                    );
                }
                Err(e) => {
                    error!("❌ Failed to sync blockchain: {}", e);
                }
            }
        }
    });

    let server_address = "127.0.0.1";
    let server_port = 8080;

    info!(
        "📡 Server listening on http://{}:{}",
        server_address, server_port
    );

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .app_data(web::Data::new(app_state.clone()))
            .wrap(middleware::Logger::default())
            .wrap(cors)
            .service(
                web::scope("/api")
                    .route("/health", web::get().to(handlers::health))
                    .route("/blocks", web::get().to(handlers::get_blocks))
                    .route(
                        "/blocks/{height}",
                        web::get().to(handlers::get_block_by_height),
                    )
                    .route(
                        "/blocks/hash/{hash}",
                        web::get().to(handlers::get_block_by_hash),
                    )
                    .route("/transactions", web::get().to(handlers::get_transactions))
                    .route(
                        "/transactions/{hash}",
                        web::get().to(handlers::get_transaction_by_hash),
                    )
                    .route("/stats", web::get().to(handlers::get_blockchain_stats))
                    .route(
                        "/address/{address}",
                        web::get().to(handlers::get_address_info),
                    ),
            )
    })
    .bind(format!("{}:{}", server_address, server_port))?
    .run()
    .await
}
