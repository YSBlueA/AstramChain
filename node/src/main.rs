mod p2p;
mod server;

use crate::p2p::manager::MAX_OUTBOUND;
use chrono::Utc;
use log::info;
use netcoin_config::config::Config;
use netcoin_core::Blockchain;
use netcoin_core::block;
use netcoin_core::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use netcoin_core::consensus;
use netcoin_core::transaction::Transaction;
use netcoin_node::NodeHandle;
use netcoin_node::NodeState;
use netcoin_node::p2p::manager::PeerManager;
use serde_json::Value;
use server::run_server;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as OtherOrdering;
use std::sync::{Arc, Mutex};
use tokio::signal;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() {
    println!("🚀 Netcoin node starting...");

    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .init();

    let cfg = Config::load();

    // DB path for core blockchain
    let db_path = cfg.data_dir.clone();

    // Initialize P2P networking
    let p2p = Arc::new(PeerManager::new());

    /*
    {
        let p2p_clone = p2p.clone();
        p2p_clone.set_on_block(|block: block::Block| {
            tokio::spawn(async move {
                match netcoin_core::consensus::validate_and_add_block(block).await {
                    Ok(_) => info!("Block added via p2p"),
                    Err(e) => log::warn!("Received invalid block from p2p: {:?}", e),
                }
            });
        });
    }
    */

    let p2p_clone = p2p.clone();
    tokio::spawn(async move {
        if let Err(e) = p2p_clone.start_listener("0.0.0.0:8335").await {
            log::error!("P2P listener failed: {:?}", e);
        }
    });

    let p2p_clone = p2p.clone();
    let dns_list = p2p_clone.dns_seed_lookup().await.unwrap_or_default();

    let saved_list = p2p_clone.load_saved_peers();
    let mut peers: HashSet<String> = HashSet::new();

    for addr in dns_list {
        peers.insert(addr);
    }

    for sp in saved_list {
        peers.insert(sp.addr);
    }

    let target_peers: Vec<String> = peers.into_iter().take(MAX_OUTBOUND).collect();

    for addr in target_peers {
        let p2p_clone = p2p.clone();
        tokio::spawn(async move {
            if let Err(e) = p2p_clone.connect_peer(&addr).await {
                log::warn!("Failed connect {}: {:?}", addr, e);
            }
        });
    }

    print!("Initialize Block chain...\n");

    // Initialize core Blockchain (RocksDB-backed)
    let mut bc = match Blockchain::new(db_path.as_str()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to open blockchain DB: {}", e);
            // try to create empty instance (this depends on core API)
            std::process::exit(1);
        }
    };

    // If chain is empty (no tip), create genesis from wallet address
    // Read wallet address from file
    let wallet_file =
        fs::read_to_string(cfg.wallet_path.clone()).expect("Failed to read wallet file");
    let wallet: Value = serde_json::from_str(&wallet_file).expect("Failed to parse wallet JSON");
    let miner_address = wallet["address"]
        .as_str()
        .expect("Failed to get address from wallet")
        .to_string();

    // If DB has no tip, create genesis block
    if bc.chain_tip.is_none() {
        println!("No chain tip found — creating genesis block...");
        let genesis_hash = bc
            .create_genesis(&miner_address)
            .expect("create_genesis failed");
        // load genesis header & tx to in-memory chain view
        if let Ok(Some(header)) = bc.load_header(&genesis_hash) {
            // need block transactions loaded too -> load txs by scanning index i:0
            // Simplify: construct block from header and coinbase tx from DB
            // Try to load coinbase tx via stored tx key (i:0 -> hash -> t:<txid>)
            // For simplicity, we will append a minimal block header-only view.
            let block = Block {
                header,
                transactions: vec![], // empty details (can be expanded)
                hash: genesis_hash.clone(),
            };
            // Build NodeState with this genesis header
            let node = NodeState {
                bc,
                blockchain: vec![block],
                pending: vec![],
                seen_tx: HashSet::new(),
                p2p: p2p.clone(),
            };
            let node_handle = Arc::new(Mutex::new(node));
            start_services(node_handle, miner_address).await;
            return;
        }
    }

    // Otherwise, we have an existing chain tip. For simplicity, we won't reconstruct full chain here.
    // We'll create NodeState with empty in-memory chain but with bc loaded.
    let node = NodeState {
        bc,
        blockchain: vec![],
        pending: vec![],
        seen_tx: HashSet::new(),
        p2p: p2p.clone(),
    };
    let node_handle = Arc::new(Mutex::new(node));

    start_services(node_handle.clone(), miner_address).await;
}

