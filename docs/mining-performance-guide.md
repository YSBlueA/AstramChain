# Mining Performance Testing Guide

## Quick Start

### 1. Build with CUDA
```powershell
cargo build --release --features cuda-miner
```

### 2. Start Mining
```powershell
.\target\release\Astram-node.exe --miner --address 0xYOUR_ADDRESS_HERE
```

### Expected Output
```
[DAG] Generating 4GB DAG for epoch 0... (this takes several minutes)
[DAG] Progress: 0%
[DAG] Progress: 10%
...
[DAG] Progress: 100%
[DAG] Generation complete!
[CUDA] DAG uploaded to GPU, starting mining...
[DEBUG] Mining: Entering memory-hard mining loop, difficulty=0x1e0fffff requires 4 leading zeros
Mining hashrate: 85.2 MH/s  (example)
```

## Performance Benchmarking

### Test 1: DAG Generation Speed
Measures CPU parallelization efficiency:
```powershell
cargo run --release --example bench-dag
```

**Expected**: 3-5 minutes on modern 8-core CPU

### Test 2: CPU Miner Performance
Test without GPU (baseline):
```powershell
cargo build --release
.\target\release\Astram-node.exe --miner --cpu-only
```

**Expected**: 100-500 KH/s

### Test 3: GPU Miner Performance
Full CUDA mining:
```powershell
cargo build --release --features cuda-miner
.\target\release\Astram-node.exe --miner
```

**Expected on GTX 1050 Ti**: 50-150 MH/s

### Test 4: Memory Bandwidth Utilization
Check GPU memory bandwidth usage:
```powershell
# In another terminal, run nvidia-smi
nvidia-smi dmon -s u
```

Should see high memory utilization (>80%).

## Optimization Tuning

### Environment Variables

#### CUDA_BATCH_SIZE
Number of hashes per GPU kernel call:
```powershell
$env:CUDA_BATCH_SIZE = "33554432"  # 32M hashes (default: 16M)
.\target\release\Astram-node.exe --miner
```

**Effects**:
- Larger: Better GPU utilization, slower cancel response
- Smaller: Faster response, lower hashrate

**Recommended**:
- GTX 1050 Ti: 16M (default)
- RTX 3070: 32M
- RTX 4090: 64M

### Code-Level Tuning

#### 1. Threads Per Block ([cuda.rs](../core/src/consensus/cuda.rs#L16))
```rust
const THREADS_PER_BLOCK: u32 = 512;  // Current
```

Try different values for your GPU:
- **128**: Older GPUs (compute capability < 3.0)
- **256**: GTX 900/1000 series
- **512**: GTX 1050 Ti to RTX 2000 series (recommended)
- **1024**: RTX 3000/4000 series

#### 2. Max Blocks ([cuda.rs](../core/src/consensus/cuda.rs#L17))
```rust
const MAX_BLOCKS: u32 = 8192;  // Current
```

Adjust based on GPU:
- GTX 1050 Ti (768 cores): 4096-8192
- RTX 3070 (5888 cores): 16384
- RTX 4090 (16384 cores): 32768

Formula: `MAX_BLOCKS = (CUDA_cores / THREADS_PER_BLOCK) * 4`

#### 3. Mix Iterations ([dag.rs](../core/src/consensus/dag.rs#L9))
```rust
pub const MIX_ITERATIONS: usize = 64;  // Current
```

**Trade-off**:
- Higher: More memory-hard, slower mining, better ASIC resistance
- Lower: Faster mining, less memory-hard

Recommended: 64 (matching KawPow)

#### 4. DAG Item Size ([dag.rs](../core/src/consensus/dag.rs#L8))
```rust
pub const DAG_ITEM_SIZE: usize = 128;  // Current
```

**Fixed at 128 bytes** - changing breaks consensus!

## Troubleshooting

### Problem: "Failed to upload DAG to GPU: out of memory"
**Cause**: GPU has less than 4GB VRAM

**Solutions**:
1. Use CPU-only mining:
   ```powershell
   cargo build --release
   .\target\release\Astram-node.exe --miner --cpu-only
   ```

2. Close other GPU applications (browsers, games)

3. Reduce system reserved VRAM in BIOS/UEFI

### Problem: Low hashrate on GPU (<10 MH/s)
**Possible causes**:
1. Power limit throttling
   ```powershell
   nvidia-smi -pl 120  # Set power limit to 120W
   ```

2. Temperature throttling
   - Check temperature: `nvidia-smi -q | Select-String "GPU Current Temp"`
   - Improve cooling

