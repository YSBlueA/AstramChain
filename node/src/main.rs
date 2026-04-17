// Use library exports instead of declaring local modules to avoid duplicate crate types
use Astram_core::Blockchain;
use astram_config::config::Config;
use astram_node::ChainState;
use astram_node::MempoolState;
use astram_node::MiningState;
use astram_node::NodeHandle;
use astram_node::NodeHandles;
use astram_node::NodeMeta;
use astram_node::p2p::service::P2PService;
use astram_node::server::{run_server, run_public_server};
use flexi_logger::{Age, Cleanup, Criterion, Duplicate, FileSpec, Logger, Naming, WriteMode};
use log::{info, warn};
use serde::Deserialize;
use serde_json::Value;

use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as OtherOrdering;
use std::sync::{Arc, Mutex};
use tokio::signal;
use tokio::time::{Duration, sleep};


#[derive(Debug, Clone, Deserialize)]
struct DnsNodeInfo {
    address: String,
    port: u16,
    #[serde(rename = "version")]
    _version: String,
    height: u64,
    #[serde(rename = "last_seen")]
    _last_seen: i64,
    #[serde(rename = "first_seen")]
    _first_seen: i64,
    uptime_hours: f64,
}

#[derive(Debug, Deserialize)]
struct DnsNodesResponse {
    nodes: Vec<DnsNodeInfo>,
    count: usize,
}

#[derive(Debug, Clone)]
struct NodeSettings {
    data_dir: String,
    p2p_bind_addr: String,
    p2p_port: u16,
    http_bind_addr: String,
    http_port: u16,
    /// Public RPC port — exposes only safe read-only endpoints (no wallet/mining/relay).
    /// Set to 0 to disable. Default: 18533.
    public_rpc_port: u16,
    eth_rpc_bind_addr: String,
    eth_rpc_port: u16,
    dns_server_url: String,
    bootstrap_peers: Vec<String>,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            p2p_bind_addr: "0.0.0.0".to_string(),
            p2p_port: 8335,
            http_bind_addr: "127.0.0.1".to_string(),
            http_port: 19533,
            public_rpc_port: 18533,
            eth_rpc_bind_addr: "127.0.0.1".to_string(),
            eth_rpc_port: 8545,
            dns_server_url: "http://161.33.19.183:8053".to_string(),
            bootstrap_peers: Vec::new(),
        }
    }
}

fn default_data_dir() -> String {
    let home = dirs::home_dir().expect("Cannot find home directory");

    if cfg!(target_os = "windows") {
        let base = dirs::data_dir().unwrap_or(home).join("Astram");
        return base.join("data").to_string_lossy().into_owned();
    }

    home.join(".Astram")
        .join("data")
        .to_string_lossy()
        .into_owned()
}

fn expand_path_value(value: &str) -> String {
    let expanded = shellexpand::tilde(value).into_owned();
    if expanded.contains("%USERPROFILE%") {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return expanded.replace("%USERPROFILE%", &profile);
        }
    }
    expanded
}

fn resolve_node_settings_path() -> PathBuf {
    let exe_path = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("config/nodeSettings.conf"))
    });

    if let Some(ref path) = exe_path {
        if path.exists() {
            return path.clone();
        }
    }

    let cwd_path = PathBuf::from("config/nodeSettings.conf");
    if cwd_path.exists() {
        return cwd_path;
    }

    exe_path.unwrap_or(cwd_path)
}

fn load_node_settings() -> NodeSettings {
    let mut settings = NodeSettings::default();
    let path = resolve_node_settings_path();

    match fs::read_to_string(&path) {
        Ok(contents) => {
            for (line_no, raw_line) in contents.lines().enumerate() {
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                let (key, value) = match line.split_once('=') {
                    Some(pair) => pair,
                    None => {
                        println!(
                            "[WARN] Invalid node setting on line {}: {}",
                            line_no + 1,
                            raw_line
                        );
                        continue;
                    }
                };

                let key = key.trim();
                let value = value.trim();
                match key {
                    "DATA_DIR" => settings.data_dir = expand_path_value(value),
                    "P2P_BIND_ADDR" => settings.p2p_bind_addr = value.to_string(),
                    "P2P_PORT" => settings.p2p_port = value.parse().unwrap_or(settings.p2p_port),
                    "HTTP_BIND_ADDR" => settings.http_bind_addr = value.to_string(),
                    "HTTP_PORT" => settings.http_port = value.parse().unwrap_or(settings.http_port),
                    "PUBLIC_RPC_PORT" => settings.public_rpc_port = value.parse().unwrap_or(settings.public_rpc_port),
                    "ETH_RPC_BIND_ADDR" => settings.eth_rpc_bind_addr = value.to_string(),
                    "ETH_RPC_PORT" => {
                        settings.eth_rpc_port = value.parse().unwrap_or(settings.eth_rpc_port)
                    }
                    "DNS_SERVER_URL" => settings.dns_server_url = value.to_string(),
                    "ASTRAM_NETWORK" | "ASTRAM_NETWORK_ID" | "ASTRAM_CHAIN_ID" | "ASTRAM_NETWORK_MAGIC" => {
                        #[cfg(debug_assertions)]
                        {
                            // Debug builds: allow environment variable overrides for testing
                            unsafe {
                                std::env::set_var(key, value);
                            }
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            // Release builds: network parameters are hardcoded, ignore config overrides
                            warn!("[RELEASE] Network parameter override '{}' ignored - using hardcoded mainnet values", key);
                        }
                    }
                    "BOOTSTRAP_PEERS" => {
                        settings.bootstrap_peers = value
                            .split(',')
                            .map(|entry| entry.trim().to_string())
                            .filter(|entry| !entry.is_empty())
                            .collect();
                    }
                    _ => println!("[WARN] Unknown node setting key: {}", key),
                }
            }
        }
        Err(err) => {
            println!("[WARN] Node settings file not found at {:?}: {}", path, err);
        }
    }

    settings.data_dir = expand_path_value(&settings.data_dir);
    settings
}

