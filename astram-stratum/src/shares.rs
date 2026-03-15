use primitive_types::U256;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// A single accepted share from a miner.
#[derive(Debug, Clone)]
pub struct Share {
    pub miner_address: String,
    pub worker_name: String,
    /// Pool difficulty at the time this share was submitted
    pub difficulty: u32,
    pub timestamp: i64,
    pub job_id: String,
}

/// A block found by the pool.
#[derive(Debug, Clone, Serialize)]
pub struct FoundBlock {
    pub height: u64,
    pub hash: String,
    pub timestamp: i64,
    pub reward: String,   // hex U256
    pub finder: String,   // miner_address of the worker who found it
    pub shares_in_window: usize,
}

/// Currently-connected, authenticated worker (keyed by extranonce1 in ShareTracker).
#[derive(Debug, Clone)]
pub struct ConnectedWorker {
    pub address: String,
    pub worker_name: String,
    pub difficulty: u32,
    pub connected_at: i64,
}

/// Per-miner aggregated statistics (for the stats API).
#[derive(Debug, Clone, Serialize)]
pub struct MinerStats {
    pub address: String,
    pub worker_name: String,
    pub difficulty: u32,
    pub shares_accepted: u64,
    pub shares_rejected: u64,
    pub shares_in_window: u64,
    pub balance: String,   // hex U256 pending payout
    pub last_share_at: Option<i64>,
    pub connected: bool,
}

/// Global share tracker shared across all stratum connections.
///
/// All mutations are gated through a `Mutex<ShareTracker>` in the caller.
pub struct ShareTracker {
    /// PPLNS rolling window of accepted shares (capped at `window_size`)
    pub recent_shares: VecDeque<Share>,
    /// Maximum number of shares in the PPLNS window
    pub window_size: usize,
    /// All blocks found by this pool (newest last)
    pub found_blocks: Vec<FoundBlock>,
    /// Pending (unpaid) balances per miner address
    pub balances: HashMap<String, U256>,
    /// Total accepted shares across all miners (lifetime)
    pub total_shares_accepted: u64,
    /// Total rejected shares
    pub total_shares_rejected: u64,
    /// Per-miner accepted share count (lifetime)
    pub miner_accepted: HashMap<String, u64>,
    /// Per-miner rejected share count (lifetime)
    pub miner_rejected: HashMap<String, u64>,
    /// Last accepted share timestamp per miner
    pub miner_last_share: HashMap<String, i64>,
    /// Currently-connected authenticated workers (key = extranonce1)
    pub connected_workers: HashMap<String, ConnectedWorker>,
}

impl ShareTracker {
    pub fn new(window_size: usize) -> Self {
        Self {
            recent_shares: VecDeque::new(),
            window_size,
            found_blocks: Vec::new(),
            connected_workers: HashMap::new(),
            balances: HashMap::new(),
            total_shares_accepted: 0,
            total_shares_rejected: 0,
            miner_accepted: HashMap::new(),
            miner_rejected: HashMap::new(),
            miner_last_share: HashMap::new(),
        }
    }

    /// Register a newly-authenticated worker (called on mining.authorize).
    pub fn register_worker(&mut self, extranonce1: String, address: String, worker_name: String, difficulty: u32) {
        self.connected_workers.insert(extranonce1, ConnectedWorker {
            address,
            worker_name,
            difficulty,
            connected_at: chrono::Utc::now().timestamp(),
        });
    }

    /// Remove a worker when its connection drops.
    pub fn unregister_worker(&mut self, extranonce1: &str) {
        self.connected_workers.remove(extranonce1);
    }

    /// Record an accepted share; trims the PPLNS window if needed.
    pub fn add_share(&mut self, share: Share) {
        self.total_shares_accepted += 1;
        *self.miner_accepted.entry(share.miner_address.clone()).or_default() += 1;
        self.miner_last_share.insert(share.miner_address.clone(), share.timestamp);

        self.recent_shares.push_back(share);
        while self.recent_shares.len() > self.window_size {
            self.recent_shares.pop_front();
        }
    }

