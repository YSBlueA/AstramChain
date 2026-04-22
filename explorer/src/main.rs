mod api;
mod db;
mod handlers;
mod rpc;
mod state;

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use clap::Parser;
use flexi_logger::{Age, Cleanup, Criterion, Duplicate, FileSpec, Logger, Naming, WriteMode};
use db::ExplorerDB;
use log::{error, info};
use rpc::NodeRpcClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{Duration, interval};

// ─── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "Astram-explorer", about = "Astram blockchain explorer")]
struct Cli {
    /// Path to explorer settings file (default: config/explorerSettings.conf)
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
}

// ─── Settings ────────────────────────────────────────────────────────────────

struct ExplorerSettings {
    db_path: String,
    node_rpc_url: String,
    bind_addr: String,
    port: u16,
    sync_interval_secs: u64,
}

fn resolve_conf_path() -> PathBuf {
    let exe_path = std::env::current_exe().ok().and_then(|p| {
        p.parent().map(|d| d.join("config/explorerSettings.conf"))
    });
    if let Some(ref p) = exe_path {
        if p.exists() { return p.clone(); }
    }
    let cwd = PathBuf::from("config/explorerSettings.conf");
    if cwd.exists() { return cwd; }
    exe_path.unwrap_or(cwd)
}

fn load_conf(path: &std::path::Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[WARN] explorerSettings not found at {:?}: {}", path, e);
            return map;
        }
    };
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn get_setting(key: &str, file: &HashMap<String, String>, default: &str) -> String {
    std::env::var(key)
        .ok()
        .or_else(|| file.get(key).cloned())
        .unwrap_or_else(|| default.to_string())
}

fn load_explorer_settings(config_path: Option<PathBuf>) -> ExplorerSettings {
    let path = config_path.unwrap_or_else(resolve_conf_path);
    let file = load_conf(&path);
    if path.exists() {
        eprintln!("[INFO] Loaded explorer settings from {:?}", path);
    }

    ExplorerSettings {
        db_path:            get_setting("DB_PATH",             &file, "explorer_data"),
        node_rpc_url:       get_setting("NODE_RPC_URL",        &file, "http://127.0.0.1:19533"),
        bind_addr:          get_setting("BIND_ADDR",           &file, "0.0.0.0"),
        port:               get_setting("PORT",                &file, "8080").parse().unwrap_or(8080),
        sync_interval_secs: get_setting("SYNC_INTERVAL_SECS", &file, "10").parse().unwrap_or(10),
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let settings = load_explorer_settings(cli.config);

    Logger::try_with_env_or_str("info")
        .expect("Failed to build logger")
        .log_to_file(FileSpec::default().directory("logs").basename("explorer"))
        .duplicate_to_stderr(Duplicate::All)
        .rotate(
            Criterion::Age(Age::Day),
            Naming::Timestamps,
            Cleanup::KeepLogFiles(5),
        )
        .write_mode(WriteMode::BufferAndFlush)
        .start()
        .expect("Failed to start logger");

    info!("Astram Explorer starting...");

    // Explorer database initialization
    let explorer_db = Arc::new(
        ExplorerDB::new(&settings.db_path).expect("Failed to open explorer database")
    );
    info!("Explorer database initialized at {}", settings.db_path);

    // Background sync with the Node process
    let db_sync = explorer_db.clone();
    let rpc_client = Arc::new(NodeRpcClient::new(&settings.node_rpc_url));
    let rpc_for_sync = rpc_client.clone();
    let sync_secs = settings.sync_interval_secs;
    tokio::spawn(async move {
        info!("Starting blockchain indexing...");

        match sync_blockchain(&db_sync, &rpc_for_sync).await {
            Ok(()) => info!("Initial blockchain sync completed"),
            Err(e) => error!("Failed to sync blockchain on startup: {}", e),
        }

        let mut sync_interval = interval(Duration::from_secs(sync_secs));
        loop {
            sync_interval.tick().await;
            if let Err(e) = sync_blockchain(&db_sync, &rpc_for_sync).await {
                error!("Failed to sync blockchain: {}", e);
            }
        }
    });

    let server_address = settings.bind_addr.clone();
    let server_port = settings.port;

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
                    .route("/richlist", web::get().to(handlers::get_richlist))
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
    // UTXO 데이터가 없으면 전체 재동기화 필요 (이전 버전 DB 마이그레이션)
    let last_synced = {
        let height = db.get_last_synced_height()?;
        if height > 0 && !db.has_utxo_data() {
            log::info!("⚠️  UTXO data missing in Explorer DB (old format). Resetting to full re-sync...");
            db.set_last_synced_height(0)?;
            0
        } else {
            height
        }
    };

    let mut utxo_map = std::collections::HashMap::new();
    let (blocks, transactions, created_utxos, spent_utxos) = if last_synced == 0 {
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

    // UTXO DB 업데이트: WriteBatch로 한 번에 처리
    if let Err(e) = db.apply_utxo_changes(&created_utxos, &spent_utxos) {
        error!("Failed to apply UTXO changes: {}", e);
    }

    // Collect unique addresses touched in this batch to avoid N+1 per-tx updates
    let mut addresses_to_update = std::collections::HashSet::new();

    for tx in &transactions {
        db.save_transaction(tx)?;
        new_transactions += 1;

        addresses_to_update.insert(tx.from.clone());
        addresses_to_update.insert(tx.to.clone());
    }

    // Batch address update: one scan per unique address instead of two per transaction
    for address in addresses_to_update {
        if let Err(e) = db.update_address_info(&address) {
            error!("Failed to update address info for {}: {}", address, e);
        }
    }

    // Update sync metadata (block_count / tx_count / total_volume are maintained by save_block/save_transaction)
    db.set_last_synced_height(latest_height)?;

    if new_blocks > 0 || new_transactions > 0 {
        let total_blocks = db.get_block_count().unwrap_or(0);
        let total_txs = db.get_transaction_count().unwrap_or(0);
        info!(
            "Indexed {} new blocks, {} new transactions (Height: {} -> {}, DB total: {} blocks, {} txs)",
            new_blocks, new_transactions, last_synced, latest_height, total_blocks, total_txs
        );
    }

    Ok(())
}