fn to_socket_addr(addr: &str, port: u16, fallback: SocketAddr) -> SocketAddr {
    format!("{}:{}", addr, port).parse().unwrap_or(fallback)
}

#[tokio::main]
async fn main() {
    println!("[INFO] Astram node starting...");

    // Load settings first so we can place log files inside data_dir/logs/
    let node_settings = Arc::new(load_node_settings());

    let log_dir = format!("{}/logs", node_settings.data_dir);
    let _ = fs::create_dir_all(&log_dir);

    #[cfg(debug_assertions)]
    let log_spec = "info, warp=warn, hyper=warn, reqwest=warn, Astram::http=warn";

    #[cfg(not(debug_assertions))]
    let log_spec =
        "info, astram_node::p2p::manager=warn, warp=warn, hyper=warn, reqwest=warn, Astram::http=warn";

    Logger::try_with_env_or_str(log_spec)
        .expect("Failed to build logger")
        .log_to_file(FileSpec::default().directory(&log_dir).basename("node"))
        .duplicate_to_stderr(Duplicate::All)
        .rotate(
            Criterion::Age(Age::Day),
            Naming::Timestamps,
            Cleanup::KeepLogFiles(5),
        )
        .write_mode(WriteMode::BufferAndFlush)
        .start()
        .expect("Failed to start logger");

    let cfg = Config::load();

    // Read wallet address from file (expand paths configured via CLI)
    let wallet_path = cfg.wallet_path_resolved();
    let wallet_file =
        fs::read_to_string(wallet_path.as_path()).expect("Failed to read wallet file");
    let wallet: Value = serde_json::from_str(&wallet_file).expect("Failed to parse wallet JSON");
    let miner_address = wallet["address"]
        .as_str()
        .expect("Failed to get address from wallet")
        .to_string();

    // DB path for core blockchain
    let db_path = node_settings.data_dir.clone();
    if let Err(err) = fs::create_dir_all(&db_path) {
        eprintln!(
            "[ERROR] Failed to create data directory {}: {}",
            db_path, err
        );
        std::process::exit(1);
    }

    print!("Initialize Block chain...\n");

    // Check for stale LOCK file and remove it if necessary
    let lock_path = std::path::Path::new(&db_path).join("LOCK");
    if lock_path.exists() {
        println!("[WARN] Found existing LOCK file, attempting to clean up...");

        // Try to remove stale lock file
        match fs::remove_file(&lock_path) {
            Ok(_) => {
                println!("[INFO] Removed stale LOCK file");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to remove LOCK file: {}", e);
                eprintln!("Another instance may be running. Please stop it first.");
                std::process::exit(1);
            }
        }
    }

    // Initialize core Blockchain (RocksDB-backed)
    let bc = match Blockchain::new(db_path.as_str()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to open blockchain DB: {}", e);
            eprintln!("If another instance is running, please stop it first.");
            std::process::exit(1);
        }
    };
    let bc = Arc::new(Mutex::new(bc));

    // Check and recover tip if needed
    {
        let mut bc_guard = bc.lock().unwrap();
        
        // Always scan DB and recover tip to ensure correct height before sync
        let block_count = bc_guard.count_blocks();
        log::info!("📊 Database contains {} blocks", block_count);
        
        if block_count == 0 {
            log::info!("📭 Empty database - starting fresh blockchain");
        } else {
            log::info!("🔍 Scanning database to find highest block...");
            if let Err(e) = bc_guard.recover_tip() {
                log::error!("❌ Failed to recover tip: {}", e);
            } else if let Some(tip_hash) = &bc_guard.chain_tip {
                if let Ok(Some(header)) = bc_guard.load_header(tip_hash) {
                    log::info!("✅ Blockchain tip recovered: height {} (hash: {})", header.index, &tip_hash[..16]);
                }
            }
        }
    }

    // Initialize P2P networking
    let p2p_service = P2PService::new();

    let mining_state = Arc::new(MiningState::default());

    let p2p_handle = p2p_service.manager();

    let chain_state = Arc::new(Mutex::new(ChainState::default()));
    let node_meta = Arc::new(NodeMeta {
        miner_address: Arc::new(Mutex::new(miner_address.clone())),
        my_public_address: Arc::new(Mutex::new(None)),
        node_start_time: std::time::Instant::now(),
    });

    let node = NodeHandles {
        bc: bc.clone(),
        mempool: Arc::new(Mutex::new(MempoolState::default())),
        mining: mining_state.clone(),
    };

    let node_handle = Arc::new(node);

    // Set current blockchain height in P2P manager
    let my_height = {
        let bc = node_handle.bc.lock().unwrap();
        if let Some(tip_hash) = &bc.chain_tip {
            log::info!("[INIT] Chain tip hash: {}", tip_hash);
            match bc.load_header(tip_hash) {
                Ok(Some(header)) => {
                    let height = header.index;
                    log::info!("[INIT] Successfully loaded tip header at height {}", height);
                    height
                }
                Ok(None) => {
                    log::error!("[INIT] Chain tip '{}' exists but header not found in DB!", tip_hash);
                    0
                }
                Err(e) => {
                    log::error!("[INIT] Failed to load chain tip header: {}", e);
                    0
                }
            }
        } else {
            log::info!("[INIT] No chain tip found (empty blockchain)");
            0
        }
    };
    p2p_handle.set_my_height(my_height);
    info!("[INFO] Local blockchain height set to: {}", my_height);

    let bind_addr = format!("{}:{}", node_settings.p2p_bind_addr, node_settings.p2p_port);

    // Set listening port in P2P manager (for self-connection detection)
    p2p_handle.set_my_listening_port(node_settings.p2p_port);
    p2p_handle.set_my_bind_addr(node_settings.p2p_bind_addr.clone());

    p2p_service
        .start(bind_addr, node_handle.clone(), chain_state.clone())
        .await
        .expect("p2p start failed");

    // Start Ethereum JSON-RPC server for MetaMask
    // Graceful shutdown flag
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    let node_for_shutdown = node_handle.clone();

    // Setup signal handler for graceful shutdown
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                println!("\n[WARN] Shutdown signal received, cleaning up...");
                println!(
                    "[INFO] Note: Connection errors during shutdown are expected and can be ignored"
                );
                shutdown_flag_clone.store(true, OtherOrdering::SeqCst);

                // Cancel ongoing mining immediately
                node_for_shutdown
                    .mining
                    .cancel_flag
                    .store(true, OtherOrdering::SeqCst);
                println!("[WARN] Mining cancellation requested...");
            }
            Err(err) => {
                eprintln!("Error setting up signal handler: {}", err);
            }
        }
    });

    let (task_handles, server_handle) = start_services(
        node_handle.clone(),
        p2p_handle.clone(),
        chain_state.clone(),
        node_meta.clone(),
        shutdown_flag.clone(),
        node_settings.clone(),
    )
    .await;

    // Wait for all background tasks to complete
    println!("[INFO] Waiting for all tasks to complete...");
    for handle in task_handles {
        let _ = handle.await;
    }

    // Give time for active connections to close before aborting server
    println!("[INFO] Closing active connections...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Abort HTTP server (it runs indefinitely)
    // Note: This will cause connection errors for any open browser tabs - this is expected
    server_handle.abort();
    println!("[INFO] HTTP server stopped");

    // Give more time for all resources to be released
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Cleanup: Close database properly
    {
        println!("[INFO] Closing database...");

        // First, try to flush the DB while we still have a reference
        {
            if let Ok(bc) = node_handle.bc.lock() {
                // Flush WAL and compact
                if let Err(e) = bc.db.flush() {
                    log::warn!("Failed to flush DB: {}", e);
                } else {
                    println!("[OK] Database flushed");
                }

                // Cancel IO operations
                bc.db.cancel_all_background_work(true);
                println!("[INFO] Background work cancelled");
            }
        }

        // Give DB time to complete all operations
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Drop the node_handle to release our reference
        drop(node_handle);
    }

    // Final wait to ensure LOCK file is released by OS
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("\n╔═══════════════════════════════════════╗");
    println!("║  Astram node stopped gracefully ✓    ║");
    println!("╚═══════════════════════════════════════╝");

    // Force process exit to ensure all resources are released
    std::process::exit(0);
}

