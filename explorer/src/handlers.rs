use crate::rpc::NodeRpcClient;
use crate::state::{AddressInfo, AppState, BlockInfo, BlockchainStats, TransactionInfo};
use actix_web::{HttpResponse, web};
use chrono::Utc;
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub timestamp: String,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

// 헬스 체크 엔드포인트
pub async fn health() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "healthy".to_string(),
        version: "0.1.0".to_string(),
        timestamp: Utc::now().to_rfc3339(),
    })
}

// 모든 블록 조회
pub async fn get_blocks(
    state: web::Data<Arc<Mutex<AppState>>>,
    query: web::Query<PaginationParams>,
) -> HttpResponse {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20);

    let app_state = state.lock().unwrap();

    let start = ((page - 1) * limit) as usize;
    let end = (page * limit) as usize;

    let blocks: Vec<BlockInfo> = app_state
        .cached_blocks
        .iter()
        .rev()
        .skip(start)
        .take(limit as usize)
        .cloned()
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "blocks": blocks,
        "page": page,
        "limit": limit,
        "total": app_state.cached_blocks.len(),
    }))
}

// 높이로 블록 조회
pub async fn get_block_by_height(
    state: web::Data<Arc<Mutex<AppState>>>,
    path: web::Path<u64>,
) -> HttpResponse {
    let height = path.into_inner();
    let app_state = state.lock().unwrap();

    if let Some(block) = app_state.cached_blocks.iter().find(|b| b.height == height) {
        HttpResponse::Ok().json(block)
    } else {
        HttpResponse::NotFound().json(serde_json::json!({
            "error": "Block not found"
        }))
    }
}

// 해시로 블록 조회
pub async fn get_block_by_hash(
    state: web::Data<Arc<Mutex<AppState>>>,
    path: web::Path<String>,
) -> HttpResponse {
    let hash = path.into_inner();
    let app_state = state.lock().unwrap();

    if let Some(block) = app_state.cached_blocks.iter().find(|b| b.hash == hash) {
        HttpResponse::Ok().json(block)
    } else {
        HttpResponse::NotFound().json(serde_json::json!({
            "error": "Block not found"
        }))
    }
}

// 모든 트랜잭션 조회
pub async fn get_transactions(
    state: web::Data<Arc<Mutex<AppState>>>,
    query: web::Query<PaginationParams>,
) -> HttpResponse {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20);

    let app_state = state.lock().unwrap();

    let start = ((page - 1) * limit) as usize;
    let end = (page * limit) as usize;

    let transactions: Vec<TransactionInfo> = app_state
        .cached_transactions
        .iter()
        .rev()
        .skip(start)
        .take(limit as usize)
        .cloned()
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "transactions": transactions,
        "page": page,
        "limit": limit,
        "total": app_state.cached_transactions.len(),
    }))
}

// 해시로 트랜잭션 조회
pub async fn get_transaction_by_hash(
    state: web::Data<Arc<Mutex<AppState>>>,
    path: web::Path<String>,
) -> HttpResponse {
    let mut hash = path.into_inner();

    // If hash starts with 0x, it might be an Ethereum transaction hash
    // Try to resolve it to NetCoin txid
    if hash.starts_with("0x") {
        let rpc = NodeRpcClient::new("http://127.0.0.1:8333");
        if let Ok(netcoin_txid) = rpc.resolve_eth_hash(&hash).await {
            log::info!(
                "🔄 Resolved ETH hash {} to NetCoin txid {}",
                hash,
                netcoin_txid
            );
            hash = netcoin_txid;
        } else {
            // Strip 0x and try as regular hash
            hash = hash.strip_prefix("0x").unwrap_or(&hash).to_string();
        }
    }

    let app_state = state.lock().unwrap();

    if let Some(tx) = app_state
        .cached_transactions
        .iter()
        .find(|t| t.hash == hash)
    {
        HttpResponse::Ok().json(tx)
    } else {
        HttpResponse::NotFound().json(serde_json::json!({
            "error": "Transaction not found"
        }))
    }
}

