mod api;
mod db;
mod handlers;
mod rpc;
mod state;

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use db::ExplorerDB;
use log::{error, info};
use rpc::NodeRpcClient;
use std::sync::Arc;
use tokio::time::{Duration, interval};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("Astram Explorer starting...");

    // Explorer database initialization
    let db_path = "explorer_data";
    let explorer_db = Arc::new(ExplorerDB::new(db_path).expect("Failed to open explorer database"));

    info!("Explorer database initialized at {}", db_path);

    // Background sync with the Node process
    let db_sync = explorer_db.clone();
    let rpc_client = Arc::new(NodeRpcClient::new("http://127.0.0.1:19533"));
    let rpc_for_sync = rpc_client.clone();
    tokio::spawn(async move {

        info!("Starting blockchain indexing...");

        // Initial sync
        match sync_blockchain(&db_sync, &rpc_for_sync).await {
            Ok(()) => {
                info!("Initial blockchain sync completed");
            }
            Err(e) => {
                error!("Failed to sync blockchain on startup: {}", e);
            }
        }

        // Sync every 10 seconds
        let mut sync_interval = interval(Duration::from_secs(10));

        loop {
            sync_interval.tick().await;

            match sync_blockchain(&db_sync, &rpc_for_sync).await {
                Ok(()) => {
                    // Success logging is handled in sync_blockchain
                }
                Err(e) => {
                    error!("Failed to sync blockchain: {}", e);
                }
            }
        }
    });

    let server_address = "0.0.0.0";
    let server_port = 8080;

    info!(
        "Server listening on http://{}:{}",
        server_address, server_port
    );

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .app_data(web::Data::new(explorer_db.clone()))
            .app_data(web::Data::new(rpc_client.clone()))
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
                    )
                    .route("/node/status", web::get().to(handlers::get_node_status)),
            )
    })
    .bind(format!("{}:{}", server_address, server_port))?
    .run()
    .await
}

/// Fetch blockchain data from the node and index into the database
async fn sync_blockchain(db: &ExplorerDB, rpc_client: &NodeRpcClient) -> anyhow::Result<()> {
    // Load last synced height
    let last_synced = db.get_last_synced_height()?;

    let mut utxo_map = std::collections::HashMap::new();
    let (blocks, transactions) = if last_synced == 0 {
        // Full sync: fetch entire blockchain
        log::info!("Initial sync: fetching entire blockchain from Node");
        rpc_client
            .fetch_blockchain_with_transactions(&mut utxo_map)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch blockchain: {}", e))?
    } else {
        // Incremental sync: fetch blocks after last synced height
        log::info!(
            "Incremental sync from height {} (last synced: {})",
            last_synced + 1,
            last_synced
        );
        rpc_client
            .fetch_blocks_range(last_synced + 1, &mut utxo_map)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch blockchain: {}", e))?
    };

    if blocks.is_empty() {
        log::debug!("ℹ️  No new blocks from node RPC");
        return Ok(());
    }

    let latest_height = blocks.iter().map(|b| b.height).max().unwrap_or(last_synced);
    log::info!("🔄 ExplorerSync: {} new blocks from RPC, height {} -> {}", blocks.len(), last_synced, latest_height);

    // Index all blocks
    let mut new_blocks = 0;
    let mut new_transactions = 0;

    for block in &blocks {
        log::debug!("💾 ExplorerSync: Persisting block height={} to local DB", block.height);
        db.save_block(block)?;
        new_blocks += 1;
    }

    for tx in &transactions {
        db.save_transaction(tx)?;
        new_transactions += 1;

        // Update address info for involved addresses
        if let Err(e) = db.update_address_info(&tx.from) {
            error!("Failed to update address info for {}: {}", tx.from, e);
        }
        if let Err(e) = db.update_address_info(&tx.to) {
            error!("Failed to update address info for {}: {}", tx.to, e);
        }
    }

    // Update metadata
    db.set_block_count(latest_height)?;
    db.set_transaction_count(latest_height)?; // Each block has 1 tx (coinbase)
    db.set_last_synced_height(latest_height)?;

    if new_blocks > 0 || new_transactions > 0 {
        info!(
            "Indexed {} new blocks, {} new transactions (Height: {} -> {}, Total: {} blocks, {} txs)",
            new_blocks, new_transactions, last_synced, latest_height, latest_height, latest_height
        );
    }

    Ok(())
}
