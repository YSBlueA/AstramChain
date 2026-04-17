# Mining Performance Guide

Astram uses the KawPow-Blake3 algorithm — a memory-hard PoW using a 4 GB DAG.  
GPU mining requires an NVIDIA GPU with ≥ 4 GB VRAM and a working CUDA installation.

## Quick Start

### 1. Build the miner

```bash
# Linux (recommended — uses build script)
./build-release.sh

# Manual build
cargo build --release -p Astram-miner --features cuda-miner
```

### 2. Configure mining mode

Edit `config/minerSettings.conf`:

**Solo mining** (mine directly against your local node):
```ini
MINING_MODE=solo
NODE_RPC_URL=http://127.0.0.1:19533
STATUS_PORT=8090
```

**Pool mining** (connect to a Stratum pool):
```ini
MINING_MODE=pool
POOL_HOST=127.0.0.1
POOL_PORT=3333
WORKER_NAME=worker1
STATUS_PORT=8090
```

### 3. Start the node first (solo mode)

```bash
./release/linux/Astram.sh node
```

### 4. Start the miner

```bash
./release/linux/Astram.sh miner
```

Or run the binary directly:

```bash
./target/release/Astram-miner
```

### Expected Output

```
[INFO] Astram miner starting...
[INFO] Mode: Solo
[SOLO] Starting solo miner → node: http://127.0.0.1:19533
[SOLO] Miner address: 0xabc123...
[DAG] Generating 4 GB DAG for epoch 0... (this takes 3–5 minutes)
[DAG] Progress: 10%
...
[DAG] Progress: 100%
[CUDA] DAG uploaded to GPU, starting mining...
[SOLO] Mining block #1234 | diff=0x1e0fffff | txs=3 | reward=8000000000000000000 wei
Mining hashrate: 85.2 MH/s
```

The miner status dashboard is available at `http://localhost:8090`.

---

## Performance Benchmarking

### DAG Generation Speed

DAG generation is parallelized with rayon. Expected time: **3–5 minutes** on a modern desktop.

Monitor progress via log output or the status dashboard.  
The DAG regenerates once per epoch (every 7,500 blocks, ~10.4 days at 120 s/block).

### Expected Hashrates

| GPU | VRAM | Est. Hashrate |
|-----|------|---------------|
| GTX 1050 Ti | 4 GB | 50–100 MH/s |
| GTX 1660 Super | 6 GB | 100–150 MH/s |
| RTX 3060 Ti | 8 GB | 200–300 MH/s |
| RTX 3070 | 8 GB | 250–400 MH/s |
| RTX 4090 | 24 GB | 600–900 MH/s |

*Estimated — actual performance will vary. Benchmarking results pending.*

---

## Optimization Tuning

### Environment Variables

#### `CUDA_BATCH_SIZE`
Number of hashes per GPU kernel call (default: 16 M):

```bash
CUDA_BATCH_SIZE=33554432 ./target/release/Astram-miner   # 32 M hashes
```

| GPU | Recommended |
|-----|-------------|
| GTX 1050 Ti | 16 M (default) |
| RTX 3070 | 32 M |
| RTX 4090 | 64 M |

Larger batch = better GPU utilization, slower cancel response.  
Smaller batch = faster new-job response, slightly lower hashrate.

#### `CUDA_VISIBLE_DEVICES`
Select a specific GPU:

```bash
CUDA_VISIBLE_DEVICES=0 ./target/release/Astram-miner
```

### Code-Level Tuning

These constants are in [core/src/consensus/cuda.rs](../core/src/consensus/cuda.rs):

#### Threads Per Block (default: 512)

```rust
const THREADS_PER_BLOCK: u32 = 512;
```

| Compute capability | Recommended |
|--------------------|-------------|
| < 3.0 (older GPUs) | 128 |
| GTX 900–1000 series | 256 |
| GTX 1050 Ti – RTX 2000 | 512 |
| RTX 3000–4000 series | 1024 |

#### Max Blocks (default: 8192)

```rust
const MAX_BLOCKS: u32 = 8192;
```

Formula: `MAX_BLOCKS = (CUDA_cores / THREADS_PER_BLOCK) × 4`

| GPU | Recommended |
|-----|-------------|
| GTX 1050 Ti (768 cores) | 4096–8192 |
| RTX 3070 (5888 cores) | 16384 |
| RTX 4090 (16384 cores) | 32768 |

#### Mix Iterations (default: 64)

Defined in [core/src/consensus/dag.rs](../core/src/consensus/dag.rs):

