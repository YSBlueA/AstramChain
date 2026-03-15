use crate::session::MinerSession;

/// VarDiff configuration
#[derive(Debug, Clone)]
pub struct VarDiffConfig {
    /// Minimum pool difficulty (leading zeros)
    pub min_diff: u32,
    /// Maximum pool difficulty (leading zeros)
    pub max_diff: u32,
    /// Target seconds between shares (e.g. 15s)
    pub target_share_time: f64,
    /// How far actual share time can deviate before adjustment (ratio)
    /// e.g. 0.5 means adjust when avg is <0.5x or >2x of target
    pub variance_percent: f64,
    /// Minimum shares before VarDiff considers an adjustment
    pub min_samples: usize,
}

impl Default for VarDiffConfig {
    fn default() -> Self {
        Self {
            min_diff: 1,
            max_diff: 32,
            target_share_time: 15.0,
            variance_percent: 0.5,
            min_samples: 5,
        }
    }
}

/// Check whether the miner's difficulty should change and return the new value.
/// Returns `None` if no adjustment is needed or not enough data yet.
///
/// Two modes:
///   1. Share-based: enough samples collected → adjust by avg share time.
///   2. Time-based:  no (or few) shares yet → reduce difficulty if the miner
///      has gone more than 2× target_share_time without a share.
pub fn check_vardiff(session: &MinerSession, cfg: &VarDiffConfig) -> Option<u32> {
    let now = chrono::Utc::now().timestamp();

    // ── Time-based fallback (no / few samples) ───────────────────────────────
    if session.share_timestamps.len() < cfg.min_samples {
        // Use connection time if no shares ever, otherwise time since last share.
        let reference = session
            .share_timestamps
            .back()
            .copied()
            .unwrap_or(session.connected_at);
        let elapsed = (now - reference) as f64;

        if elapsed > cfg.target_share_time * 2.0 {
            let new_diff = session.difficulty.saturating_sub(1).max(cfg.min_diff);
            if new_diff != session.difficulty {
                log::debug!(
                    "[VarDiff] worker={}.{} no share for {:.0}s → diff {} → {}",
                    session.miner_address,
                    session.worker_name,
                    elapsed,
                    session.difficulty,
                    new_diff
                );
                return Some(new_diff);
            }
        }
        return None;
    }

    // ── Share-based adjustment ────────────────────────────────────────────────
    let avg = session.avg_share_time()?;

    let low = cfg.target_share_time * cfg.variance_percent;
    let high = cfg.target_share_time / cfg.variance_percent;

    if avg >= low && avg <= high {
        return None;
    }

    // Adjust: shares too fast → raise diff, shares too slow → lower diff
    let new_diff = if avg < low {
        (session.difficulty + 1).min(cfg.max_diff)
    } else {
        session.difficulty.saturating_sub(1).max(cfg.min_diff)
    };

    if new_diff == session.difficulty {
        return None;
    }

    log::debug!(
        "[VarDiff] worker={}.{} avg_share_time={:.1}s target={:.1}s diff {} → {}",
        session.miner_address,
        session.worker_name,
        avg,
        cfg.target_share_time,
        session.difficulty,
        new_diff
    );

    Some(new_diff)
}
