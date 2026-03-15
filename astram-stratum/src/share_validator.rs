use Astram_core::block::{BlockHeader, compute_header_hash};
use primitive_types::U256;
use anyhow::{Result, anyhow};

/// Pool-side difficulty expressed as a leading-zero count (NOT compact bits).
/// Miners submit shares that meet `pool_leading_zeros`, which is lower than
/// the actual block difficulty so they get frequent share feedback.
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

/// Convert compact bits (nBits) to a U256 target value.
/// Identical logic to Blockchain::compact_to_target in the node.
fn compact_bits_to_u256(bits: u32) -> U256 {
    let exponent = bits >> 24;
    let mantissa = bits & 0x007f_ffff;
    if mantissa == 0 {
        return U256::zero();
    }
    if exponent <= 3 {
        U256::from(mantissa >> (8 * (3 - exponent)))
    } else {
        U256::from(mantissa) << (8 * (exponent - 3))
    }
}

/// Parse a 64-char hex hash string into a U256 (big-endian).
fn hash_hex_to_u256(hash_hex: &str) -> Result<U256> {
    let bytes = hex::decode(hash_hex)
        .map_err(|e| anyhow!("invalid hash hex: {}", e))?;
    if bytes.len() != 32 {
        return Err(anyhow!("hash must be 32 bytes, got {}", bytes.len()));
    }
    Ok(U256::from_big_endian(&bytes))
}

/// Validate a miner-submitted nonce.
///
/// * `header`        – BlockHeader with the submitted nonce already filled in.
/// * `pool_diff`     – pool-side leading-zero requirement (plain count, e.g. 1).
/// * `block_compact` – network difficulty in compact-bits format (e.g. 0x1f05_6013).
///
/// Block detection uses the same numeric comparison as the node
/// (`hash < compact_target`), not a leading-zero string check.
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

    // Block difficulty check: same numeric comparison as the node
    // (compact_to_leading_zeros is unreliable for high-exponent compact values)
    let block_target = compact_bits_to_u256(block_compact);
    if block_target.is_zero() {
        // Degenerate target — treat as not a block
        return Ok(ShareResult::AcceptedShare { hash });
    }
    let hash_value = hash_hex_to_u256(&hash)?;
    if hash_value < block_target {
        return Ok(ShareResult::FoundBlock { hash });
    }

    Ok(ShareResult::AcceptedShare { hash })
}

/// Convert a pool leading-zero count to the target string sent to miners.
/// e.g. pool_diff=4 → "0000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
pub fn pool_diff_to_target(pool_diff: PoolDifficulty) -> String {
    let zeros = pool_diff.min(64) as usize;
    let rest = 64usize.saturating_sub(zeros);
    format!("{}{}", "0".repeat(zeros), "f".repeat(rest))
}

/// Derive a reasonable initial pool difficulty from the network compact bits.
///
/// Strategy: pool difficulty (leading-zero count) is chosen so the expected
/// share rate is roughly 1 per 15 seconds at the miner's hashrate, which is
/// always easier than the network.
///
/// We derive it from the number of leading hex zeros the network target has,
/// using the numeric target directly so it works for all compact-bits values.
pub fn initial_pool_difficulty(block_compact: u32) -> PoolDifficulty {
    let network_target = compact_bits_to_u256(block_compact);
    if network_target.is_zero() {
        return 1;
    }

    // Count how many leading hex nibbles of the target are zero.
    // That tells us how many leading zeros the HARDEST hash the network accepts
    // has — so pool_diff should be 2 less (easier).
    let mut target_bytes = [0u8; 32];
    network_target.to_big_endian(&mut target_bytes);

    // Count leading zero nibbles in the 256-bit target
    let mut leading_zero_nibbles: u32 = 0;
    for byte in &target_bytes {
        if *byte == 0 {
            leading_zero_nibbles += 2;
        } else {
            if byte >> 4 == 0 {
                leading_zero_nibbles += 1;
            }
            break;
        }
    }

    // Pool difficulty: 2 leading zeros easier than network, minimum 1
    leading_zero_nibbles.saturating_sub(2).max(1)
}

/// Validate that a claimed full-block hash satisfies block difficulty.
/// Used by the GBT `submitblock` path where the miner already did full PoW.
pub fn validate_full_block(header: &BlockHeader, block_compact: u32) -> Result<String> {
    let hash = compute_header_hash(header)?;
    let block_target = compact_bits_to_u256(block_compact);
    if block_target.is_zero() {
        return Err(anyhow!("degenerate block target (compact bits = 0x{:08x})", block_compact));
    }
    let hash_value = hash_hex_to_u256(&hash)?;
    if hash_value >= block_target {
        return Err(anyhow!(
            "block hash {} does not meet difficulty (compact bits 0x{:08x})",
            &hash[..8],
            block_compact
        ));
    }
    Ok(hash)
}