    pub fn add_rejected(&mut self, miner_address: &str) {
        self.total_shares_rejected += 1;
        *self.miner_rejected.entry(miner_address.to_string()).or_default() += 1;
    }

    /// Distribute `total_reward` among miners proportional to their share
    /// count in the current PPLNS window and credit their pending balances.
    ///
    /// `pool_fee_fraction` is deducted first (e.g. 0.01 = 1 %).
    /// Returns a list of `(address, credited_amount)` for logging.
    pub fn distribute_pplns(
        &mut self,
        total_reward: U256,
        pool_fee_fraction: f64,
        finder: &str,
        block_height: u64,
        block_hash: String,
        timestamp: i64,
    ) -> Vec<(String, U256)> {
        // Deduplicate: if this block hash was already processed, skip entirely.
        if self.found_blocks.iter().any(|b| b.hash == block_hash) {
            return Vec::new();
        }

        let window_len = self.recent_shares.len();

        // Count shares per miner in the window
        let mut window_counts: HashMap<&str, u64> = HashMap::new();
        for share in &self.recent_shares {
            *window_counts.entry(share.miner_address.as_str()).or_default() += 1;
        }

        // Deduct pool fee
        let fee_amount = {
            let fee_u128 = (total_reward.as_u128() as f64 * pool_fee_fraction) as u128;
            U256::from(fee_u128)
        };
        let distributable = total_reward.saturating_sub(fee_amount);

        // Distribute proportionally
        let total_in_window = window_len as u64;
        let mut credits: Vec<(String, U256)> = Vec::new();

        if total_in_window == 0 {
            // No shares in window – credit full reward to finder
            let balance = self.balances.entry(finder.to_string()).or_default();
            *balance = balance.saturating_add(distributable);
            credits.push((finder.to_string(), distributable));
        } else {
            for (addr, count) in &window_counts {
                // credit = distributable * count / total_in_window
                let credit = distributable * U256::from(*count) / U256::from(total_in_window);
                if credit > U256::zero() {
                    let balance = self.balances.entry(addr.to_string()).or_default();
                    *balance = balance.saturating_add(credit);
                    credits.push((addr.to_string(), credit));
                }
            }
        }

        // Record found block
        self.found_blocks.push(FoundBlock {
            height: block_height,
            hash: block_hash,
            timestamp,
            reward: format!("0x{:x}", total_reward),
            finder: finder.to_string(),
            shares_in_window: window_len,
        });

        credits
    }

    /// Snapshot of per-miner stats (for the stats API).
    pub fn miner_stats(&self) -> Vec<MinerStats> {
        // Count shares in window per miner
        let mut window_counts: HashMap<&str, u64> = HashMap::new();
        for share in &self.recent_shares {
            *window_counts.entry(share.miner_address.as_str()).or_default() += 1;
        }

        // Build a map of address -> connected worker info (last connected worker per address)
        let mut connected_info: HashMap<&str, &ConnectedWorker> = HashMap::new();
        for w in self.connected_workers.values() {
            connected_info.insert(w.address.as_str(), w);
        }

        // Union of all known addresses: share history + currently connected
        let mut addresses: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for addr in self.miner_accepted.keys() { addresses.insert(addr); }
        for addr in self.miner_rejected.keys() { addresses.insert(addr); }
        for w in self.connected_workers.values() { addresses.insert(w.address.as_str()); }

        addresses
            .into_iter()
            .map(|addr| {
                let conn = connected_info.get(addr);
                MinerStats {
                    address: addr.to_string(),
                    worker_name: conn.map(|w| w.worker_name.clone()).unwrap_or_default(),
                    difficulty: conn.map(|w| w.difficulty).unwrap_or(0),
                    shares_accepted: *self.miner_accepted.get(addr).unwrap_or(&0),
                    shares_rejected: *self.miner_rejected.get(addr).unwrap_or(&0),
                    shares_in_window: *window_counts.get(addr).unwrap_or(&0),
                    balance: format!(
                        "0x{:x}",
                        self.balances.get(addr).cloned().unwrap_or(U256::zero())
                    ),
                    last_share_at: self.miner_last_share.get(addr).cloned(),
                    connected: conn.is_some(),
                }
            })
            .collect()
    }
}