3. Wrong GPU selected
   ```powershell
   $env:CUDA_VISIBLE_DEVICES = "0"  # Use GPU 0
   ```

4. Background GPU usage
   - Close Chrome/browsers (GPU acceleration)
   - Close games, video players

### Problem: DAG generation takes >10 minutes
**Cause**: CPU bottleneck or insufficient RAM

**Solutions**:
1. Close other applications (need 4GB+ free RAM)
2. Wait for completion (only happens once per epoch)
3. TODO: Enable DAG caching to disk

### Problem: "GPU hash mismatch"
**Cause**: CUDA kernel computation error

**Debug**:
1. Rebuild with debug symbols:
   ```powershell
   cargo clean
   cargo build --release --features cuda-miner
   ```

2. Check GPU stability:
   ```powershell
   nvidia-smi -q -d MEMORY,CLOCK
   ```

3. Reduce overclock if any

## Performance Comparison

### GTX 1050 Ti (768 cores, 4GB GDDR5)

| Algorithm | Hashrate | Power | Efficiency |
|-----------|----------|-------|------------|
| SHA256d (old) | 40 MH/s | 75W | 0.53 MH/J |
| **Blake3-KawPow (new)** | **~100 MH/s** | 75W | **1.33 MH/J** |

*Estimated - actual performance TBD*

### Expected Scaling

| GPU Model | VRAM | Cores | Est. Hashrate |
|-----------|------|-------|---------------|
| GTX 1050 Ti | 4GB | 768 | 50-100 MH/s |
| GTX 1660 Super | 6GB | 1408 | 100-150 MH/s |
| RTX 3060 Ti | 8GB | 4864 | 200-300 MH/s |
| RTX 3070 | 8GB | 5888 | 250-400 MH/s |
| RTX 4090 | 24GB | 16384 | 600-900 MH/s |

## Profiling Tools

### 1. Nsight Compute (NVIDIA)
```powershell
ncu --target-processes all .\target\release\Astram-node.exe --miner
```

Analyzes:
- Memory bandwidth utilization
- Compute throughput
- Occupancy
- Warp efficiency

### 2. Nsight Systems (NVIDIA)
```powershell
nsys profile .\target\release\Astram-node.exe --miner
```

Shows:
- Timeline of GPU/CPU activity
- Kernel launch overhead
- Memory copy operations

### 3. Built-in Hashrate Monitor
Enabled by default, prints every 5 seconds:
```
Mining hashrate: 85.2 MH/s
Mining hashrate: 87.1 MH/s
```

## Advanced: DAG Caching (TODO)

### Implementation Plan
```rust
// core/src/consensus/dag_cache.rs

pub struct DagCache {
    cache_dir: PathBuf,  // ~/.astram/dag/
}

impl DagCache {
    pub fn load_or_generate(&self, epoch: u64) -> Result<Vec<u8>> {
        let path = self.cache_dir.join(format!("epoch-{}.bin", epoch));
        
        if path.exists() {
            // Load from disk (~1-2 seconds)
            std::fs::read(&path)
        } else {
            // Generate and save
            let dag = generate_full_dag(epoch)?;
            std::fs::write(&path, &dag)?;
            Ok(dag)
        }
    }
}
```

**Benefits**:
- Skip 3-5 minute DAG generation on restart
- Disk space: 4GB per epoch (~20GB for 5 epochs)

## Mining Pool Considerations

### Work Distribution
Pool server must:
1. Generate DAG for current epoch
2. Distribute work with current epoch number
3. Miners verify epoch matches before starting

### Share Submission
```json
{
  "method": "submit",
  "params": {
    "worker": "miner1",
    "job_id": "12345",
    "nonce": "0x1234567890abcdef",
    "hash": "0x0000abcd...",
    "epoch": 0
  }
}
```

### Epoch Transitions
When block 7500 is mined:
1. All miners regenerate DAG (~3-5 min downtime)
2. Pool updates epoch in work template
3. Mining resumes with new DAG

**Mitigation**: Pre-generate next epoch's DAG in background

## Next Steps

1. **Benchmark Real Performance**
   - Test on GTX 1050 Ti
   - Compare with estimates
   - Publish results

2. **Optimize CUDA Kernel**
   - Shared memory for mix state
   - Coalesced DAG accesses
   - Reduce register usage

3. **Implement DAG Caching**
   - Persistent storage
   - Integrity verification
   - Auto-cleanup old epochs

4. **Mining Pool Software**
   - Stratum protocol with DAG support
   - Epoch synchronization
   - Share verification

---

**Last Updated**: February 22, 2026  
**Testing Status**: Integration complete, performance benchmarking pending
