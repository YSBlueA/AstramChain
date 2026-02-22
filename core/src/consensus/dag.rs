// DAG (Directed Acyclic Graph) generation for memory-hard PoW
// Inspired by Ethash/KawPow but using Blake3 instead of Keccak

use blake3;
use anyhow::Result;

/// DAG parameters
pub const DAG_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4GB
pub const DAG_ITEM_SIZE: usize = 128; // 128 bytes per item
pub const DAG_ITEM_COUNT: usize = DAG_SIZE / DAG_ITEM_SIZE; // 32M items
pub const EPOCH_LENGTH: u64 = 7500; // Regenerate DAG every 7500 blocks (~5 days @ 1 min/block)
pub const MIX_ITERATIONS: usize = 32; // Increased from 16 (balanced performance/security)

/// Compute epoch from block number
pub fn get_epoch(block_number: u64) -> u64 {
    block_number / EPOCH_LENGTH
}

/// Generate seed hash for epoch (Blake3-based)
pub fn get_seed_hash(epoch: u64) -> [u8; 32] {
    if epoch == 0 {
        // Genesis seed
        *blake3::hash(b"Astram Genesis DAG Seed").as_bytes()
    } else {
        // Recursive: seed[n] = Blake3(seed[n-1])
        let prev_seed = get_seed_hash(epoch - 1);
        *blake3::hash(&prev_seed).as_bytes()
    }
}

/// Generate a single DAG item from index and seed
/// Uses Blake3 in a pseudo-random fashion similar to Ethash
pub fn generate_dag_item(index: u32, seed: &[u8; 32]) -> [u8; DAG_ITEM_SIZE] {
    let mut item = [0u8; DAG_ITEM_SIZE];
    
    // Initial hash: Blake3(seed || index)
    let mut input = Vec::with_capacity(36);
    input.extend_from_slice(seed);
    input.extend_from_slice(&index.to_le_bytes());
    let initial = blake3::hash(&input);
    
    // Fill first 32 bytes
    item[..32].copy_from_slice(initial.as_bytes());
    
    // Expand to 128 bytes using Blake3 in counter mode
    for i in 1u32..4u32 {
        let mut counter_input = Vec::with_capacity(64);
        counter_input.extend_from_slice(&item[..32]);
        counter_input.extend_from_slice(&i.to_le_bytes());
        let expansion = blake3::hash(&counter_input);
        let start = (i * 32) as usize;
        let end = std::cmp::min(start + 32, DAG_ITEM_SIZE);
        item[start..end].copy_from_slice(&expansion.as_bytes()[..end-start]);
    }
    
    // Simple mixing without recursive parent lookup (much faster)
    // Use FNV-like hash mixing for pseudo-randomness
    for round in 0u32..4u32 {
        let mut mix_input = Vec::with_capacity(132);
        mix_input.extend_from_slice(&item);
        mix_input.extend_from_slice(&round.to_le_bytes());
        let mixed = blake3::hash(&mix_input);
        
        // XOR first 32 bytes
        for j in 0..32 {
            item[j] ^= mixed.as_bytes()[j];
        }
    }
    
    item
}