/// Measure network latency to a peer by attempting a quick TCP connection
async fn measure_latency(address: &str) -> Option<u64> {
    let start = std::time::Instant::now();

    match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(address),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            let latency = start.elapsed().as_millis() as u64;
            Some(latency)
        }
        Ok(Err(e)) => {
            log::debug!("Connection to {} failed: {}", address, e);
            None
        }
        Err(_) => {
            log::debug!("Connection to {} timed out after 3s", address);
            None
        }
    }
}

#[derive(Debug, Clone)]
struct ScoredPeer {
    address: String,
    height: u64,
    uptime_hours: f64,
    latency_ms: u64,
    score: f64,
}

/// Get the appropriate localhost address for connecting to a local peer
/// - In WSL: Returns the Windows host IP (from default gateway)
/// - Otherwise: Returns 127.0.0.1
fn get_localhost_address() -> String {
    // Check if we're in WSL environment
    if std::env::var("WSL_DISTRO_NAME").is_ok() || 
       std::env::var("WSL_INTEROP").is_ok() {
        // Method 1: Try to get default gateway from 'ip route' command
        if let Ok(output) = std::process::Command::new("ip")
            .args(&["route", "show", "default"])
            .output() 
        {
            if let Ok(route_output) = String::from_utf8(output.stdout) {
                // Parse: "default via 172.25.128.1 dev eth0"
                for part in route_output.split_whitespace() {
                    // Look for IP address pattern after "via"
                    if part.contains('.') && !part.starts_with("127.") {
                        // Basic validation: check if it looks like an IP
                        let segments: Vec<&str> = part.split('.').collect();
                        if segments.len() == 4 && segments.iter().all(|s| s.parse::<u8>().is_ok()) {
                            info!("🔧 WSL detected: using Windows host IP {} (from default gateway)", part);
                            return part.to_string();
                        }
                    }
                }
            }
        }
        
        // Method 2: Fallback to /etc/resolv.conf nameserver
        if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in contents.lines() {
                if line.trim().starts_with("nameserver") {
                    if let Some(ip) = line.split_whitespace().nth(1) {
                        // Validate it's not a loopback address
                        if !ip.starts_with("127.") {
                            info!("🔧 WSL detected: using Windows host IP {} (from /etc/resolv.conf)", ip);
                            return ip.to_string();
                        }
                    }
                }
            }
        }
        
        warn!("⚠️  WSL detected but could not determine Windows host IP, falling back to 127.0.0.1");
    }
    
    // Default to standard localhost
    "127.0.0.1".to_string()
}