// 블록체인 통계 조회
pub async fn get_blockchain_stats(state: web::Data<Arc<Mutex<AppState>>>) -> HttpResponse {
    // snapshot cached data (do not hold lock across network calls)
    let (cached_blocks, cached_txs) = {
        let s = state.lock().unwrap();
        (s.cached_blocks.clone(), s.cached_transactions.clone())
    };

    // Try to fetch authoritative counts from Node DB
    let rpc = NodeRpcClient::new("http://127.0.0.1:8333");
    let (total_blocks, total_transactions) = match rpc.fetch_counts().await {
        Ok((b, t)) => (b, t),
        Err(_) => (cached_blocks.len() as u64, cached_txs.len() as u64),
    };

    // Fetch total volume from DB via Node
    let total_volume: U256 = match rpc.fetch_total_volume().await {
        Ok(vol) => vol,
        Err(_) => cached_txs
            .iter()
            .fold(U256::zero(), |acc, t| acc + t.amount + t.fee),
    };

    let average_block_time = if cached_blocks.len() > 1 {
        if let (Some(first), Some(last)) = (cached_blocks.first(), cached_blocks.last()) {
            let duration = (last.timestamp - first.timestamp).num_seconds() as f64;
            duration / ((cached_blocks.len() - 1) as f64)
        } else {
            0.0
        }
    } else {
        0.0
    };

    let stats = BlockchainStats {
        total_blocks,
        total_transactions,
        total_volume,
        average_block_time,
        average_block_size: 250, // 평균값
        current_difficulty: cached_blocks.last().map(|b| b.difficulty).unwrap_or(0),
        network_hashrate: "0.00 TH/s".to_string(),
    };

    HttpResponse::Ok().json(stats)
}

// 주소별 정보 조회
pub async fn get_address_info(
    state: web::Data<Arc<Mutex<AppState>>>,
    path: web::Path<String>,
) -> HttpResponse {
    let address = path.into_inner();
    log::info!("📍 Explorer handler: Fetching address info for {}", address);

    let info: AddressInfo = {
        // Try Node RPC first
        let rpc = NodeRpcClient::new("http://127.0.0.1:8333");

        match rpc.fetch_address_info(&address).await {
            Ok((balance, received, sent, transaction_count)) => {
                log::info!(
                    "✅ Got from Node RPC - balance: {}, received: {}, sent: {}, tx_count: {}",
                    balance,
                    received,
                    sent,
                    transaction_count
                );
                let app_state = state.lock().unwrap();

                let last_transaction = app_state
                    .cached_transactions
                    .iter()
                    .filter(|t| t.from == address || t.to == address)
                    .max_by_key(|t| t.timestamp)
                    .map(|t| t.timestamp);

                let info = AddressInfo {
                    address: address.clone(),
                    balance,
                    sent,
                    received,
                    transaction_count,
                    last_transaction,
                };

                log::info!(
                    "📤 Sending to client - balance: {}, received: {}, sent: {}",
                    info.balance,
                    info.received,
                    info.sent
                );

                info
            }

            Err(_) => {
                // Fallback to cached data
                let app_state = state.lock().unwrap();

                let mut sent = U256::zero();
                let mut received = U256::zero();
                let mut last_transaction: Option<chrono::DateTime<Utc>> = None;

                for tx in &app_state.cached_transactions {
                    if tx.to == address {
                        received += tx.amount;
                        last_transaction = Some(tx.timestamp);
                    }
                    if tx.from == address {
                        sent += tx.amount + tx.fee;
                        last_transaction = Some(tx.timestamp);
                    }
                }

                let balance = if received > sent {
                    received - sent
                } else {
                    U256::zero()
                };

                let transaction_count = app_state
                    .cached_transactions
                    .iter()
                    .filter(|t| t.from == address || t.to == address)
                    .count();

                AddressInfo {
                    address,
                    balance,
                    sent,
                    received,
                    transaction_count,
                    last_transaction,
                }
            }
        }
    };

    HttpResponse::Ok().json(info)
}

// 노드 상태 조회 엔드포인트
pub async fn get_node_status() -> HttpResponse {
    let rpc = NodeRpcClient::new("http://127.0.0.1:8333");

    match rpc.fetch_node_status().await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(e) => HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "Failed to connect to node",
            "message": e.to_string(),
            "status": "offline"
        })),
    }
}
