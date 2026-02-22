# KawPow-Blake3 PoW Algorithm

## Overview

Astram uses a memory-hard Proof-of-Work algorithm inspired by KawPow (Ravencoin) but using Blake3 instead of SHA256. This design provides **GPU optimization** while maintaining **ASIC resistance** through high memory requirements.

## Algorithm Design

### Three-Stage Hashing

```
1. Blake3(header) → initial_hash [32 bytes]
2. Memory Mix with 4GB DAG → mixed_data [128 bytes]  
3. Blake3(mixed_data) → final_hash [32 bytes]
```

### Stage 1: Initial Hash
- Input: Block header (bincode serialized)
- Algorithm: Blake3 single-pass hash
- Output: 32-byte header hash

### Stage 2: Memory-Hard Mixing (The ASIC-Resistant Part)
- Input: header_hash + nonce
- **DAG Accesses**: 64 random reads from 4GB dataset
- **DAG Item Size**: 128 bytes per item
- **Total DAG**: 4GB = 32M items × 128 bytes
- Each iteration:
  1. Compute DAG index from current mix state
  2. Fetch 128-byte DAG item from memory
  3. XOR with current mix
  4. Blake3 hash for next iteration
- Output: 128-byte final mix

### Stage 3: Final Hash
- Input: 128-byte mixed data
- Algorithm: Blake3 single-pass hash
- Output: Final 32-byte hash for difficulty check

## DAG (Directed Acyclic Graph)

### Parameters
- **DAG Size**: 4GB (4,294,967,296 bytes)
- **Item Size**: 128 bytes
- **Item Count**: 33,554,432 (32M items)
- **Epoch Length**: 7,500 blocks (~5 days @ 1 min/block)

### Epoch System
Each epoch has a unique DAG generated from a seed:
- **Epoch 0**: seed = Blake3("Astram Genesis DAG Seed")
- **Epoch N**: seed = Blake3(seed[N-1])

DAG regenerates every 7,500 blocks to prevent pre-computed ASIC optimizations.

### DAG Generation
```rust
for index in 0..32M {
    item = Blake3(seed || index)
    expand item to 128 bytes using Blake3 counter mode
    mix with 4 rounds of Blake3 hashing
}
```

Generation time: **~3-5 minutes** on modern CPUs (parallelized with rayon)

### Memory Requirements
- **Minimum VRAM**: 4GB (for GPU mining)
- **Minimum RAM**: 4GB (for CPU mining)
- **Storage**: Optional DAG caching to disk (~4GB per epoch)

## ASIC Resistance Model

### Why This Design Resists ASICs

1. **High Memory Bandwidth Requirement**
   - 64 random accesses × 128 bytes = 8KB per hash
   - At 1 GH/s: 8 TB/s memory bandwidth needed
   - Modern GPUs: 200-500 GB/s (feasible)
   - ASIC cost amplification: Must include large DRAM chips

2. **Random Access Pattern**
   - Cannot predict which DAG items will be accessed
   - Prevents caching optimizations
   - Requires full 4GB dataset in fast memory

3. **Blake3 Computation**
   - Modern, SIMD-optimized algorithm
   - GPUs excel at SIMD workloads
   - ASICs gain less advantage vs SHA256

4. **Epoch Changes**
   - DAG regenerates every 7,500 blocks
   - Prevents long-term hardware specialization
   - Any ASIC must be general-purpose enough to handle new DAGs

### Cost Amplification Factor

**GPU Mining**:
- GTX 1050 Ti: 4GB GDDR5, ~$150
- Expected hashrate: 50-150 MH/s (estimated)

**Hypothetical ASIC**:
- Must include 4GB GDDR6: ~$100 in components
- Custom chip: $1M+ NRE cost
- Break-even only at massive scale (>10,000 units)
- Risk of epoch change invalidating optimizations

**Result**: ASICs ~3-5× more expensive per hash than GPUs (vs 100,000× for SHA256)

## Performance Characteristics

### Expected Hashrates

**CPU Mining** (4GB RAM):
- Modern 8-core: 100-500 KH/s
- High-end 16-core: 500KH-1MH/s

**GPU Mining** (4GB+ VRAM):
- GTX 1050 Ti (4GB): 50-150 MH/s (estimated)
- RTX 3070 (8GB): 200-400 MH/s (estimated)
- RTX 4090 (24GB): 500-800 MH/s (estimated)

*Note: These are estimates. Actual performance will vary based on optimization.*

### Bottlenecks