/// Fetch best nodes from DNS server, excluding self
async fn fetch_best_nodes_from_dns(
    node_meta: Arc<NodeMeta>,
    settings: &NodeSettings,
    my_port: u16,
    limit: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Get my public address from state
    let my_address = { node_meta.my_public_address.lock().unwrap().clone() };

    let dns_url = settings.dns_server_url.clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5)) // 5 second timeout
        .build()?;
    let nodes_url = format!("{}/nodes?limit={}", dns_url, limit * 3); // Fetch more to test latency

    info!("Fetching best nodes from DNS server at {}", dns_url);

    let response = client.get(&nodes_url).send().await?;

    if response.status().is_success() {
        let result: DnsNodesResponse = response.json().await?;
        info!("Retrieved {} nodes from DNS server", result.count);

        // Log all discovered nodes for visibility
        for (idx, node) in result.nodes.iter().enumerate() {
            info!(
                "  DNS Node {}: {}:{} (height: {}, uptime: {:.1}h)",
                idx + 1,
                node.address,
                node.port,
                node.height,
                node.uptime_hours
            );
        }

        // Filter out self - use public address if available
        let candidates: Vec<DnsNodeInfo> = result
            .nodes
            .into_iter()
            .filter(|node| {
                let node_id = format!("{}:{}", node.address, node.port);

                // Filter out exact match with public address+port (if we have it)
                if let Some(ref my_public_ip) = my_address {
                    let my_id = format!("{}:{}", my_public_ip, my_port);
                    if node_id == my_id {
                        info!("  ⏭️  Skipping {} - exact match with my public IP:port ({})", node_id, my_id);
                        return false;
                    }
                }

                // Filter out localhost addresses
                if node.address == "127.0.0.1"
                    || node.address == "localhost"
                    || node.address == "::1"
                {
                    info!(
                        "  ⏭️  Skipping {}:{} - localhost address",
                        node.address, node.port
                    );
                    return false;
                }

                info!("  ✅ Candidate: {}:{} (height: {})", node.address, node.port, node.height);
                true
            })
            .collect();

        info!(
            "Selected {} candidates for latency testing",
            candidates.len()
        );

        let fallback_candidates: Vec<String> = candidates
            .iter()
            .map(|node| format!("{}:{}", node.address, node.port))
            .collect();

        // Measure latency for each candidate in parallel
        let mut scored_peers = Vec::new();

        info!("🔍 Starting latency measurements for {} candidates...", candidates.len());
        for node in candidates.into_iter() {
            let addr = format!("{}:{}", node.address, node.port);
            
            // Convert to localhost if it's the same public IP (for local node discovery)
            let test_addr = if let Some(ref my_public_ip) = my_address {
                if node.address == *my_public_ip {
                    let localhost_ip = get_localhost_address();
                    let localhost_addr = format!("{}:{}", localhost_ip, node.port);
                    info!("  Testing latency to {} (using {} for local connection)...", addr, localhost_addr);
                    localhost_addr
                } else {
                    info!("  Testing latency to {}...", addr);
                    addr.clone()
                }
            } else {
                info!("  Testing latency to {}...", addr);
                addr.clone()
            };
            
            let latency = measure_latency(&test_addr).await;

            if let Some(latency_ms) = latency {
                // Calculate composite score:
                // - 30% height (normalized)
                // - 20% uptime (capped at 168h)
                // - 50% network latency (lower is better)

                // For scoring, we need to normalize. We'll do final scoring after collecting all
                scored_peers.push(ScoredPeer {
                    address: addr.clone(),
                    height: node.height,
                    uptime_hours: node.uptime_hours,
                    latency_ms,
                    score: 0.0, // Will calculate after we have all data
                });

                info!(
                    "  ✅ {} - height: {}, uptime: {:.1}h, latency: {}ms",
                    addr,
                    node.height,
                    node.uptime_hours,
                    latency_ms
                );
            } else {
                warn!("  ❌ {} - unreachable (latency probe failed)", addr);
            }
        }

        info!("📊 Latency testing complete: {}/{} peers reachable", scored_peers.len(), fallback_candidates.len());

        if scored_peers.is_empty() {
            let fallback_count = fallback_candidates.len().min(limit);
            if fallback_count > 0 {
                warn!(
                    "⚠️  No peers passed latency probe; falling back to {} raw DNS candidates",
                    fallback_count
                );
                for (idx, addr) in fallback_candidates.iter().take(fallback_count).enumerate() {
                    info!("  Fallback {}: {}", idx + 1, addr);
                }
            } else {
                warn!("❌ No peers available (DNS candidates and latency probes both failed)");
            }
            return Ok(fallback_candidates.into_iter().take(limit).collect());
        }

        // Normalize and calculate final scores
        let max_height = scored_peers.iter().map(|p| p.height).max().unwrap_or(1) as f64;
        let min_latency = scored_peers.iter().map(|p| p.latency_ms).min().unwrap_or(1) as f64;
        let max_latency = scored_peers
            .iter()
            .map(|p| p.latency_ms)
            .max()
            .unwrap_or(1000) as f64;

        for peer in &mut scored_peers {
            let height_score = (peer.height as f64 / max_height.max(1.0)) * 0.3;
            let uptime_score = (peer.uptime_hours.min(168.0) / 168.0) * 0.2;

            // Latency score: lower latency = higher score
            let latency_normalized = if max_latency > min_latency {
                1.0 - ((peer.latency_ms as f64 - min_latency) / (max_latency - min_latency))
            } else {
                1.0
            };
            let latency_score = latency_normalized * 0.5;

            peer.score = height_score + uptime_score + latency_score;
        }

        // Sort by score (descending)
        scored_peers.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Log top peers
        info!("\n[INFO] Best peers by composite score:");
        for (i, peer) in scored_peers.iter().take(limit).enumerate() {
            info!(
                "  {}. {} - score: {:.3} (height: {}, uptime: {:.1}h, latency: {}ms)",
                i + 1,
                peer.address,
                peer.score,
                peer.height,
                peer.uptime_hours,
                peer.latency_ms
            );
        }

        let best_peers: Vec<String> = scored_peers
            .into_iter()
            .take(limit)
            .map(|p| p.address)
            .collect();

        Ok(best_peers)
    } else {
        let error_text = response.text().await?;
        Err(format!("Failed to fetch nodes from DNS server: {}", error_text).into())
    }
}

