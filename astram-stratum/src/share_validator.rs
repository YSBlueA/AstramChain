use Astram_core::block::{BlockHeader, compute_header_hash};
use Astram_core::consensus::compact_to_leading_zeros;
use anyhow::{Result, anyhow};

/// Pool-side difficulty expressed as a leading-zero count (NOT compact bits).
/// Miners submit shares that meet `pool_leading_zeros`, which is lower than
/// the actual block leading zeros so they get frequent share feedback.
pub type PoolDifficulty = u32;

/// Result of validating a submitted share.
#[derive(Debug)]
pub enum ShareResult {
    /// Hash meets pool difficulty only – record as a share, do NOT submit as block.
    AcceptedShare { hash: String },
    /// Hash meets BOTH pool difficulty AND block difficulty – submit to node!
    FoundBlock { hash: String },
    /// Hash does not even meet pool difficulty.
    Rejected { reason: String },
}

/// Validate a miner-submitted nonce.
///
/// * `header` – BlockHeader with the submitted nonce already filled in.
/// * `pool_diff` – pool-side leading-zero requirement (plain count, e.g. 3).
/// * `block_compact` – network difficulty in compact-bits format (e.g. 0x1e0a_0000).
pub fn validate_share(
    header: &BlockHeader,
    pool_diff: PoolDifficulty,
    block_compact: u32,
) -> Result<ShareResult> {
    let hash = compute_header_hash(header)?;

    // Pool difficulty check (leading zeros as hex characters)
    let pool_prefix = "0".repeat(pool_diff as usize);
    if !hash.starts_with(&pool_prefix) {
        return Ok(ShareResult::Rejected {
            reason: format!(
                "hash {} does not meet pool difficulty {} (need {} leading zeros)",
                &hash[..8],
                pool_diff,
                pool_diff
            ),
        });
    }

    // Block difficulty check using the same logic as consensus
    let block_leading = compact_to_leading_zeros(block_compact) as usize;
    let block_prefix = "0".repeat(block_leading);
    if hash.starts_with(&block_prefix) {
        return Ok(ShareResult::FoundBlock { hash });
    }

    Ok(ShareResult::AcceptedShare { hash })
}

/// Convert a pool leading-zero count to the target string sent to miners.
/// e.g. pool_diff=4 → "0000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff..."
pub fn pool_diff_to_target(pool_diff: PoolDifficulty) -> String {
    let zeros = pool_diff.min(64) as usize;
    let rest = 64usize.saturating_sub(zeros);
    format!("{}{}", "0".repeat(zeros), "f".repeat(rest))
}

/// Derive a reasonable initial pool difficulty from the network compact bits.
/// We target ~1 share every 15 seconds, so pool difficulty is always several
/// steps easier than the network difficulty.
pub fn initial_pool_difficulty(block_compact: u32) -> PoolDifficulty {
    let block_zeros = compact_to_leading_zeros(block_compact);
    // Start 2 leading zeros below block requirement, min 1
    block_zeros.saturating_sub(2).max(1)
}

/// Validate that a claimed full-block hash satisfies block difficulty.
/// Used by the GBT `submitblock` path where the miner already did full PoW.
pub fn validate_full_block(header: &BlockHeader, block_compact: u32) -> Result<String> {
    let hash = compute_header_hash(header)?;
    let block_leading = compact_to_leading_zeros(block_compact) as usize;
    if !hash.starts_with(&"0".repeat(block_leading)) {
        return Err(anyhow!(
            "block hash {} does not meet difficulty (need {} leading zeros)",
            &hash[..8],
            block_leading
        ));
    }
    Ok(hash)
}
