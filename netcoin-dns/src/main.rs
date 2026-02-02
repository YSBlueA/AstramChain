use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use clap::Parser;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to bind DNS server
    #[arg(short, long, default_value = "8053")]
    port: u16,

    /// Maximum age of nodes in seconds before considering them stale
    #[arg(short, long, default_value = "3600")]
    max_age: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub address: String,
    pub port: u16,
    pub version: String,
    pub height: u64,
    pub last_seen: i64,
    pub first_seen: i64,   // When node was first registered
    pub uptime_hours: f64, // Hours since first registration
}

#[derive(Clone)]
pub struct AppState {
    nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
    max_age: u64,
}

#[derive(Deserialize)]
struct RegisterRequest {
    address: String,
    port: u16,
    version: String,
    height: u64,
}

#[derive(Serialize)]
struct RegisterResponse {
    success: bool,
    message: String,
    node_count: usize,
}

#[derive(Serialize)]
struct NodesResponse {
    nodes: Vec<NodeInfo>,
    count: usize,
}

#[derive(Deserialize)]
struct GetNodesQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    min_height: Option<u64>,
}

impl AppState {
    fn new(max_age: u64) -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            max_age,
        }
    }

    fn cleanup_stale_nodes(&self) {
        let now = Utc::now().timestamp();
        let mut nodes = self.nodes.write();
        let before_count = nodes.len();

        nodes.retain(|_, node| {
            let age = now - node.last_seen;
            age < self.max_age as i64
        });

        let removed = before_count - nodes.len();
        if removed > 0 {
            info!("Cleaned up {} stale nodes", removed);
        }
    }

    /// Check node connectivity and remove unreachable nodes
    async fn health_check_nodes(&self) {
        let node_addresses: Vec<(String, String, u16)> = {
            let nodes = self.nodes.read();
            nodes
                .iter()
                .map(|(id, node)| (id.clone(), node.address.clone(), node.port))
                .collect()
        };

        let total_nodes = node_addresses.len();
        if total_nodes == 0 {
            return;
        }

        info!("Starting health check for {} nodes...", total_nodes);

        let mut to_remove = Vec::new();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();

        // Process in batches to avoid overload
        const BATCH_SIZE: usize = 20;
        const MAX_CONCURRENT: usize = 5;

        for batch in node_addresses.chunks(BATCH_SIZE) {
            let mut tasks = Vec::new();

            for (node_id, address, _port) in batch {
                let client = client.clone();
                let node_id = node_id.clone();
                let address = address.clone();

                let task = tokio::spawn(async move {
                    let health_url = format!("http://{}:8333/health", address);
                    match client.get(&health_url).send().await {
                        Ok(response) if response.status().is_success() => (node_id, true),
                        _ => (node_id, false),
                    }
                });

                tasks.push(task);

                // Limit concurrent requests
                if tasks.len() >= MAX_CONCURRENT {
                    for task in tasks.drain(..) {
                        if let Ok((node_id, is_reachable)) = task.await {
                            if !is_reachable {
                                to_remove.push(node_id);
                            }
                        }
                    }
                }
            }

            // Process remaining tasks in batch
            for task in tasks {
                if let Ok((node_id, is_reachable)) = task.await {
                    if !is_reachable {
                        to_remove.push(node_id);
                    }
                }
            }

            // Small delay between batches
            if batch.len() == BATCH_SIZE {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        if !to_remove.is_empty() {
            let mut nodes = self.nodes.write();
            for node_id in &to_remove {
                warn!("Removing unreachable node: {}", node_id);
                nodes.remove(node_id);
            }
            info!(
                "Health check complete: removed {} of {} nodes",
                to_remove.len(),
                total_nodes
            );
        } else {
            info!(
                "Health check complete: all {} nodes are healthy",
                total_nodes
            );
        }
    }
}

// Register a node
async fn register_node(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let node_id = format!("{}:{}", req.address, req.port);
    let now = Utc::now().timestamp();

    // Check if node already exists to preserve first_seen
    let (first_seen, uptime_hours) = {
        let nodes = state.nodes.read();
        if let Some(existing) = nodes.get(&node_id) {
            let hours = (now - existing.first_seen) as f64 / 3600.0;
            (existing.first_seen, hours)
        } else {
            (now, 0.0)
        }
    };

    let node_info = NodeInfo {
        address: req.address.clone(),
        port: req.port,
        version: req.version.clone(),
        height: req.height,
        last_seen: now,
        first_seen,
        uptime_hours,
    };

    state.nodes.write().insert(node_id.clone(), node_info);
    info!(
        "Registered node: {} (height: {}, uptime: {:.1}h)",
        node_id, req.height, uptime_hours
    );

    let node_count = state.nodes.read().len();

    Json(RegisterResponse {
        success: true,
        message: format!("Node {} registered successfully", node_id),
        node_count,
    })
}

// Get list of nodes
async fn get_nodes(
    State(state): State<AppState>,
    Query(query): Query<GetNodesQuery>,
) -> impl IntoResponse {
    state.cleanup_stale_nodes();

    let nodes = state.nodes.read();
    let mut node_list: Vec<NodeInfo> = nodes.values().cloned().collect();

    // Filter by minimum height if specified
    if let Some(min_height) = query.min_height {
        node_list.retain(|n| n.height >= min_height);
    }

    // Calculate values needed for scoring before sorting
    let max_height = node_list.iter().map(|n| n.height).max().unwrap_or(1) as f64;
    let now = Utc::now().timestamp();

    // Sort by composite score:
    // - 40% weight: blockchain height
    // - 30% weight: uptime (capped at 168 hours = 1 week)
    // - 30% weight: recent activity (last_seen)
    node_list.sort_by(|a, b| {
        // Height score (normalized, higher is better)
        let height_score_a = (a.height as f64 / max_height.max(1.0)) * 0.4;
        let height_score_b = (b.height as f64 / max_height.max(1.0)) * 0.4;

        // Uptime score (capped at 168 hours, normalized)
        let uptime_score_a = (a.uptime_hours.min(168.0) / 168.0) * 0.3;
        let uptime_score_b = (b.uptime_hours.min(168.0) / 168.0) * 0.3;

        // Recency score (last seen within 5 minutes = 1.0, older = lower)
        let recency_a = ((300.0 - (now - a.last_seen) as f64).max(0.0) / 300.0) * 0.3;
        let recency_b = ((300.0 - (now - b.last_seen) as f64).max(0.0) / 300.0) * 0.3;

        let total_score_a = height_score_a + uptime_score_a + recency_a;
        let total_score_b = height_score_b + uptime_score_b + recency_b;

        // Sort descending by total score
        total_score_b
            .partial_cmp(&total_score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Apply limit if specified
    if let Some(limit) = query.limit {
        node_list.truncate(limit);
    }

    let count = node_list.len();

    Json(NodesResponse {
        nodes: node_list,
        count,
    })
}

// Health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let node_count = state.nodes.read().len();

    Json(serde_json::json!({
        "status": "healthy",
        "node_count": node_count,
        "timestamp": Utc::now().timestamp(),
    }))
}

// Get statistics
async fn get_stats(State(state): State<AppState>) -> impl IntoResponse {
    state.cleanup_stale_nodes();

    let nodes = state.nodes.read();
    let node_count = nodes.len();

    let mut versions: HashMap<String, usize> = HashMap::new();
    let mut max_height = 0u64;
    let mut total_height = 0u64;

    for node in nodes.values() {
        *versions.entry(node.version.clone()).or_insert(0) += 1;
        max_height = max_height.max(node.height);
        total_height += node.height;
    }

    let avg_height = if node_count > 0 {
        total_height / node_count as u64
    } else {
        0
    };

    Json(serde_json::json!({
        "node_count": node_count,
        "max_height": max_height,
        "avg_height": avg_height,
        "versions": versions,
        "timestamp": Utc::now().timestamp(),
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();

    info!("Starting Netcoin DNS Server...");
    info!("Max node age: {} seconds", args.max_age);

    let state = AppState::new(args.max_age);

    // Spawn periodic cleanup task (removes stale nodes based on last_seen)
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            cleanup_state.cleanup_stale_nodes();
        }
    });

    // Spawn periodic health check task (actively checks node connectivity)
    let health_check_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(600)); // 10 minutes
        interval.tick().await; // Skip first immediate tick
        loop {
            interval.tick().await;
            info!("Starting periodic health check of registered nodes...");
            health_check_state.health_check_nodes().await;
        }
    });

    // Build router
    let app = Router::new()
        .route("/", get(|| async { "Netcoin DNS Server" }))
        .route("/health", get(health_check))
        .route("/register", post(register_node))
        .route("/nodes", get(get_nodes))
        .route("/stats", get(get_stats))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    info!("DNS server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
