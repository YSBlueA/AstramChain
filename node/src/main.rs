mod server;

use netcoin_core::Blockchain; // netcoin_core crate: 이전에 만든 모듈 경로에 맞춰 조정
use netcoin_core::block::{Block, BlockHeader, compute_merkle_root, compute_header_hash};
use netcoin_core::transaction::{Transaction};
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use server::run_server;
use std::fs;
use serde_json::Value;
use chrono::Utc;
use netcoin_node::NodeHandle; // lib.rs의 NodeHandle 사용
use netcoin_node::NodeState;
use netcoin_config::config::Config;
/// 노드 상태: core 블록체인 + in-memory chain view + pending tx queue

#[tokio::main]
async fn main() {
    println!("🚀 Netcoin node starting...");

    let cfg = Config::load();

    // DB path for core blockchain
    let db_path = cfg.data_dir.clone();

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
    let wallet_file = fs::read_to_string(cfg.wallet_path.clone())
        .expect("Failed to read wallet file");
    let wallet: Value = serde_json::from_str(&wallet_file)
        .expect("Failed to parse wallet JSON");
    let miner_address = wallet["address"].as_str()
        .expect("Failed to get address from wallet")
        .to_string();

    // If DB has no tip, create genesis block
    if bc.chain_tip.is_none() {
        println!("No chain tip found — creating genesis block...");
        let genesis_hash = bc.create_genesis(&miner_address).expect("create_genesis failed");
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
            let node = NodeState { bc, blockchain: vec![block], pending: vec![] };
            let node_handle = Arc::new(Mutex::new(node));
            start_services(node_handle, miner_address).await;
            return;
        }
    }

    // Otherwise, we have an existing chain tip. For simplicity, we won't reconstruct full chain here.
    // We'll create NodeState with empty in-memory chain but with bc loaded.
    let node = NodeState { bc, blockchain: vec![], pending: vec![] };
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
    // mining/miner loop: every 10s attempt to mine pending txs
    loop {
        {
            let mut state = node_handle.lock().unwrap();
            // gather pending txs copy
            let mut txs = vec![];
            // coinbase will be added by miner below
            txs.append(&mut state.pending);

            println!("⛏️  Mining {} pending tx(s)...", txs.len());

            // include coinbase as first tx
            let mut coinbase = Transaction::coinbase(&miner_address, current_block_reward(&state));
            coinbase = coinbase.with_txid(); // ensure txid set
            let mut block_txs = vec![coinbase.clone()];
            block_txs.append(&mut txs);

            // build header template
            let prev_hash = state.bc.chain_tip.clone().unwrap_or_else(|| "0".repeat(64));
            let merkle = compute_merkle_root(&block_txs.iter().map(|t| t.txid.clone()).collect::<Vec<_>>());
            let mut nonce: u64 = 0;
            let difficulty = state.bc.difficulty; // difficulty bits is stored in core
            let mut found = None;

            // simple PoW: find header hash with leading zeros = difficulty
            loop {
                let header = BlockHeader {
                    index: 0, // core will set index based on chain state; but core expects proper index when inserting.
                    previous_hash: prev_hash.clone(),
                    merkle_root: merkle.clone(),
                    timestamp: Utc::now().timestamp(),
                    nonce,
                    difficulty: difficulty,
                    state_root: None,
                };


                match compute_header_hash(&header) {
                    Ok(hash) => {
                        if hash.starts_with(&"0".repeat(difficulty as usize)) {
                            found = Some((header, hash));
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Error computing header hash: {}", e);
                        break;
                    }
                }
                nonce = nonce.wrapping_add(1);
                // avoid tight CPU loop in this example: break after many iterations so program remains responsive
                // In production, miner should keep running; here we allow a limited search per cycle.
                if nonce % 1_000_000 == 0 {
                    // yield a bit
                    // (no-op) – keep going; you can add a small sleep if needed
                    // avoid CPU overload
                    tokio::task::yield_now().await;
                }
            }

            if let Some((mut header, mut hash)) = found {
                // set proper index value based on chain tip
                // We'll try to get previous index from DB via tip
                if let Some(tip_hash) = state.bc.chain_tip.clone() {
                    // try to load previous header to obtain index
                    if let Ok(Some(prev_header)) = state.bc.load_header(&tip_hash) {
                        header.index = prev_header.index + 1;
                    } else {
                        header.index = 0;
                    }
                } else {
                    header.index = 0;
                }

                // ✅ recompute hash with final header (after index set)
                hash = compute_header_hash(&header).unwrap();

                // build final block
                let block = Block {
                    header: header.clone(),
                    transactions: block_txs.clone(),
                    hash: hash.clone(),
                };

                // attempt to insert via core's validate_and_insert_block (it will validate merkle etc.)
                match state.bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        println!("✅ Mined new block index={} hash={}", block.header.index, block.hash);
                        // update chain_tip in state.bc already done by core method
                        // update in-memory chain view
                        state.blockchain.push(block);
                        // clear pending (we consumed them earlier)
                        state.pending.clear();
                        // persist tip / DB already persisted by core
                    }
                    Err(e) => {
                        eprintln!("Block insertion failed: {}", e);
                        // if insertion failed (e.g., prev not found) push txs back to pending
                        // requeue non-coinbase transactions
                        // For safety, we re-add txs[1..] to pending
                        for tx in block_txs.into_iter().skip(1) {
                            state.pending.push(tx);
                        }
                    }
                }
            } else {
                println!("⛏️  No block found this cycle (nonce loop ended).");
            }
        } // release lock

        sleep(Duration::from_secs(10)).await;
    }

    server_handle.await.unwrap();
}

/// simple block reward halving logic: adjust as needed
fn current_block_reward(state: &NodeState) -> u64 {
    // naive: fixed 50 in this example or halving per 210000 as needed
    50
}
