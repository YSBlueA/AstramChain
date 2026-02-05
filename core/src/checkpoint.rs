/// Checkpoint Policy System (Not Consensus Rules)
/// 
/// Checkpoints are POLICY-LEVEL protections, NOT consensus rules.
/// They provide additional security for:
/// - Mining pools (prevent deep reorg attacks on payouts)
/// - Exchanges (safe confirmation depth for deposits)
/// - Block explorers (stable reference points)
/// 
/// Effects:
/// - Prevents long-range history rewrite attacks
/// - Provides network stability during early mainnet phase
/// - Gives social consensus anchor points
/// 
/// Important: Checkpoints do NOT affect the validity of blocks themselves.
/// They only affect chain selection policy in this specific node implementation.

/// Checkpoint: Policy-level chain anchor point
/// Format: (block_height, block_hash)
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub height: u64,
    pub hash: String,
    pub description: String, // Human-readable description of checkpoint significance
}

/// Get hardcoded checkpoints for NetCoin
/// 
/// These are POLICY anchors, not consensus rules.
/// Nodes MAY use these for:
/// - Rejecting chains that conflict with checkpoints
/// - Faster initial sync by trusting checkpoint validity
/// - Protection against very deep reorganizations
/// 
/// Checkpoints should be set at:
/// - Genesis (height 0)
/// - Major network milestones (every 10,000 blocks)
/// - After significant events (hard forks, major upgrades)
pub fn get_checkpoints() -> Vec<Checkpoint> {
    vec![
        // Genesis block - immutable network origin
        Checkpoint {
            height: 0,
            hash: "".to_string(), // Set after genesis creation
            description: "Genesis Block - Network Origin".to_string(),
        },
        // Add more checkpoints as network matures
        // Example milestone checkpoints:
        // Checkpoint {
        //     height: 10000,
        //     hash: "abc123...".to_string(),
        //     description: "First 10K blocks milestone".to_string(),
        // },
        // Checkpoint {
        //     height: 100000,
        //     hash: "def456...".to_string(),
        //     description: "100K blocks - Network established".to_string(),
        // },
    ]
}

/// Policy check: Validate that a chain doesn't conflict with checkpoints
/// 
/// This is a POLICY decision, not a consensus rule.
/// Returns false if the block conflicts with a known checkpoint,
/// which means this chain should be rejected by THIS node's policy.
/// 
/// Other nodes without checkpoints enabled will still see the block as valid.
pub fn validate_against_checkpoints(height: u64, hash: &str) -> bool {
    let checkpoints = get_checkpoints();

    for cp in checkpoints {
        if cp.height == height {
            if cp.hash.is_empty() {
                // Checkpoint not set yet, skip validation
                continue;
            }

            if cp.hash != hash {
                log::error!(
                    "🚨 CHECKPOINT POLICY VIOLATION: Block at height {} has hash {}, but checkpoint requires {}",
                    height,
                    &hash[..16.min(hash.len())],
                    &cp.hash[..16.min(cp.hash.len())]
                );
                log::error!(
                    "   Checkpoint description: {}",
                    cp.description
                );
                return false;
            } else {
                log::info!(
                    "✅ Block matches checkpoint at height {}: {}",
                    height,
                    cp.description
                );
            }
        }
    }

    true
}

/// Get the latest checkpoint height
/// Blocks below this height are considered policy-final by this node
pub fn get_latest_checkpoint_height() -> u64 {
    let checkpoints = get_checkpoints();
    checkpoints.iter().map(|cp| cp.height).max().unwrap_or(0)
}

/// Check if reorganization would conflict with checkpoint policy
/// Returns (allowed, reason)
pub fn check_reorg_against_checkpoints(
    reorg_depth: u64,
    current_height: u64,
) -> (bool, Option<String>) {
    let latest_checkpoint = get_latest_checkpoint_height();
    let reorg_target_height = current_height.saturating_sub(reorg_depth);

    if reorg_target_height <= latest_checkpoint {
        return (
            false,
            Some(format!(
                "Reorg would go below checkpoint at height {} (target height: {})",
                latest_checkpoint, reorg_target_height
            )),
        );
    }

    (true, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_validation() {
        // Valid block at non-checkpoint height
        assert!(validate_against_checkpoints(50, "any_hash"));

        // Genesis checkpoint (empty hash = not enforced yet)
        assert!(validate_against_checkpoints(0, "any_hash"));
    }

    #[test]
    fn test_latest_checkpoint() {
        let height = get_latest_checkpoint_height();
        assert_eq!(height, 0); // Currently only genesis
    }

    #[test]
    fn test_reorg_checkpoint_policy() {
        // Reorg of 10 blocks from height 100 - should be allowed (target: 90)
        let (allowed, _) = check_reorg_against_checkpoints(10, 100);
        assert!(allowed);

        // Deep reorg that would go below genesis (checkpoint at 0)
        let (allowed, reason) = check_reorg_against_checkpoints(150, 100);
        assert!(!allowed);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("checkpoint"));
    }
}
