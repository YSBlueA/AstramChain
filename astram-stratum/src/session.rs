use std::collections::VecDeque;

/// Per-connection miner state
#[derive(Debug)]
pub struct MinerSession {
    /// Worker name in "address.worker" format (e.g. "ASRMxxx.rig1")
    pub worker_name: String,
    /// Wallet address extracted from worker name
    pub miner_address: String,
    /// Unique per-connection extranonce1 (hex)
    pub extranonce1: String,
    /// Current pool difficulty for this miner (leading zero count)
    pub difficulty: u32,
    /// Shares accepted this session
    pub shares_accepted: u64,
    /// Shares rejected this session
    pub shares_rejected: u64,
    /// Unix timestamp when miner connected
    pub connected_at: i64,
    /// Unix timestamp of last accepted share
    pub last_share_at: Option<i64>,
    /// Recent accepted share timestamps for VarDiff (ring buffer, seconds)
    pub share_timestamps: VecDeque<i64>,
}

impl MinerSession {
    pub fn new(extranonce1: String, initial_difficulty: u32) -> Self {
        Self {
            worker_name: String::new(),
            miner_address: String::new(),
            extranonce1,
            difficulty: initial_difficulty,
            shares_accepted: 0,
            shares_rejected: 0,
            connected_at: chrono::Utc::now().timestamp(),
            last_share_at: None,
            share_timestamps: VecDeque::with_capacity(64),
        }
    }

    /// Populate address and worker name from "address.worker" or just "address"
    pub fn authorize(&mut self, login: &str) {
        if let Some(dot) = login.find('.') {
            self.miner_address = login[..dot].to_string();
            self.worker_name = login[dot + 1..].to_string();
        } else {
            self.miner_address = login.to_string();
            self.worker_name = "default".to_string();
        }
    }

    /// Record an accepted share and update timestamps for VarDiff
    pub fn record_accepted_share(&mut self) {
        let now = chrono::Utc::now().timestamp();
        self.shares_accepted += 1;
        self.last_share_at = Some(now);
        self.share_timestamps.push_back(now);
        // Keep only last 64 timestamps
        if self.share_timestamps.len() > 64 {
            self.share_timestamps.pop_front();
        }
    }

    pub fn record_rejected_share(&mut self) {
        self.shares_rejected += 1;
    }

    /// Average seconds between the last N accepted shares (for VarDiff).
    /// Returns None if fewer than 2 shares have been recorded.
    pub fn avg_share_time(&self) -> Option<f64> {
        let ts = &self.share_timestamps;
        if ts.len() < 2 {
            return None;
        }
        let span = (ts.back().unwrap() - ts.front().unwrap()) as f64;
        Some(span / (ts.len() - 1) as f64)
    }

    pub fn is_authorized(&self) -> bool {
        !self.miner_address.is_empty()
    }
}