async fn start_services(node_handle: NodeHandle, miner_address: String) {
    println!("🚀 my address {}", miner_address);

    let nh: Arc<Mutex<NodeState>> = node_handle.clone();
    // start HTTP server in background thread (warp is async so run in tokio)
    let server_handle = {
        tokio::spawn(async move {
            run_server(nh).await;
        })
    };

    println!("🚀 mining starting...");
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_net = cancel_flag.clone();

    // mining/miner loop: every 10s attempt to mine pending txs
    loop {
        cancel_flag.store(false, OtherOrdering::SeqCst);

        // Snapshot pending txs + mining params while holding the lock briefly
        let (snapshot_txs, difficulty, prev_hash, index_snapshot, p2p_handle) = {
            let mut state = node_handle.lock().unwrap();

            // clone pending transactions to work on them outside the lock
            let txs_copy = state.pending.clone();

            // previous tip hash
            let prev_hash = state.bc.chain_tip.clone().unwrap_or_else(|| "0".repeat(64));
            let diff = state.bc.difficulty;

            // determine next index from tip header (so header.index is known before mining)
            let mut next_index: u64 = 0;
            if let Some(tip_hash) = state.bc.chain_tip.clone() {
                if let Ok(Some(prev_header)) = state.bc.load_header(&tip_hash) {
                    next_index = prev_header.index + 1;
                } else {
                    next_index = 0;
                }
            } else {
                next_index = 0;
            }

            // clear pending locally — we'll requeue on failure
            state.pending.clear();

            (txs_copy, diff, prev_hash, next_index, state.p2p.clone())
        };

        // prepare block transactions: coinbase + pending
        // NOTE: we pass pending txs to consensus::mine_block_with_coinbase which will prepend coinbase
        let block_txs_for_logging = snapshot_txs.len();
        println!("⛏️ Mining {} pending tx(s)...", block_txs_for_logging);

        // prepare parameters for blocking mining call
        let prev_hash = prev_hash.clone();
        let difficulty_local = difficulty;
        let index_local = index_snapshot;
        let miner_addr_cloned = miner_address.clone();
        let txs_cloned = snapshot_txs.clone();
        let coinbase_reward = current_block_reward_snapshot();
        let cancel_for_thread = cancel_flag.clone();

        // Run CPU-bound mining in a blocking task so we don't block the tokio runtime
        let mined_block_res: anyhow::Result<Block> = tokio::task::spawn_blocking(move || {
            // call into core/consensus
            consensus::mine_block_with_coinbase(
                index_local,
                prev_hash,
                difficulty_local,
                txs_cloned,
                &miner_addr_cloned,
                coinbase_reward,
                cancel_for_thread,
            )
        })
        .await
        .expect("mining task panicked");

        match mined_block_res {
            Ok(mut block) => {
                // Re-acquire lock to insert block atomically and to handle concurrent tip changes
                let mut state = node_handle.lock().unwrap();

                // As a safety, recompute index based on current tip in case chain advanced
                if let Some(tip_hash) = state.bc.chain_tip.clone() {
                    if let Ok(Some(prev_header)) = state.bc.load_header(&tip_hash) {
                        block.header.index = prev_header.index + 1;
                    } else {
                        block.header.index = 0;
                    }
                } else {
                    block.header.index = 0;
                }

                // update timestamp and recompute hash (index/timestamp changed)
                block.header.timestamp = Utc::now().timestamp();
                block.hash =
                    compute_header_hash(&block.header).expect("recompute header hash failed");

                match state.bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        println!(
                            "✅ Mined new block index={} hash={}",
                            block.header.index, block.hash
                        );
                        let block_to_broadcast = block.clone();

                        state.blockchain.push(block);
                        // pending already cleared earlier
                        println!("✅ Block mined! Broadcasting...");

                        // -------------------------
                        // Broadcast mined block
                        // -------------------------
                        // broadcast_block returns () (fire-and-forget), so just await it
                        p2p_handle.broadcast_block(&block_to_broadcast).await;
                    }
                    Err(e) => {
                        eprintln!("Block insertion failed: {}", e);
                        // requeue non-coinbase txs back to pending
                        for tx in block.transactions.into_iter().skip(1) {
                            state.pending.push(tx);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("⛏️ Mining error: {}", e);
                // requeue pending txs
                let mut state = node_handle.lock().unwrap();
                for tx in snapshot_txs.into_iter() {
                    state.pending.push(tx);
                }
            }
        }

        // wait a bit before next cycle
        sleep(Duration::from_secs(10)).await;
    }

    // server_handle.await.unwrap(); // unreachable because loop is infinite
}

fn current_block_reward_snapshot() -> u64 {
    // keep simple for now
    50
}
