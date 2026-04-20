// node/src/p2p/service.rs
use crate::ChainState;
use crate::NodeHandle;
use crate::p2p::manager::{MAX_OUTBOUND, PeerManager};
use hex;
use log::{debug, info, warn};
use Astram_core::block;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

pub struct P2PService {
    pub manager: Arc<PeerManager>,
}

impl P2PService {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(PeerManager::new()),
        }
    }

    pub fn manager(&self) -> Arc<PeerManager> {
        self.manager.clone()
    }

    pub async fn start(
        &self,
        bind_addr: String,
        node_handle: NodeHandle,
        chain_state: Arc<std::sync::Mutex<ChainState>>,
    ) -> anyhow::Result<()> {
        self.start_listener(bind_addr).await;
        self.connect_initial_peers().await;
        self.register_handlers(node_handle.clone(), chain_state.clone());
        self.start_block_sync(node_handle.clone());
        self.start_peer_maintenance();

        Ok(())
    }

    async fn start_listener(&self, addr: String) {
        let p2p = self.manager.clone();

        tokio::spawn(async move {
            if let Err(e) = p2p.start_listener(&addr).await {
                log::error!("❌ CRITICAL: P2P listener failed: {:?}", e);
                log::error!("   Address: {}", addr);
                log::error!("   This node CANNOT accept incoming peer connections!");
                log::error!("   Check if port is already in use or firewall is blocking");
                std::process::exit(1);
            }
        });

        // Give listener a moment to bind
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    async fn connect_initial_peers(&self) {
        let p2p = self.manager.clone();

        let dns_list = p2p.dns_seed_lookup().await.unwrap_or_default();
        let saved_list = p2p.load_saved_peers();

        let mut peers = HashSet::new();
        for addr in dns_list {
            peers.insert(addr);
        }
        for sp in saved_list {
            peers.insert(sp.addr);
        }

        for addr in peers.into_iter().take(MAX_OUTBOUND) {
            let p2p_clone = p2p.clone();
            tokio::spawn(async move {
                if let Err(e) = p2p_clone.connect_peer(&addr).await {
                    warn!("Failed connect {}: {:?}", addr, e);
                }
            });
        }
    }

    fn register_handlers(
        &self,
        node_handle: NodeHandle,
        chain_state: Arc<std::sync::Mutex<ChainState>>,
    ) {
        let p2p = self.manager.clone();

        // Create unbounded channel for sequential block processing (prevents deadlock)
        let (block_tx, mut block_rx) = tokio::sync::mpsc::unbounded_channel::<block::Block>();

        // Check if a block hash already exists in our DB (used by Headers handler to skip duplicates)
        let nh_exists = node_handle.clone();
        p2p.set_check_block_exists(move |hash_hex: &str| {
            if let Ok(bc) = nh_exists.bc.try_lock() {
                let key = format!("b:{}", hash_hex);
                bc.db.get(key.as_bytes()).ok().flatten().is_some()
            } else {
                false // If lock contended, assume block doesn't exist (safe to re-request)
            }
        });

        // set chain locator callback for syncing (use persisted DB tip, not in-memory cache)
        let nh_locator = node_handle.clone();
        p2p.set_on_get_chain_locator(move || {
            let mut locator = Vec::new();
            let bc = nh_locator.bc.lock().unwrap();

            let mut current_hash = bc.chain_tip.clone();
            while let Some(hash) = current_hash {
                if let Ok(bytes) = hex::decode(&hash) {
                    locator.push(bytes);
                }

                if locator.len() >= 10 {
                    break;
                }

                match bc.load_header(&hash) {
                    Ok(Some(header)) if header.index > 0 => {
                        current_hash = Some(header.previous_hash.clone());
                    }
                    _ => break,
                }
            }

            locator
        });

        // headers handler - detect chain reorg from genesis
        let nh_headers = node_handle.clone();
        p2p.set_on_headers(move |peer_id, headers| {
            if headers.is_empty() {
                return false;
            }

            // Check if this is a genesis-starting chain that differs from ours
            let first_header = &headers[0];
            if first_header.index == 0 {
                let bc = nh_headers.bc.lock().unwrap();

                // Check our genesis
                if let Ok(Some(our_genesis_hash)) = bc.db.get(b"i:0") {
                    let our_genesis_hash_str = String::from_utf8_lossy(&our_genesis_hash);
                    if let Ok(received_genesis_hash) = Astram_core::block::compute_header_hash(first_header) {
                        if our_genesis_hash_str != received_genesis_hash {
                            let our_height = bc.chain_tip.as_ref()
                                .and_then(|tip| bc.load_header(tip).ok().flatten())
                                .map(|h| h.index)
                                .unwrap_or(0);

                            let their_height = headers.last().map(|h| h.index).unwrap_or(0);

                            log::warn!(
                                "🔄 DIFFERENT GENESIS DETECTED from peer {}! Our height: {}, Their height: {}",
                                peer_id,
                                our_height, their_height
                            );
                            log::warn!("   Our genesis:   {}", our_genesis_hash_str);
                            log::warn!("   Their genesis: {}", received_genesis_hash);
                            log::warn!(
                                "❌ Rejecting peer {} due to incompatible genesis (keeping local chain)",
                                peer_id
                            );
                            // Return false so manager drops this peer.
                            return false;
                        }
                    }
                }
            }

            true
        });

        // getheaders handler - load headers from DB
        //
        // PERFORMANCE: Use index-based forward scan (O(locator + 200) DB reads) instead of
        // walking the entire chain backwards (O(N) reads). The old approach loaded every block
        // while holding bc lock, freezing P2P/RPC as the chain grew.
        let nh = node_handle.clone();
        p2p.set_on_getheaders(move |locator_hashes, _stop_hash| {
            let bc = nh.bc.lock().unwrap();

            // Step 1: determine the start index from the locator hashes.
            // Each locator hash is loaded individually (fast O(1) DB lookup per entry).
            let start_index: u64 = if locator_hashes.is_empty() {
                0
            } else {
                let mut found = 0u64;
                for loc_bytes in &locator_hashes {
                    let hash_hex = hex::encode(loc_bytes);
                    // load_header reads "b:{hash}" - one DB read, no chain walk
                    if let Ok(Some(header)) = bc.load_header(&hash_hex) {
                        found = header.index + 1;
                        break;
                    }
                }
                found
            };

            // Step 2: forward scan using index keys "i:{n}" → hash → header.
            // At most 200 reads regardless of chain length.
            let mut headers = Vec::new();
            let mut index = start_index;
            while headers.len() < 200 {
                let key = format!("i:{}", index);
                match bc.db.get(key.as_bytes()) {
                    Ok(Some(hash_bytes)) => {
                        if let Ok(hash) = String::from_utf8(hash_bytes.to_vec()) {
                            if let Ok(Some(header)) = bc.load_header(&hash) {
                                headers.push(header);
                            }
                        }
                        index += 1;
                    }
                    _ => break,
                }
            }

            debug!(
                "[P2P] GetHeaders: returning {} headers starting at index {}",
                headers.len(),
                start_index
            );
            headers
        });

        // Sequential block processing task (prevents deadlock from concurrent lock acquisition)
        let nh_processor = node_handle.clone();
        let chain_processor = chain_state.clone();
        let p2p_processor = p2p.clone();
        tokio::spawn(async move {
            info!("[P2P] 🔄 Sequential block processor task started");
            while let Some(block) = block_rx.recv().await {
                debug!("[P2P] 📦 Processing block #{} {} from queue", block.header.index, &block.hash[..16]);
                let handler_start = std::time::Instant::now();
                
                let state = nh_processor.clone();
                let chain_async = chain_processor.clone();
                let p2p_block = p2p_processor.clone();

                // Check if this is a block we recently mined ourselves
                {
                    let chain = chain_async.lock().unwrap();
                    
                    if chain.recently_mined_blocks.contains_key(&block.hash) {
                        info!(
                            "[INFO] Ignoring block we mined ourselves: index={} hash={}",
                            block.header.index, block.hash
                        );
                        continue;  // Skip to next block in queue
                    }
                }

                // Cancel ongoing mining only if the incoming block is at or beyond the current tip
                // (blocks below current tip cannot affect the active mining task)
                if state.mining.active.load(std::sync::atomic::Ordering::SeqCst) {
                    let current_tip_height = {
                        if let Ok(bc) = state.bc.try_lock() {
                            if let Some(tip_hash) = &bc.chain_tip {
                                bc.load_header(tip_hash).ok().flatten().map(|h| h.index)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    let should_cancel = match current_tip_height {
                        Some(tip_h) => block.header.index >= tip_h,
                        None => true, // can't determine, cancel to be safe
                    };
                    if should_cancel {
                        state
                            .mining
                            .cancel_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    } else {
                        debug!(
                            "[P2P] Block #{} below current tip #{}, skipping mining cancel",
                            block.header.index,
                            current_tip_height.unwrap_or(0)
                        );
                    }
                }

                // Try to insert the block
                let lock_start = std::time::Instant::now();
                debug!("[LOCK-DEBUG] 🔒 Block #{} attempting bc.lock()...", block.header.index);
                let mut bc = state.bc.lock().unwrap();
                debug!("[LOCK-DEBUG] ✅ Block #{} acquired bc.lock() after {:?}", block.header.index, lock_start.elapsed());

                // Early duplicate check: skip only if the block is in the DB AND has a valid
                // index entry on the main chain. A block may exist as a fork block (b: written
                // but i: missing); in that case we must retry insertion so the index gets set.
                let block_key = format!("b:{}", block.hash);
                let index_key = format!("i:{}", block.header.index);
                let already_indexed = bc.db.get(block_key.as_bytes()).ok().flatten().is_some()
                    && bc.db.get(index_key.as_bytes()).ok().flatten()
                        .map(|v| v.as_slice() == block.hash.as_bytes())
                        .unwrap_or(false);
                if already_indexed {
                    debug!("[P2P] Block #{} ({}) already in DB with index, skipping", block.header.index, &block.hash[..16]);
                    drop(bc);
                    continue;
                }

                let validation_start = std::time::Instant::now();
                let syncing = p2p_block.get_syncing();
                match bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        if syncing {
                            debug!(
                                "[OK] Block #{} added (validation: {:?})",
                                block.header.index, validation_start.elapsed()
                            );
                        } else {
                            info!(
                                "[OK] Block #{} added (validation: {:?})",
                                block.header.index, validation_start.elapsed()
                            );
                        }
                        
                        // Release bc lock before taking chain lock
                        let lock_drop_time = std::time::Instant::now();
                        debug!("[LOCK-DEBUG] ⏳ Block #{} releasing bc.lock()...", block.header.index);
                        drop(bc);
                        debug!("[LOCK-DEBUG] ✅ Block #{} released bc.lock() after {:?}", block.header.index, lock_drop_time.elapsed());
                        
                        {
                            let mut chain = chain_async.lock().unwrap();
                            chain.blockchain.push(block.clone());
                            chain.enforce_memory_limit(); // Security: Enforce memory limit
                        }

                        // Update P2P manager height
                        p2p_block.set_my_height(block.header.index);

                        // Remove transactions from pending pool that are in the new block
                        let block_txids: std::collections::HashSet<String> = block
                            .transactions
                            .iter()
                            .map(|tx| tx.txid.clone())
                            .collect();

                        let removed_count = block_txids.len().saturating_sub(1); // -1 for coinbase
                        {
                            let mut mempool = state.mempool.lock().unwrap();
                            mempool.pending.retain(|tx| !block_txids.contains(&tx.txid));
                        }

                        if removed_count > 0 {
                            info!(
                                "[INFO] Removed {} transactions from mempool",
                                removed_count
                            );
                        }

                        // Reacquire bc lock for reorganization check
                        let lock_reacq_time = std::time::Instant::now();
                        debug!("[LOCK-DEBUG] 🔒 Block #{} attempting bc.lock() for reorg check...", block.header.index);
                        let mut bc = state.bc.lock().unwrap();
                        debug!("[LOCK-DEBUG] ✅ Block #{} acquired bc.lock() for reorg after {:?}", block.header.index, lock_reacq_time.elapsed());
                        
                        // Check if this block triggers a chain reorganization
                        match bc.reorganize_if_needed(&block.hash) {
                            Ok(true) => {
                                info!("[OK] Chain reorganization completed");
                            }
                            Ok(false) => {
                                // No reorg needed, current chain is best
                            }
                            Err(e) => {
                                warn!("[WARN] Reorganization check failed: {:?}", e);
                            }
                        }

                        // Try to process orphan blocks that may now be valid
                        {
                            let mut chain = chain_async.lock().unwrap();
                            Self::process_orphan_blocks(
                                &mut bc,
                                &mut chain,
                                &state.mempool,
                                p2p_block.clone(),
                            );
                        }

                        if syncing {
                            debug!("[P2P] Block #{} synced (total time {:?})", block.header.index, handler_start.elapsed());
                        } else {
                            info!("[P2P] ✅ Block handler COMPLETED for block #{} (total time {:?})", block.header.index, handler_start.elapsed());
                        }
                        if state.mining.active.load(std::sync::atomic::Ordering::SeqCst) {
                            info!("[INFO] Mining cancelled, restarting with updated chain...");
                        }
                    }
                    Err(e) => {
                        // Block validation failed - check if it's an orphan or fork
                        let error_msg = format!("{:?}", e);
                        
                        if error_msg.contains("previous header not found") || error_msg.contains("fork detected") {
                            debug!("[P2P] Block #{} is orphan/fork: {}", block.header.index, error_msg);
                            
                            // For fork blocks, try to store and trigger reorganization
                            if error_msg.contains("fork detected") {
                                // This is a fork block - parent exists but not on our chain tip
                                // Store it separately and check if it creates a better chain
                                debug!("[P2P] 🔀 Fork block detected at height {}, attempting chain reorganization...", block.header.index);
                                
                                // Validate the fork block without chain tip check
                                match bc.validate_fork_block(&block) {
                                    Ok(_) => {
                                        debug!("[P2P] ✅ Fork block validated, checking if reorg needed...");
                                        
                                        // Try to reorganize to this fork
                                        match bc.reorganize_if_needed(&block.hash) {
                                            Ok(true) => {
                                                info!("[P2P] ✅ Chain reorganized to fork block #{}", block.header.index);
                                                
                                                // Update chain state
                                                drop(bc);
                                                let mut chain = chain_async.lock().unwrap();
                                                chain.blockchain.push(block.clone());
                                                chain.enforce_memory_limit();
                                                
                                                p2p_block.set_my_height(block.header.index);
                                                info!("[INFO] Mining cancelled, restarted with new chain after reorg");
                                            }
                                            Ok(false) => {
                                                debug!("[P2P] Fork block exists but our chain has more work, keeping current chain");
                                                drop(bc);  // Release lock before continue
                                            }
                                            Err(reorg_err) => {
                                                warn!("[WARN] Reorganization failed: {:?}", reorg_err);
                                                drop(bc);  // Release lock before continue
                                            }
                                        }
                                    }
                                    Err(val_err) => {
                                        warn!("[WARN] Fork block validation failed: {:?}", val_err);
                                        drop(bc);  // Release lock before continue
                                    }
                                }
                                
                                continue;
                            }
                            
                            // Regular orphan handling
                            // Use the currently-held bc lock to avoid self-deadlock on re-lock.
                            let current_height = if let Some(tip_hash) = &bc.chain_tip {
                                if let Ok(Some(header)) = bc.load_header(tip_hash) {
                                    header.index + 1
                                } else {
                                    0
                                }
                            } else {
                                0
                            };

                            let orphan_release_start = std::time::Instant::now();
                            debug!(
                                "[LOCK-DEBUG] ⏳ Block #{} releasing bc.lock() before orphan handling...",
                                block.header.index
                            );
                            drop(bc);
                            debug!(
                                "[LOCK-DEBUG] ✅ Block #{} released bc.lock() before orphan handling after {:?}",
                                block.header.index,
                                orphan_release_start.elapsed()
                            );

                            // 동기화 중: 현재 높이 + 1000 이상 차이나는 블록은 무시
                            if block.header.index > current_height + 1000 {
                                warn!(
                                    "[SYNC] ⚠️ Block #{} too far ahead (current: {}), ignoring to save memory during sync",
                                    block.header.index, current_height
                                );
                                continue;
                            }

                            // Security: Check orphan pool size limit before adding
                            let now = chrono::Utc::now().timestamp();
                            
                            let mut chain = chain_async.lock().unwrap();
                            
                            // 동기화 중: 높이가 먼 orphan부터 먼저 정리
                            let max_orphan_height_gap = 1000;
                            chain.orphan_blocks.retain(|_, (orphan_block, _)| {
                                // 현재 높이보다 너무 높은 orphan은 버림
                                orphan_block.header.index <= current_height + max_orphan_height_gap
                            });
                            
                            if chain.orphan_blocks.len() >= crate::MAX_ORPHAN_BLOCKS {
                                warn!(
                                    "[WARN] Orphan pool full ({} blocks), removing highest-indexed blocks",
                                    chain.orphan_blocks.len()
                                );
                                
                                // Sort and remove orphans with highest index (closest to target)
                                let mut orphan_vec: Vec<_> = chain.orphan_blocks
                                    .iter()
                                    .map(|(h, (block, ts))| (h.clone(), block.header.index, *ts))
                                    .collect();
                                
                                // 동기화 중이므로 가장 가까운 높이부터 제거 (top-down)
                                orphan_vec.sort_by(|a, b| b.1.cmp(&a.1)); // 높이 역순
                                
                                // 상위 25%만 제거해서 즉시 가득 찬 상태 해결
                                let remove_count = (crate::MAX_ORPHAN_BLOCKS / 4).max(1);
                                for (hash, _, _) in orphan_vec.iter().take(remove_count) {
                                    chain.orphan_blocks.remove(hash);
                                }
                            }
                            
                            // Clean up expired orphans (older than 30 minutes)
                            chain.orphan_blocks.retain(|_, (_, timestamp)| {
                                now - *timestamp < crate::ORPHAN_TIMEOUT
                            });
                            
                            chain.orphan_blocks.insert(block.hash.clone(), (block.clone(), now));
                            
                            info!(
                                "[SYNC] 📦 Orphan block STORED: index={}, hash={}, parent={}, pool_size={}",
                                block.header.index,
                                &block.hash[..16],
                                &block.header.previous_hash[..16],
                                chain.orphan_blocks.len()
                            );
                            
                            // 3. 역방향 탐색: parent block 요청
                            let parent_hash = block.header.previous_hash.clone();
                            info!("[SYNC] 📥 Requesting parent block {} for orphan #{}", 
                                  &parent_hash[..16], block.header.index);
                            
                            // Drop chain lock before reacquiring bc lock for process_orphan_blocks
                            // This prevents potential deadlock
                            drop(chain);
                            
                            // Now try to process any orphan blocks that may now be valid
                            {
                                let lock_orphan_time = std::time::Instant::now();
                                debug!("[LOCK-DEBUG] 🔒 Block #{} attempting bc.lock() for orphan processing...", block.header.index);
                                let mut bc_for_orphan = state.bc.lock().unwrap();
                                debug!("[LOCK-DEBUG] ✅ Block #{} acquired bc.lock() for orphan after {:?}", block.header.index, lock_orphan_time.elapsed());
                                
                                let mut chain_for_orphan = chain_async.lock().unwrap();
                                Self::process_orphan_blocks(
                                    &mut bc_for_orphan,
                                    &mut chain_for_orphan,
                                    &state.mempool,
                                    p2p_block.clone(),
                                );
                            }
                            
                            // Request parent block
                            p2p_block.request_block_by_hash(&parent_hash);
                            
                        } else {
                            debug!("[DEBUG] Invalid block from p2p: {:?}", e);
                        }
                    }
                }
                
                debug!("[P2P] ✅ Block #{} processed in {:?}", block.header.index, handler_start.elapsed());
            }
            warn!("[P2P] Sequential block processor task ended");
        });

        // Periodic seen_tx cleanup — every 10 min, evict entries older than 1 hour.
        // Moved out of per-TX hot path to avoid O(N) retain on every transaction.
        let nh_cleanup = node_handle.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(600));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().timestamp();
                let mut mempool = nh_cleanup.mempool.lock().unwrap();
                let before = mempool.seen_tx.len();
                mempool.seen_tx.retain(|_, &mut ts| now - ts < 3600);
                let removed = before - mempool.seen_tx.len();
                if removed > 0 {
                    log::debug!("[INFO] Cleaned up {} stale seen_tx entries", removed);
                }
            }
        });

        // Block handler - just enqueue blocks for sequential processing
        p2p.set_on_block(move |block: block::Block| {
            if let Err(e) = block_tx.send(block) {
                warn!("[P2P] Failed to enqueue block for processing: {:?}", e);
            }
        });

        // transaction handler
        let nh3 = node_handle.clone();
        let p2p_for_tx = p2p.clone();
        p2p.set_on_tx(move |tx: Astram_core::transaction::Transaction| {
            info!("[P2P] 💸 TX handler START for tx {}", hex::encode(&tx.txid[..8]));
            let handler_start = std::time::Instant::now();
            
            let nh_async = nh3.clone();
            let p2p_tx_relay = p2p_for_tx.clone();
            tokio::spawn(async move {
                // Check and update state in a separate scope to ensure lock is released
                let should_relay = {
                    let state = nh_async;

                    {
                        info!("[P2P] 🔒 TX handler: acquiring mempool lock for seen_tx check...");
                        let lock_start = std::time::Instant::now();
                        let mempool = state.mempool.lock().unwrap();
                        info!("[P2P] ✅ TX handler: mempool lock acquired (took {:?})", lock_start.elapsed());

                        // O(1) dedup via seen_tx — no need to scan pending Vec
                        if mempool.seen_tx.contains_key(&tx.txid) {
                            info!("[INFO] Transaction {} already seen, skipping", tx.txid);
                            return;
                        }
                    }

                    // Validate transaction signatures
                    info!("[P2P] 🔐 TX handler: validating signatures...");
                    let validation_start = std::time::Instant::now();
                    match tx.verify_signatures() {
                        Ok(true) => {
                            info!("[P2P] ✅ TX handler: signatures validated (took {:?})", validation_start.elapsed());
                            info!("[OK] Transaction {} received and validated from p2p", tx.txid);
                            
                            // Security: Check for double-spending in mempool
                            let mut tx_utxos = std::collections::HashSet::new();
                            for inp in &tx.inputs {
                                tx_utxos.insert(format!("{}:{}", inp.txid, inp.vout));
                            }
                            
                            let now = chrono::Utc::now().timestamp();
                            
                            info!("[P2P] 🔒 TX handler: reacquiring mempool lock for conflict check...");
                            let lock_start = std::time::Instant::now();
                            let mut mempool = state.mempool.lock().unwrap();
                            info!("[P2P] ✅ TX handler: mempool lock reacquired (took {:?})", lock_start.elapsed());

                            if mempool.seen_tx.contains_key(&tx.txid) {
                                info!("[INFO] Transaction {} already recorded, skipping", tx.txid);
                                return;
                            }

                            let mut has_conflict = false;
                            for pending_tx in &mempool.pending {
                                for pending_inp in &pending_tx.inputs {
                                    let pending_utxo =
                                        format!("{}:{}", pending_inp.txid, pending_inp.vout);
                                    if tx_utxos.contains(&pending_utxo) {
                                        warn!(
                                            "[WARN] Double-spend detected in P2P TX {}: UTXO {} already used by pending TX {}",
                                            tx.txid, pending_utxo, pending_tx.txid
                                        );
                                        has_conflict = true;
                                        break;
                                    }
                                }
                                if has_conflict {
                                    break;
                                }
                            }

                            if has_conflict {
                                false
                            } else {
                                // Mark transaction as seen with timestamp
                                mempool.seen_tx.insert(tx.txid.clone(), now);

                                // Add to mempool
                                mempool.pending.push(tx.clone());
                                // Security: Enforce mempool limits after adding transaction
                                mempool.enforce_mempool_limit();
                                info!("[INFO] Mempool size: {} transactions", mempool.pending.len());
                                info!("[P2P] ✅ TX handler: transaction added to mempool (total handler time {:?})", handler_start.elapsed());

                                true // Should relay to other peers
                            }
                        }
                        Ok(false) => {
                            warn!("[WARN] Transaction {} has invalid signatures", tx.txid);
                            info!("[P2P] ❌ TX handler: invalid signatures (total time {:?})", handler_start.elapsed());
                            false
                        }
                        Err(e) => {
                            warn!("[WARN] Transaction {} validation error: {:?}", tx.txid, e);
                            info!("[P2P] ❌ TX handler: validation error (total time {:?})", handler_start.elapsed());
                            false
                        }
                    }
                }; // Lock is released here
                
                // Relay transaction to other peers if validated
                if should_relay {
                    info!("[P2P] 📡 TX handler: relaying to peers...");
                    let relay_start = std::time::Instant::now();
                    p2p_tx_relay.broadcast_tx(&tx).await;
                    info!("[P2P] ✅ TX handler: relayed (took {:?}), total handler time {:?}", relay_start.elapsed(), handler_start.elapsed());
                    info!("[INFO] Relayed transaction {} to other peers", tx.txid);
                }
            });
        });

        // getdata handler - send requested blocks/transactions
        let nh4 = node_handle.clone();
        let p2p_clone = p2p.clone();
        p2p.set_on_getdata(move |peer_id, object_type, hashes| {
            use crate::p2p::messages::InventoryType;
            
            let state = nh4.clone();
            let p2p_inner = p2p_clone.clone();
            
            match object_type {
                InventoryType::Block => {
                    // Load and send requested blocks in order.
                    // IMPORTANT: do not spawn per-block tasks here, otherwise block
                    // responses can be reordered and create orphan storms on peers.
                    let mut sent_count = 0usize;
                    for hash_bytes in hashes {
                        let hash_hex = hex::encode(&hash_bytes);
                        // Try to load block from DB
                        if let Ok(Some(block)) = state.bc.lock().unwrap().load_block(&hash_hex) {
                            // Send immediately via peer writer queue to preserve ordering
                            p2p_inner.send_to_peer(
                                &peer_id,
                                crate::p2p::messages::P2pMessage::Block { block },
                            );
                            sent_count += 1;
                        }
                    }
                    if sent_count > 0 {
                        debug!(
                            "[SYNC] Sent {} requested blocks to {} in-order",
                            sent_count, peer_id
                        );
                    }
                }
                InventoryType::Transaction => {
                    // TODO: Send transactions from mempool
                }
                InventoryType::Error => {
                    // Ignore error type
                }
            }
        });
    }

    /// Process orphan blocks that may now be valid
    fn process_orphan_blocks(
        bc: &mut Astram_core::Blockchain,
        chain: &mut ChainState,
        mempool: &std::sync::Mutex<crate::MempoolState>,
        p2p_handle: Arc<PeerManager>,
    ) {
        if !chain.orphan_blocks.is_empty() {
            info!("[SYNC] 🔨 process_orphan_blocks called with {} orphans", chain.orphan_blocks.len());
        }
        
        let mut processed_any = true;
        let max_iterations = 100; // Prevent infinite loops
        let mut iterations = 0;

        while processed_any && iterations < max_iterations {
            processed_any = false;
            iterations += 1;

            // Find orphan blocks whose parent now exists
            let orphans_to_try: Vec<_> = chain
                .orphan_blocks
                .iter()
                .map(|(hash, (block, _))| (hash.clone(), block.clone()))
                .collect();

            for (hash, block) in orphans_to_try {
                // Check if parent exists now
                if let Ok(Some(_)) = bc.load_block(&block.header.previous_hash) {
                    info!("[SYNC] ✅ Parent found for orphan block #{}, retrying...", block.header.index);
                    // Parent exists! Try to validate and insert
                    match bc.validate_and_insert_block(&block) {
                        Ok(_) => {
                            info!(
                                "[OK] Orphan block now valid: index={} hash={}",
                                block.header.index, &hash[..16]
                            );
                            chain.blockchain.push(block.clone());
                            chain.enforce_memory_limit(); // Security: Enforce memory limit
                            chain.orphan_blocks.remove(&hash);
                            processed_any = true;

                            // Update P2P manager height
                            p2p_handle.set_my_height(block.header.index);

                            // Remove transactions from mempool
                            let block_txids: std::collections::HashSet<String> = block
                                .transactions
                                .iter()
                                .map(|tx| tx.txid.clone())
                                .collect();
                            {
                                let mut mempool = mempool.lock().unwrap();
                                mempool.pending.retain(|tx| !block_txids.contains(&tx.txid));
                            }

                            // Check for reorganization
                            let _ = bc.reorganize_if_needed(&hash);
                        }
                        Err(e) => {
                            warn!(
                                "[WARN] Orphan block still invalid: index={} hash={}, error: {:?}",
                                block.header.index, &hash[..16], e
                            );
                            // Keep in orphan pool for now
                        }
                    }
                }
            }
        }

        // Clean up old orphan blocks (older than 1 hour)
        let now = chrono::Utc::now().timestamp();
        let one_hour = 3600;
        chain.orphan_blocks.retain(|hash, (block, timestamp)| {
            let age = now - *timestamp;
            if age > one_hour {
                info!(
                    "[INFO] Removing old orphan block: index={} hash={} (age: {}s)",
                    block.header.index,
                    &hash[..16],
                    age
                );
                false
            } else {
                true
            }
        });

        if !chain.orphan_blocks.is_empty() {
            info!("Orphan pool size: {}", chain.orphan_blocks.len());
        }
    }

    /// 피어 유지 관리: heartbeat(Ping) 전송 + 피어 수 부족 시 재연결
    fn start_peer_maintenance(&self) {
        let p2p = self.manager.clone();

        // Heartbeat: 30초마다 Ping을 보내서 NAT/방화벽 연결 유지
        let p2p_ping = p2p.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(30)).await;
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let peer_count = p2p_ping.get_connected_peer_count();
                if peer_count > 0 {
                    debug!("[P2P] Sending Ping to {} peers (heartbeat)", peer_count);
                    p2p_ping.broadcast_ping(nonce);
                }
            }
        });

        // 재연결: 30초마다 피어 수 확인, 부족하면 저장된 피어에 재연결 시도
        tokio::spawn(async move {
            // 초기 연결 후 30초 대기
            sleep(Duration::from_secs(30)).await;

            loop {
                let peer_count = p2p.get_connected_peer_count();

                if peer_count < MAX_OUTBOUND / 2 {
                    info!(
                        "[P2P] Low peer count ({}/{}), attempting reconnect to saved peers...",
                        peer_count,
                        MAX_OUTBOUND
                    );

                    let saved = p2p.load_saved_peers();
                    for sp in saved.into_iter().take(MAX_OUTBOUND) {
                        let p2p_clone = p2p.clone();
                        tokio::spawn(async move {
                            if let Err(e) = p2p_clone.connect_peer(&sp.addr).await {
                                warn!("[P2P] Reconnect to {} failed: {:?}", sp.addr, e);
                            }
                        });
                    }
                }

                sleep(Duration::from_secs(30)).await;
            }
        });
    }

    fn start_block_sync(&self, node_handle: NodeHandle) {
        let p2p = self.manager.clone();
        tokio::spawn(async move {
            // How long a stalled peer stays banned from sync (10 minutes).
            const BAN_DURATION: std::time::Duration = std::time::Duration::from_secs(600);
            // If no new block committed AND none received from network within this window → ban.
            // 30s gives time for large batches (200 blocks) to be delivered and committed.
            const PEER_STALL_SECS: u64 = 30;
            // Log every N blocks to avoid spamming.
            const SYNC_LOG_INTERVAL: u64 = 100;
            // How often to refresh peer heights when already synced.
            // 10s means we catch up within 10s if a peer mines new blocks.
            const HEIGHT_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
            // Minimum gap between GetHeaders requests to the same peer (avoid spam).
            const GETHEADERS_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(2);

            // Pre-set syncing=true so blocks processed before the first loop iteration
            // (startup race window) use debug! level instead of spamming info!.
            // The loop will set it back to false once we confirm we are caught up.
            p2p.set_syncing(true);
            let mut last_syncing_state = true;
            let mut last_height_refresh = std::time::Instant::now();
            let mut last_logged_height: u64 = 0;

            // Targeted sync state
            let mut sync_peer: Option<String> = None;       // peer we are currently syncing from
            let mut last_progress_height: u64 = 0;
            let mut last_progress_time = std::time::Instant::now();
            // Rate-limit GetHeaders requests to avoid spamming the same peer.
            // Initialize far enough in the past so the first iteration sends immediately.
            let mut last_getheaders_sent = std::time::Instant::now() - Duration::from_secs(10);

            loop {
                // Purge expired bans to keep the map small.
                p2p.cleanup_sync_bans(BAN_DURATION);

                // ── 1. current height ────────────────────────────────────────────
                let my_height = {
                    let bc = node_handle.bc.lock().unwrap();
                    bc.chain_tip
                        .as_ref()
                        .and_then(|h| bc.load_header(h).ok().flatten())
                        .map(|h| h.index + 1)
                        .unwrap_or(0)
                };

                // ── 2. best available peer (non-banned, connected, highest height) ──
                let best = p2p.get_best_sync_peer(BAN_DURATION);
                let max_available_height = best.as_ref().map(|(_, h)| *h).unwrap_or(0);

                let should_sync = my_height < max_available_height;

                if should_sync {
                    if !last_syncing_state {
                        p2p.set_syncing(true);
                        last_syncing_state = true;
                    }

                    // ── 3. pick / validate current sync peer ─────────────────────
                    let target_peer = match &sync_peer {
                        Some(id) => {
                            // Keep this peer only if it's still connected, not banned,
                            // and its height is still ahead of ours.
                            let peer_height = p2p.get_peer_heights()
                                .get(id).copied().unwrap_or(0);
                            let still_valid =
                                !p2p.is_sync_banned(id, BAN_DURATION)
                                && peer_height > my_height;
                            if still_valid {
                                Some(id.clone())
                            } else {
                                // Current peer no longer useful — pick fresh best.
                                None
                            }
                        }
                        None => None,
                    };

                    let target_peer = match target_peer {
                        Some(id) => id,
                        None => {
                            match best {
                                Some((id, h)) => {
                                    info!(
                                        "[SYNC] 🎯 Targeting peer {} (height: {}, our height: {})",
                                        id, h, my_height
                                    );
                                    // Reset stall tracker when we switch peers.
                                    last_progress_height = my_height;
                                    last_progress_time = std::time::Instant::now();
                                    sync_peer = Some(id.clone());
                                    id
                                }
                                None => {
                                    // All peers are banned or gone. Wait for reconnect.
                                    debug!("[SYNC] No eligible sync peer (all banned or disconnected). Waiting…");
                                    sleep(Duration::from_secs(2)).await;
                                    continue;
                                }
                            }
                        }
                    };

                    // ── 4. stall detection for the current target peer ────────────
                    if my_height > last_progress_height {
                        last_progress_height = my_height;
                        last_progress_time = std::time::Instant::now();
                    } else if last_progress_time.elapsed().as_secs() >= PEER_STALL_SECS {
                        // Before banning, check whether blocks are still being delivered
                        // over the network (just slow to commit due to queue backlog).
                        let secs_since_block = p2p.get_last_block_received_secs_ago();
                        if secs_since_block < PEER_STALL_SECS {
                            // Blocks are still flowing — reset stall clock, don't ban.
                            debug!(
                                "[SYNC] ℹ️  Height stuck at {} but blocks still arriving ({:.0}s ago). Waiting…",
                                my_height, secs_since_block
                            );
                            last_progress_time = std::time::Instant::now();
                        } else {
                            // True stall: no blocks from network either.
                            warn!(
                                "[SYNC] ⚠️  Peer {} stalled for {}s at block #{} (target: {}). Sync-banning for {}s.",
                                target_peer,
                                last_progress_time.elapsed().as_secs(),
                                my_height,
                                max_available_height,
                                BAN_DURATION.as_secs()
                            );
                            p2p.ban_sync_peer(&target_peer, BAN_DURATION);
                            sync_peer = None;
                            last_progress_time = std::time::Instant::now();
                            // Immediately retry with next best peer on the next loop tick.
                            sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    }

                    // ── 5. send GetHeaders to the targeted peer (rate-limited) ────
                    if my_height == 0 || my_height.saturating_sub(last_logged_height) >= SYNC_LOG_INTERVAL {
                        info!(
                            "[SYNC] Progress: {}/{} ({:.1}%) via peer {}",
                            my_height, max_available_height,
                            my_height as f64 / max_available_height.max(1) as f64 * 100.0,
                            &target_peer
                        );
                        last_logged_height = my_height;
                    }

                    if last_getheaders_sent.elapsed() >= GETHEADERS_COOLDOWN {
                        let locator = {
                            let bc = node_handle.bc.lock().unwrap();
                            bc.chain_tip
                                .as_ref()
                                .and_then(|h| hex::decode(h).ok())
                                .map(|b| vec![b])
                                .unwrap_or_default()
                        };
                        p2p.request_headers_from_peer(&target_peer, locator, None);
                        last_getheaders_sent = std::time::Instant::now();
                    }

                } else {
                    // ── synced ────────────────────────────────────────────────────
                    if last_syncing_state {
                        p2p.set_syncing(false);
                        last_syncing_state = false;
                        sync_peer = None;
                        info!("[SYNC] ✅ Fully synced at height {}", my_height);
                        // Immediately refresh peer heights so we detect new blocks fast.
                        last_height_refresh = std::time::Instant::now() - HEIGHT_REFRESH_INTERVAL;
                    }

                    // Periodic height refresh (broadcast to all peers).
                    // Every 10s so we catch new blocks from active miners quickly.
                    if last_height_refresh.elapsed() >= HEIGHT_REFRESH_INTERVAL {
                        last_height_refresh = std::time::Instant::now();
                        let locator = {
                            let bc = node_handle.bc.lock().unwrap();
                            bc.chain_tip
                                .as_ref()
                                .and_then(|h| hex::decode(h).ok())
                                .map(|b| vec![b])
                                .unwrap_or_default()
                        };
                        debug!("[P2P] Periodic height refresh");
                        p2p.request_headers_from_peers(locator, None);
                    }
                }

                sleep(Duration::from_secs(1)).await;
            }
        });
    }
}

