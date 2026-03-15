// core/consensus.rs

pub mod dag;

#[cfg(feature = "cuda-miner")]
pub mod cuda;

#[cfg(feature = "cuda-miner")]
pub use cuda::mine_block_with_coinbase_cuda;

#[cfg(feature = "cuda-miner")]
pub use cuda::mine_header_cuda;

/// Convert compact difficulty format (bits) to required leading zero count.
/// Used for display / logging purposes.
pub fn compact_to_leading_zeros(bits: u32) -> u32 {
    let exponent = bits >> 24;

    if exponent == 0 {
        return 0;
    }

    let baseline_exp = 0x1d_u32;
    let baseline_zeros = 2_u32;

    if exponent <= baseline_exp {
        let harder_steps = baseline_exp - exponent;
        return (baseline_zeros + harder_steps.saturating_mul(2)).min(64);
    }

    let easier_steps = exponent - baseline_exp;
    baseline_zeros.saturating_sub(easier_steps.saturating_mul(2))
}