fn build_fallback_peer_targets(
    p2p_handle: &Arc<astram_node::p2p::manager::PeerManager>,
    settings: &NodeSettings,
    my_public_ip: Option<String>,
    my_port: u16,
    limit: usize,
) -> Vec<String> {
    let mut unique = HashSet::new();
    let mut targets = Vec::new();
    let my_public_id = my_public_ip.map(|ip| format!("{}:{}", ip, my_port));

    for raw in &settings.bootstrap_peers {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let normalized = if trimmed.contains(':') {
            trimmed.to_string()
        } else {
            format!("{}:{}", trimmed, my_port)
        };

        if my_public_id.as_deref() == Some(normalized.as_str()) {
            continue;
        }

        if unique.insert(normalized.clone()) {
            targets.push(normalized);
        }

        if targets.len() >= limit {
            return targets;
        }
    }

    for saved in p2p_handle.load_saved_peers() {
        let addr = saved.addr.trim().to_string();
        if addr.is_empty() {
            continue;
        }
        if my_public_id.as_deref() == Some(addr.as_str()) {
            continue;
        }

        if unique.insert(addr.clone()) {
            targets.push(addr);
        }

        if targets.len() >= limit {
            return targets;
        }
    }

    targets
}

/// Register this node with the DNS server (non-blocking version)
/// Height is optional and only used for informational purposes
/// Returns the public IP address as seen by the DNS server.
async fn register_with_dns(
    _node_handle: NodeHandle, // Not used - we don't need to lock for DNS registration
    settings: &NodeSettings,
    height: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let dns_url = settings.dns_server_url.clone();
    let node_port = settings.p2p_port;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let register_url = format!("{}/register", dns_url);

    let payload = serde_json::json!({
        "port": node_port,
        "version": env!("CARGO_PKG_VERSION"),
        "height": height
    });

    let response = client.post(&register_url).json(&payload).send().await?;

    if response.status().is_success() {
        #[derive(serde::Deserialize)]
        struct RegisterResponse {
            #[serde(rename = "success")]
            _success: bool,
            message: String,
            #[serde(rename = "node_count")]
            _node_count: usize,
            registered_address: String,
            registered_port: u16,
        }

        let result: RegisterResponse = response.json().await?;
        info!(
            "Successfully registered with DNS server: {} ({}:{})",
            result.message, result.registered_address, result.registered_port
        );
        Ok(result.registered_address)
    } else {
        let error_text = response.text().await?;
        Err(format!("Failed to register with DNS server: {}", error_text).into())
    }
}