/// Generate the full DAG for an epoch (4GB - this is expensive!)
/// In production, this should be cached to disk
pub fn generate_full_dag(epoch: u64) -> Result<Vec<u8>> {
    let seed = get_seed_hash(epoch);
    let mut dag = vec![0u8; DAG_SIZE];
    
    println!("[DAG] Generating 4GB DAG for epoch {}... (this takes several minutes)", epoch);
    
    // Generate items in parallel using rayon
    use rayon::prelude::*;
    
    // Process in chunks to show progress
    let chunk_size = 100_000; // ~12.8MB chunks
    let total_chunks = (DAG_ITEM_COUNT + chunk_size - 1) / chunk_size;
    
    for chunk_idx in 0..total_chunks {
        let start_idx = chunk_idx * chunk_size;
        let end_idx = std::cmp::min(start_idx + chunk_size, DAG_ITEM_COUNT);
        
        let items: Vec<[u8; DAG_ITEM_SIZE]> = (start_idx..end_idx)
            .into_par_iter()
            .map(|i| generate_dag_item(i as u32, &seed))
            .collect();
        
        // Copy to main DAG
        for (i, item) in items.iter().enumerate() {
            let dag_offset = (start_idx + i) * DAG_ITEM_SIZE;
            dag[dag_offset..dag_offset + DAG_ITEM_SIZE].copy_from_slice(item);
        }
        
        if chunk_idx % 10 == 0 {
            let progress = (chunk_idx * 100) / total_chunks;
            println!("[DAG] Progress: {}%", progress);
        }
    }
    
    println!("[DAG] Generation complete!");
    Ok(dag)
}

/// Hash a header with the DAG (memory-hard mixing)
/// This is what miners compute repeatedly with different nonces
pub fn hash_with_dag(header_hash: &[u8; 32], nonce: u64, dag: &[u8]) -> [u8; 32] {
    // Step 1: Initial seed from header + nonce
    let mut seed_input = Vec::with_capacity(40);
    seed_input.extend_from_slice(header_hash);
    seed_input.extend_from_slice(&nonce.to_le_bytes());
    let seed = blake3::hash(&seed_input);
    
    // Step 2: Memory-hard mixing with DAG (KawPow-style)
    let mut mix = [0u8; 128];
    mix[..32].copy_from_slice(seed.as_bytes());
    
    // Expand seed to 128 bytes
    for i in 1..4 {
        let mut expand_input = Vec::with_capacity(36);
        expand_input.extend_from_slice(&mix[..32]);
        expand_input.extend_from_slice(&(i as u32).to_le_bytes());
        let expanded = blake3::hash(&expand_input);
        let start = i * 32;
        mix[start..start + 32].copy_from_slice(expanded.as_bytes());
    }
    

    
    // Perform random DAG accesses
    for iteration in 0..MIX_ITERATIONS {
        // Compute DAG index from current mix state
        let mut index_bytes = [0u8; 4];
        let offset = iteration % 4 * 32;
        index_bytes.copy_from_slice(&mix[offset..offset + 4]);
        let dag_index = (u32::from_le_bytes(index_bytes) as usize) % DAG_ITEM_COUNT;
        
        // Fetch DAG item
        let dag_offset = dag_index * DAG_ITEM_SIZE;
        let dag_item = &dag[dag_offset..dag_offset + DAG_ITEM_SIZE];
        

        
        // Mix with XOR + Blake3
        for i in 0..DAG_ITEM_SIZE {
            mix[i] ^= dag_item[i];
        }
        

        
        // Hash the mix for next iteration
        let mixed = blake3::hash(&mix);
        mix[..32].copy_from_slice(mixed.as_bytes());
        

    }
    
    // Step 3: Final Blake3 hash
    let final_hash = blake3::hash(&mix);
    *final_hash.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_epoch_calculation() {
        assert_eq!(get_epoch(0), 0);
        assert_eq!(get_epoch(7499), 0);
        assert_eq!(get_epoch(7500), 1);
        assert_eq!(get_epoch(15000), 2);
    }
    
    #[test]
    fn test_seed_deterministic() {
        let seed0_a = get_seed_hash(0);
        let seed0_b = get_seed_hash(0);
        assert_eq!(seed0_a, seed0_b);
        
        let seed1_a = get_seed_hash(1);
        let seed1_b = get_seed_hash(1);
        assert_eq!(seed1_a, seed1_b);
    }
    
    #[test]
    fn test_dag_item_deterministic() {
        let seed = get_seed_hash(0);
        let item1 = generate_dag_item(0, &seed);
        let item2 = generate_dag_item(0, &seed);
        assert_eq!(item1, item2);
    }
}