```rust
pub const MIX_ITERATIONS: usize = 64;
```

Changing this value **breaks consensus** — do not modify for mainnet.

---

## Troubleshooting

### "Failed to upload DAG to GPU: out of memory"

**Cause**: GPU has < 4 GB VRAM available.

**Solutions**:
1. Close Chrome/browsers (they use GPU memory for acceleration).
2. Close other GPU-accelerated applications.
3. Check available VRAM: `nvidia-smi`
4. Use a GPU with ≥ 4 GB VRAM.

### Low hashrate (< 10 MH/s)

**Cause**: Throttling or wrong GPU selected.

1. Check for power-limit throttling:
   ```bash
   nvidia-smi -pl 120    # Set power limit to 120 W
   ```
2. Check temperature: `nvidia-smi -q | grep "GPU Current Temp"`
3. Force a specific GPU: `CUDA_VISIBLE_DEVICES=0`
4. Close background GPU processes (browsers, video).

### DAG generation takes > 10 minutes

**Cause**: Insufficient free RAM or CPU bottleneck.

1. Ensure ≥ 4 GB free RAM during generation.
2. Close other applications.
3. Wait — generation runs only once per epoch.

### Miner can't connect to node (solo mode)

1. Check the node is running: `curl http://127.0.0.1:19533/health`
2. Verify `NODE_RPC_URL` in `config/minerSettings.conf`.
3. Check node logs: `<DATA_DIR>/logs/node_*.log`

### Miner can't connect to pool

1. Verify `POOL_HOST` and `POOL_PORT` in `config/minerSettings.conf`.
2. Ensure `astram-stratum` is running.
3. Check pool logs and the node it is connected to.

---

## Profiling Tools

### Nsight Compute

```bash
ncu --target-processes all ./target/release/Astram-miner
```

Analyzes memory bandwidth utilization, compute throughput, occupancy, and warp efficiency.

### Nsight Systems

```bash
nsys profile ./target/release/Astram-miner
```

Shows timeline of GPU/host activity, kernel launch overhead, and memory copy operations.

### Built-in Hashrate Monitor

Enabled by default — logs every ~5 seconds and updates the status dashboard at `http://localhost:8090`.

---

## Pool Mining Setup

### Start the Stratum pool server

Edit `config/pool.conf`, then:

```bash
./release/linux/Astram.sh pool    # if included in release script
# or
./target/release/astram-stratum
```

The pool connects to the node at `http://127.0.0.1:19533` by default.

### VarDiff

The pool adjusts each miner's difficulty dynamically to target ~15 s between shares:
- Min difficulty: 1
- Max difficulty: 32

### PPLNS Payout

Payouts use PPLNS (Pay Per Last N Shares), default window 10,000 shares.  
Minimum payout threshold: 0.5 ASRM (configurable).

### Epoch Transitions

When epoch boundary (every 7,500 blocks) is crossed:

1. All miners regenerate their DAG (~3–5 min downtime per miner).
2. The pool updates the epoch in the work template.
3. Mining resumes with the new DAG.

**Mitigation**: Pre-generate the next epoch's DAG in the background before the boundary is reached.

---

## Performance Comparison

| Algorithm | Memory | ASIC Resistance | Est. GPU Hashrate |
|-----------|--------|-----------------|-------------------|
| SHA256d (Bitcoin) | 0 | None | 40 MH/s |
| Scrypt (Litecoin) | 128 KB | Low | 500 KH/s |
| Ethash (Ethereum) | 4 GB | High | 30 MH/s |
| KawPow (Ravencoin) | 4 GB | High | 25 MH/s |
| **Blake3-KawPow (Astram)** | **4 GB** | **High** | **50–150 MH/s** |

Blake3 is faster than SHA-256 in software and maps well to CUDA SIMD execution.

---

## Future Optimizations

1. **DAG Disk Caching**: Save DAG to `~/.Astram/dag/epoch-N.bin`; skip 3–5 min regeneration on restart. (~4 GB per epoch, 20 GB for 5 epochs.)
2. **Light Client Verification**: Verify PoW without holding the full DAG.
3. **Adaptive Batch Size**: Auto-tune `CUDA_BATCH_SIZE` for the connected GPU.
4. **Shared Memory Optimization**: Reduce register pressure in the CUDA kernel.
5. **Coalesced DAG Access**: Improve memory access patterns for better bandwidth utilization.

---

**Last Updated**: April 2026  
**Status**: Functional — performance benchmarking in progress