/// Synchronize blockchain with peers.
///
/// Blocks until `my_height >= max available peer height`.
/// No overall timeout — the background `start_block_sync` task drives actual block
/// fetching and handles per-peer timeouts / blacklisting.  This function only monitors
/// progress and exits once the chain is caught up.
async fn sync_blockchain(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    const BAN_DURATION: std::time::Duration = std::time::Duration::from_secs(600);
    const POLL_INTERVAL: Duration = Duration::from_secs(2);
    const LOG_INTERVAL: u64 = 100; // log every N blocks

    let initial_height = {
        let bc = node_handle.bc.lock().unwrap();
        bc.chain_tip
            .as_ref()
            .and_then(|h| bc.load_header(h).ok().flatten())
            .map(|h| h.index)
            .unwrap_or(0)
    };

    // If no peers are connected yet, skip startup sync (background task will handle it).
    if p2p_handle.get_peer_heights().is_empty() {
        info!("[SYNC] No peers connected yet — skipping startup sync");
        return Ok(());
    }

    let target = p2p_handle
        .get_best_sync_peer(BAN_DURATION)
        .map(|(_, h)| h)
        .unwrap_or(0);

    if initial_height >= target {
        info!("[SYNC] Already up to date at height {}", initial_height);
        return Ok(());
    }

    info!(
        "[SYNC] Starting startup sync: {} → {} ({} blocks)",
        initial_height,
        target,
        target - initial_height
    );

    let mut last_logged = initial_height;

    loop {
        sleep(POLL_INTERVAL).await;

        let current = {
            let bc = node_handle.bc.lock().unwrap();
            bc.chain_tip
                .as_ref()
                .and_then(|h| bc.load_header(h).ok().flatten())
                .map(|h| h.index + 1)
                .unwrap_or(0)
        };

        // Refresh the target from best available (non-banned) peer each iteration
        // so we always aim at the current achievable maximum.
        let current_target = p2p_handle
            .get_best_sync_peer(BAN_DURATION)
            .map(|(_, h)| h)
            .unwrap_or(target);

        if current >= current_target {
            info!("[SYNC] ✅ Startup sync complete at height {}", current);
            break;
        }

        if current.saturating_sub(last_logged) >= LOG_INTERVAL || last_logged == initial_height {
            info!(
                "[SYNC] Startup sync: {}/{} ({:.1}%)",
                current,
                current_target,
                current as f64 / current_target.max(1) as f64 * 100.0
            );
            last_logged = current;
        }

        // If all peers are banned or disconnected, exit and let the background task handle it.
        if p2p_handle.get_connected_peer_count() == 0 {
            info!("[SYNC] No peers connected — handing off to background sync");
            break;
        }
        if p2p_handle.get_best_sync_peer(BAN_DURATION).is_none() {
            info!("[SYNC] All current peers are banned — handing off to background sync");
            break;
        }
    }

    Ok(())
}


