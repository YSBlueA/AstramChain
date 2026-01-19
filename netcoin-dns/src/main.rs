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
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
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
}

// Register a node
async fn register_node(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let node_id = format!("{}:{}", req.address, req.port);
    
    let node_info = NodeInfo {
        address: req.address.clone(),
        port: req.port,
        version: req.version.clone(),
        height: req.height,
        last_seen: Utc::now().timestamp(),
    };
    
    state.nodes.write().insert(node_id.clone(), node_info);
    info!("Registered node: {} (height: {})", node_id, req.height);
    
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
    
    // Sort by height (descending) and last_seen
    node_list.sort_by(|a, b| {
        b.height.cmp(&a.height)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
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
    
    // Spawn periodic cleanup task
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            cleanup_state.cleanup_stale_nodes();
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