1. **Memory Bandwidth** (Primary)
   - 64 × 128-byte random reads per hash
   - GPU memory bandwidth is the main limiting factor

2. **Blake3 Computation** (Secondary)
   - 66+ Blake3 hashes per mining attempt
   - Well-optimized on both CPU and GPU

3. **DAG Upload** (One-time)
   - 4GB upload to GPU: ~1-2 seconds on PCIe 3.0
   - Reused for entire mining session

## Implementation Details

### Files Modified

1. **[core/src/consensus/dag.rs](../core/src/consensus/dag.rs)** (NEW)
   - DAG generation with Blake3
   - Epoch management
   - Memory-hard mixing function

2. **[core/src/consensus/mod.rs](../core/src/consensus/mod.rs)**
   - CPU miner using DAG
   - Integrated epoch checking

3. **[core/src/consensus/cuda.rs](../core/src/consensus/cuda.rs)**
   - DAG upload to GPU
   - CUDA kernel launch with DAG parameters

4. **[core/src/consensus/cuda/miner.cu](../core/src/consensus/cuda/miner.cu)**
   - Blake3 CUDA implementation
   - Memory-hard mixing kernel
   - 64 random DAG accesses per hash

### CUDA Kernel Features

- **Blake3 Implementation**: 7-round compression in constant memory
- **DAG Access**: Global memory reads (4GB dataset)
- **Parallel Execution**: 512 threads/block, 8192 blocks maximum
- **Batch Processing**: 16M hashes per GPU call

### Compilation

```powershell
# With CUDA support (GTX 1050 Ti or better, 4GB+ VRAM)
cargo build --release --features cuda-miner

# CPU-only (no CUDA)
cargo build --release
```

## Mining Usage

### Start GPU Mining
```powershell
.\target\release\Astram-node.exe --miner --address <your_address>
```

On first run, the node will:
1. Generate 4GB DAG (~3-5 minutes)
2. Upload DAG to GPU (~1-2 seconds)
3. Start mining

### DAG Caching (TODO)
Future optimization: Save DAG to disk to avoid regeneration on restart.

```rust
// Planned cache location
~/.astram/dag/epoch-{N}.bin  // 4GB file per epoch
```

## Security Analysis

### Attack Vectors

1. **Pre-computed DAG Optimization**
   - Mitigation: Epoch changes every 7,500 blocks
   - Attacker must regenerate DAG frequently

2. **Memory Compression**
   - Mitigation: Blake3's strong diffusion prevents compression
   - Random access pattern prevents caching strategies

3. **FPGA Mining**
   - FPGAs have limited memory bandwidth
   - Cost-effectiveness similar to GPUs
   - No significant advantage

4. **GPU Clusters**
   - Expected and intended use case
   - Distributed mining pools work normally
   - Individual GPUs still need 4GB VRAM each

### Difficulty Adjustment
Uses same system as before:
- Compact bits format: `0x1e0fffff`
- Leading zeros requirement
- Adjusted per difficulty retarget algorithm

## Comparison with Other Algorithms

| Algorithm | Memory | ASIC Resistance | GPU Hashrate | Notes |
|-----------|--------|-----------------|--------------|-------|
| SHA256d (Bitcoin) | 0 | None | 40 MH/s | ASICs 100,000× faster |
| Scrypt (Litecoin) | 128KB | Low | 500 KH/s | ASICs exist |
| Ethash (Ethereum) | 4GB | High | 30 MH/s | Proven ASIC-resistant |
| **KawPow (Ravencoin)** | 4GB | High | 25 MH/s | Inspiration for this |
| **Blake3-KawPow (Astram)** | **4GB** | **High** | **50-150 MH/s** | **Faster hashing** |

## Future Optimizations

1. **DAG Caching**: Save to disk, load on restart
2. **Light Client Verification**: Verify without full DAG
3. **Memory Pool**: Share DAG across multiple mining threads
4. **Adaptive Batch Size**: Tune for different GPU models
5. **Progpow-style Random Program**: Further ASIC resistance

## References

- **Blake3**: https://github.com/BLAKE3-team/BLAKE3
- **KawPow Specification**: https://github.com/RavenCommunity/kawpow
- **Ethash Design**: https://ethereum.github.io/yellowpaper/paper.pdf
- **ASIC Resistance Analysis**: https://www.paradigm.xyz/2020/06/asic-resistance

---

**Implementation Date**: February 2026  
**Version**: 1.0  
**Status**: Experimental - Performance testing needed