async fn start_services(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
    chain_state: Arc<Mutex<ChainState>>,
    node_meta: Arc<NodeMeta>,
    shutdown_flag: Arc<AtomicBool>,
    settings: Arc<NodeSettings>,
) -> (
    Vec<tokio::task::JoinHandle<()>>,
    tokio::task::JoinHandle<()>,
) {

    let mut task_handles = Vec::new();

    let my_node_port = settings.p2p_port;

    // Register with DNS server initially with height 0 (or try to read non-blocking)
    // Don't hold lock for DNS registration - it can fail/timeout without affecting mining
    let initial_height = if let Ok(bc) = node_handle.bc.try_lock() {
        if let Some(tip_hash) = &bc.chain_tip {
            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                header.index
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0 // If can't read without blocking, use 0
    };

    // Register with DNS server (fail fast if registration fails)
    // Note: This is outside the main mining loop, so it happens only once at startup
    // Periodic re-registration is done without trying to acquire any locks
    match register_with_dns(node_handle.clone(), &settings, initial_height).await {
        Ok(registered_address) => {
            p2p_handle.set_my_public_ip(Some(registered_address.clone()));
            *node_meta.my_public_address.lock().unwrap() = Some(registered_address);
        }
        Err(e) => {
            log::error!("DNS registration failed; shutting down node: {}", e);
            std::process::exit(1);
        }
    }

    let dns_node_handle = node_handle.clone();
    let p2p_handle_dns = p2p_handle.clone();
    let node_meta_dns = node_meta.clone();
    let shutdown_flag_dns = shutdown_flag.clone();
    let settings_dns = settings.clone();
    let dns_task = tokio::spawn(async move {
        // Re-register every 5 minutes to keep the node alive in DNS
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await; // Skip first immediate tick

        info!("[DNS] Re-registration task started (interval: 300s)");

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if shutdown_flag_dns.load(OtherOrdering::SeqCst) {
                        info!("DNS registration task shutting down...");
                        break;
                    }

                    let tick_start = std::time::Instant::now();
                    info!("[DNS] ⏰ Re-registration tick START");

                    // Get current height for DNS (non-blocking attempt)
                    let height_opt = if let Ok(bc) = dns_node_handle.bc.try_lock() {
                        info!("[DNS] bc.try_lock() success");
                        if let Some(tip_hash) = &bc.chain_tip {
                            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                                Some(header.index)
                            } else {
                                None
                            }
                        } else {
                            Some(0)  // Empty blockchain
                        }
                    } else {
                        warn!("[DNS] bc.try_lock() failed - lock contended, skipping re-registration");
                        None  // Skip re-registration instead of using 0
                    };

                    // Skip re-registration if we couldn't determine height
                    let height = match height_opt {
                        Some(h) => h,
                        None => {
                            warn!("[DNS] Skipping re-registration tick (cannot determine height)");
                            continue;  // Skip this tick
                        }
                    };

                    info!("[DNS] Height determined: {} (took {:?})", height, tick_start.elapsed());

                    // Spawn DNS registration asynchronously - never blocks mining
                    let dns_handle_clone = dns_node_handle.clone();
                    let settings_clone = settings_dns.clone();
                    let p2p_handle_clone = p2p_handle_dns.clone();
                    let node_meta_clone = node_meta_dns.clone();
                    let spawn_time = std::time::Instant::now();
                    tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        info!("[DNS] Registration task spawned (spawn delay: {:?})", spawn_time.elapsed());
                        match tokio::time::timeout(
                            Duration::from_secs(2),
                            register_with_dns(dns_handle_clone.clone(), &settings_clone, height),
                        )
                        .await
                        {
                            Ok(Ok(registered_address)) => {
                                p2p_handle_clone.set_my_public_ip(Some(registered_address.clone()));
                                *node_meta_clone.my_public_address.lock().unwrap() = Some(registered_address);
                                info!(
                                    "[DNS] ✅ Re-registration OK (height={}, took {:?})",
                                    height,
                                    start.elapsed()
                                );
                            }
                            Ok(Err(e)) => {
                                warn!(
                                    "[DNS] ❌ Re-registration FAILED (height={}, took {:?}): {:?}",
                                    height,
                                    start.elapsed(),
                                    e
                                );
                            }
                            Err(e) => {
                                warn!(
                                    "[DNS] ⏱️  Re-registration TIMEOUT (height={}, took {:?}): {:?}",
                                    height,
                                    start.elapsed(),
                                    e
                                );
                            }
                        }
                    });

                    info!("[DNS] ⏰ Re-registration tick DONE (tick handling took {:?})", tick_start.elapsed());
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Check shutdown flag every second for quick response
                    if shutdown_flag_dns.load(OtherOrdering::SeqCst) {
                        info!("DNS registration task shutting down...");
                        break;
                    }
                }
            }
        }
    });
    task_handles.push(dns_task);

    // Connect to best nodes from DNS server
    let shutdown_flag_p2p = shutdown_flag.clone();
    let p2p_handle_for_task = p2p_handle.clone();
    let node_meta_for_p2p = node_meta.clone();
    let settings_p2p = settings.clone();
    let p2p_task = tokio::spawn(async move {
        // Wait a bit for DNS registration to complete
        sleep(Duration::from_secs(2)).await;

        // Initial connection to best nodes
        let mut initial_targets = match fetch_best_nodes_from_dns(
            node_meta_for_p2p.clone(),
            &settings_p2p,
            my_node_port,
            10,
        )
        .await
        {
            Ok(peer_addrs) => peer_addrs,
            Err(e) => {
                log::warn!("Failed to fetch best nodes from DNS: {}", e);
                Vec::new()
            }
        };

        if initial_targets.is_empty() {
            let my_public_ip = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
            let fallback = build_fallback_peer_targets(
                &p2p_handle_for_task,
                &settings_p2p,
                my_public_ip,
                my_node_port,
                10,
            );
            if !fallback.is_empty() {
                warn!(
                    "[P2P] DNS returned no peer targets, falling back to {} bootstrap/saved peers",
                    fallback.len()
                );
                initial_targets = fallback;
            }
        }

        info!(
            "[INFO] Connecting to {} best nodes from discovery",
            initial_targets.len()
        );
        
        let my_public_ip_for_connect = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
        
        for addr in initial_targets {
            let p2p_clone = p2p_handle_for_task.clone();
            let my_public_ip_clone = my_public_ip_for_connect.clone();
            
            // Convert to localhost if it's the same public IP (for local node discovery)
            let connection_addr = if let Some(ref my_ip) = my_public_ip_clone {
                if let Some((peer_ip, peer_port)) = addr.split_once(':') {
                    if peer_ip == my_ip {
                        let localhost_ip = get_localhost_address();
                        let localhost_addr = format!("{}:{}", localhost_ip, peer_port);
                        info!("[P2P] Converting {} to {} for local connection", addr, localhost_addr);
                        localhost_addr
                    } else {
                        addr.clone()
                    }
                } else {
                    addr.clone()
                }
            } else {
                addr.clone()
            };
            
            let original_addr = addr.clone();
            tokio::spawn(async move {
                if let Err(e) = p2p_clone.connect_peer(&connection_addr).await {
                    log::warn!("Failed to connect to peer {} ({}): {:?}", original_addr, connection_addr, e);
                } else {
                    info!("[OK] Connected to peer: {} (via {})", original_addr, connection_addr);
                }
            });
        }

        // Periodically refresh connections to best nodes (every 60 seconds)
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if shutdown_flag_p2p.load(OtherOrdering::SeqCst) {
                        info!("P2P connection refresh task shutting down...");
                        break;
                    }
                    match fetch_best_nodes_from_dns(
                        node_meta_for_p2p.clone(),
                        &settings_p2p,
                        my_node_port,
                        10,
                    )
                    .await
                    {
                        Ok(mut peer_addrs) => {
                            if peer_addrs.is_empty() {
                                let my_public_ip = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
                                let fallback = build_fallback_peer_targets(
                                    &p2p_handle_for_task,
                                    &settings_p2p,
                                    my_public_ip,
                                    my_node_port,
                                    10,
                                );
                                if !fallback.is_empty() {
                                    warn!(
                                        "[P2P] Refresh found no DNS targets, using {} fallback peers",
                                        fallback.len()
                                    );
                                    peer_addrs = fallback;
                                }
                            }

                            info!(
                                "[INFO] Refreshing connections to {} discovered peers",
                                peer_addrs.len()
                            );
                            
                            let my_public_ip_refresh = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
                            
                            for addr in peer_addrs {
                                let p2p_clone = p2p_handle_for_task.clone();
                                let my_public_ip_clone = my_public_ip_refresh.clone();
                                
                                // Convert to localhost if it's the same public IP
                                let connection_addr = if let Some(ref my_ip) = my_public_ip_clone {
                                    if let Some((peer_ip, peer_port)) = addr.split_once(':') {
                                        if peer_ip == my_ip {
                                            let localhost_ip = get_localhost_address();
                                            format!("{}:{}", localhost_ip, peer_port)
                                        } else {
                                            addr.clone()
                                        }
                                    } else {
                                        addr.clone()
                                    }
                                } else {
                                    addr.clone()
                                };
                                
                                tokio::spawn(async move {
                                    let _ = p2p_clone.connect_peer(&connection_addr).await;
                                });
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to refresh nodes from DNS: {}", e);
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Check shutdown flag every second for quick response
                    if shutdown_flag_p2p.load(OtherOrdering::SeqCst) {
                        info!("P2P connection refresh task shutting down...");
                        break;
                    }
                }
            }
        }
    });
    task_handles.push(p2p_task);

    // Wait for initial P2P connections to establish
    info!("[INFO] Waiting for P2P connections to establish...");
    
    // Wait longer for latency tests and connections to complete
    // (latency test: 3s × N peers + connection time)
    let mut wait_count = 0;
    while wait_count < 15 {  // Max 15 seconds
        sleep(Duration::from_secs(1)).await;
        wait_count += 1;
        
        // Check if we have any peers connected
        let peer_count = p2p_handle.get_peer_heights().len();
        if peer_count > 0 {
            info!("[INFO] {} peer(s) connected, proceeding...", peer_count);
            break;
        }
        
        if wait_count % 5 == 0 {
            info!("[INFO] Still waiting for peer connections... ({}/15s elapsed)", wait_count);
        }
    }

    // Step 5: Synchronize blockchain with peers
    info!("[INFO] Step 5: Synchronizing blockchain with peers...");
    if let Err(e) = sync_blockchain(node_handle.clone(), p2p_handle.clone()).await {
        log::warn!("Blockchain sync encountered error: {}", e);
    }

    let nh: NodeHandle = node_handle.clone();
    let http_addr = to_socket_addr(
        &settings.http_bind_addr,
        settings.http_port,
        SocketAddr::from(([127, 0, 0, 1], 19533)),
    );
    // start HTTP server in background thread (warp is async so run in tokio)
    let server_p2p = p2p_handle.clone();
    let server_chain = chain_state.clone();
    let server_meta = node_meta.clone();
    let server_handle = tokio::spawn(async move {
        run_server(nh, server_p2p, server_chain, server_meta, http_addr).await;
    });

    // Start public RPC server (restricted read-only endpoints, no wallet/mining/relay)
    if settings.public_rpc_port != 0 {
        let pub_addr = to_socket_addr(
            "0.0.0.0",
            settings.public_rpc_port,
            SocketAddr::from(([0, 0, 0, 0], 18533)),
        );
        let pub_nh    = node_handle.clone();
        let pub_p2p   = p2p_handle.clone();
        let pub_chain = chain_state.clone();
        let pub_meta  = node_meta.clone();
        task_handles.push(tokio::spawn(async move {
            run_public_server(pub_nh, pub_p2p, pub_chain, pub_meta, pub_addr).await;
        }));
    }

    // Return background tasks, but not server (we'll abort it)
    (task_handles, server_handle)
}
