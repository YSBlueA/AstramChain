use crate::db::ExplorerDB;
use crate::rpc::NodeRpcClient;
use crate::state::BlockchainStats;
use actix_web::{HttpResponse, web};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub timestamp: String,
}

/// Reorg alert information for security monitoring
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct ReorgAlert {
    pub severity: String, // "warning" (depth 6-49) or "critical" (depth 50+)
    pub depth: u64,
    pub old_tip_height: u64,
    pub old_tip_hash: String,
    pub new_tip_height: u64,
    pub new_tip_hash: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
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
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now().to_rfc3339(),
    })
}

// 모든 블록 조회
pub async fn get_blocks(
    db: web::Data<Arc<ExplorerDB>>,
    query: web::Query<PaginationParams>,
) -> HttpResponse {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20);
    log::info!("📦 API: Fetching blocks - page: {}, limit: {}", page, limit);

    match db.get_blocks(page, limit) {
        Ok(blocks) => {
            log::info!("✅ API: Retrieved {} blocks from DB", blocks.len());
            let total = db.get_block_count().unwrap_or(0);
            HttpResponse::Ok().json(serde_json::json!({
                "blocks": blocks,
                "page": page,
                "limit": limit,
                "total": total,
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to fetch blocks: {}", e)
        })),
    }
}

// 높이로 블록 조회
pub async fn get_block_by_height(
    db: web::Data<Arc<ExplorerDB>>,
    path: web::Path<u64>,
) -> HttpResponse {
    let height = path.into_inner();

    match db.get_block_by_height(height) {
        Ok(Some(block)) => HttpResponse::Ok().json(block),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Block not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Database error: {}", e)
        })),
    }
}

// 해시로 블록 조회
pub async fn get_block_by_hash(
    db: web::Data<Arc<ExplorerDB>>,
    path: web::Path<String>,
) -> HttpResponse {
    let hash = path.into_inner();

    match db.get_block_by_hash(&hash) {
        Ok(Some(block)) => HttpResponse::Ok().json(block),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Block not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Database error: {}", e)
        })),
    }
}

// 모든 트랜잭션 조회
pub async fn get_transactions(
    db: web::Data<Arc<ExplorerDB>>,
    query: web::Query<PaginationParams>,
) -> HttpResponse {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20);
    log::info!(
        "💾 API: Fetching transactions - page: {}, limit: {}",
        page,
        limit
    );

    match db.get_transactions(page, limit) {
        Ok(transactions) => {
            log::info!(
                "✅ API: Retrieved {} transactions from DB",
                transactions.len()
            );
            let total = db.get_transaction_count().unwrap_or(0);
            HttpResponse::Ok().json(serde_json::json!({
                "transactions": transactions,
                "page": page,
                "limit": limit,
                "total": total,
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to fetch transactions: {}", e)
        })),
    }
}

// 해시로 트랜잭션 조회
pub async fn get_transaction_by_hash(
    db: web::Data<Arc<ExplorerDB>>,
    path: web::Path<String>,
) -> HttpResponse {
    let hash = path.into_inner();

    log::info!("🔍 Looking up transaction by hash: {}", hash);

    match db.get_transaction(&hash) {
        Ok(Some(tx)) => {
            log::info!("✅ Found transaction: {}", hash);
            HttpResponse::Ok().json(tx)
        }
        Ok(None) => {
            log::warn!("❌ Transaction not found: {}", hash);
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Transaction not found"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Database error: {}", e)
        })),
    }
}

// 블록체인 통계 조회
pub async fn get_blockchain_stats(db: web::Data<Arc<ExplorerDB>>) -> HttpResponse {
    match db.get_stats() {
        Ok((total_blocks, total_transactions, total_volume)) => {
            let average_block_time = db.compute_avg_block_time(50).unwrap_or(0.0);
            let current_difficulty = db
                .get_latest_block()
                .ok()
                .flatten()
                .map(|b| b.difficulty)
                .unwrap_or(1);

            let stats = BlockchainStats {
                total_blocks,
                total_transactions,
                total_volume,
                average_block_time,
                average_block_size: 250,
                current_difficulty,
                network_hashrate: "0.00 TH/s".to_string(),
            };

            HttpResponse::Ok().json(stats)
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to fetch stats: {}", e)
        })),
    }
}

// 주소별 정보 조회
pub async fn get_address_info(
    db: web::Data<Arc<ExplorerDB>>,
    path: web::Path<String>,
) -> HttpResponse {
    let address = path.into_inner();
    log::info!("📍 Explorer handler: Fetching address info for {}", address);

    match db.get_address_info(&address) {
        Ok(Some(info)) => HttpResponse::Ok().json(info),
        Ok(None) => {
            // 캐시되지 않은 경우, 새로 계산
            match db.update_address_info(&address) {
                Ok(info) => HttpResponse::Ok().json(info),
                Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": format!("Failed to calculate address info: {}", e)
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Database error: {}", e)
        })),
    }
}

// Node status proxy
pub async fn get_node_status(rpc: web::Data<Arc<NodeRpcClient>>) -> HttpResponse {
    match rpc.fetch_status().await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(e) => HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "message": e
        })),
    }
}
