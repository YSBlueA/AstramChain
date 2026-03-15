#include <stdint.h>

// Blake3 constants and helpers
#define BLAKE3_OUT_LEN 32
#define BLAKE3_KEY_LEN 32
#define BLAKE3_BLOCK_LEN 64
#define BLAKE3_CHUNK_LEN 1024

// Blake3 IV (Initialization Vector)
__constant__ uint32_t BLAKE3_IV[8] = {
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19};

// Rotation helpers for Blake3
__device__ __forceinline__ uint32_t rotr32(uint32_t x, uint32_t n)
{
    return (x >> n) | (x << (32 - n));
}

// Blake3 G function (quarter-round)
__device__ __forceinline__ void g(uint32_t *state, int a, int b, int c, int d, uint32_t mx, uint32_t my)
{
    state[a] = state[a] + state[b] + mx;
    state[d] = rotr32(state[d] ^ state[a], 16);
    state[c] = state[c] + state[d];
    state[b] = rotr32(state[b] ^ state[c], 12);
    state[a] = state[a] + state[b] + my;
    state[d] = rotr32(state[d] ^ state[a], 8);
    state[c] = state[c] + state[d];
    state[b] = rotr32(state[b] ^ state[c], 7);
}

// Blake3 round function
__device__ __forceinline__ void round_fn(uint32_t state[16], const uint32_t *m)
{
    // Mix columns
    g(state, 0, 4, 8, 12, m[0], m[1]);
    g(state, 1, 5, 9, 13, m[2], m[3]);
    g(state, 2, 6, 10, 14, m[4], m[5]);
    g(state, 3, 7, 11, 15, m[6], m[7]);
    // Mix diagonals
    g(state, 0, 5, 10, 15, m[8], m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8, 13, m[12], m[13]);
    g(state, 3, 4, 9, 14, m[14], m[15]);
}

// Blake3 permutation (7 rounds)
__device__ void blake3_compress(uint32_t cv[8], const uint8_t block[BLAKE3_BLOCK_LEN],
                                uint8_t block_len, uint64_t counter, uint8_t flags)
{
    uint32_t state[16];

// Initialize state with chaining value and IV
#pragma unroll
    for (int i = 0; i < 8; i++)
    {
        state[i] = cv[i];
        state[i + 8] = BLAKE3_IV[i];
    }

    state[12] = (uint32_t)counter;
    state[13] = (uint32_t)(counter >> 32);
    state[14] = (uint32_t)block_len;
    state[15] = (uint32_t)flags;

    // Load message block (little-endian)
    uint32_t m[16];
#pragma unroll
    for (int i = 0; i < 16; i++)
    {
        m[i] = ((uint32_t)block[i * 4 + 0]) |
               ((uint32_t)block[i * 4 + 1] << 8) |
               ((uint32_t)block[i * 4 + 2] << 16) |
               ((uint32_t)block[i * 4 + 3] << 24);
    }

// 7 rounds
#pragma unroll
    for (int r = 0; r < 7; r++)
    {
        round_fn(state, m);
        // Permute message words for next round (Blake3 permutation)
        uint32_t tmp[16];
        tmp[0] = m[2];
        tmp[1] = m[6];
        tmp[2] = m[3];
        tmp[3] = m[10];
        tmp[4] = m[7];
        tmp[5] = m[0];
        tmp[6] = m[4];
        tmp[7] = m[13];
        tmp[8] = m[1];
        tmp[9] = m[11];
        tmp[10] = m[12];
        tmp[11] = m[5];
        tmp[12] = m[9];
        tmp[13] = m[14];
        tmp[14] = m[15];
        tmp[15] = m[8];
#pragma unroll
        for (int i = 0; i < 16; i++)
            m[i] = tmp[i];
    }

// XOR state with chaining value
#pragma unroll
    for (int i = 0; i < 8; i++)
    {
        cv[i] = state[i] ^ state[i + 8];
    }
}

// Blake3 flags per spec
#define BLAKE3_FLAG_CHUNK_START 0x01
#define BLAKE3_FLAG_CHUNK_END   0x02
#define BLAKE3_FLAG_ROOT        0x08

// Blake3 hash for mining (single-chunk input, ≤1024 bytes).
// Flags follow the Blake3 reference spec:
//   - first block of chunk: CHUNK_START
//   - last block of chunk:  CHUNK_END | ROOT  (ROOT because this IS the only chunk)
//   - only block (≤64 B):   CHUNK_START | CHUNK_END | ROOT
//
// IMPORTANT: All blocks within a single chunk use counter = 0 (the chunk index).
// Incrementing the counter per block is wrong — the counter tracks chunks, not
// blocks within a chunk. For single-chunk inputs this is always 0.
__device__ void blake3_hash_simple(const uint8_t *input, int len, uint8_t output[32])
{
    uint32_t cv[8];
    for (int i = 0; i < 8; i++)
        cv[i] = BLAKE3_IV[i];

    uint8_t block[BLAKE3_BLOCK_LEN];
    int processed = 0;
    int is_first = 1;

    // Process every block except the last one (strict <).
    // This guarantees at least one byte reaches the "final block" path.
    while (processed + BLAKE3_BLOCK_LEN < len)
    {
        for (int i = 0; i < BLAKE3_BLOCK_LEN; i++)
            block[i] = input[processed + i];

        uint8_t flags = is_first ? BLAKE3_FLAG_CHUNK_START : 0;
        // counter = 0: this is chunk 0; all blocks within a chunk share the same counter.
        blake3_compress(cv, block, BLAKE3_BLOCK_LEN, 0, flags);
        processed += BLAKE3_BLOCK_LEN;
        is_first = 0;
    }

    // Final block (may be a full 64-byte block if input length is a multiple of 64).
    int remaining = len - processed;
    for (int i = 0; i < BLAKE3_BLOCK_LEN; i++)
        block[i] = (i < remaining) ? input[processed + i] : 0;

    uint8_t final_flags = BLAKE3_FLAG_CHUNK_END | BLAKE3_FLAG_ROOT;
    if (is_first)
        final_flags |= BLAKE3_FLAG_CHUNK_START;  // single block: also CHUNK_START

    // counter = 0: chunk 0, consistent with all blocks above
    blake3_compress(cv, block, (uint8_t)remaining, 0, final_flags);

    // Extract output (little-endian word order, matching blake3 Rust crate)
    for (int i = 0; i < 8; i++)
    {
        output[i * 4 + 0] = (uint8_t)(cv[i]);
        output[i * 4 + 1] = (uint8_t)(cv[i] >> 8);
        output[i * 4 + 2] = (uint8_t)(cv[i] >> 16);
        output[i * 4 + 3] = (uint8_t)(cv[i] >> 24);
    }
}

__device__ __forceinline__ int meets_target(const uint8_t hash[32], const uint8_t target[32])
{
#pragma unroll
    for (int i = 0; i < 32; i++)
    {
        if (hash[i] < target[i])
        {
            return 1;
        }
        if (hash[i] > target[i])
        {
            return 0;
        }
    }
    return 1; // equal
}

extern "C" __global__ void mine_kernel(
    const uint8_t *prefix,
    int prefix_len,
    const uint8_t *suffix,
    int suffix_len,
    uint64_t start_nonce,
    uint64_t total,
    const uint8_t *target,
    unsigned int *found_flag,
    uint64_t *found_nonce,
    uint8_t *found_hash,
    const uint8_t *dag,
    uint64_t dag_size)
{
    uint64_t idx = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)blockDim.x * gridDim.x;

    const int DAG_ITEM_SIZE = 128;
    const int MIX_ITERATIONS = 32; // Increased from 16 for better security
    const uint64_t DAG_ITEM_COUNT = dag_size / DAG_ITEM_SIZE;

    for (uint64_t i = idx; i < total; i += stride)
    {
        if (atomicAdd(found_flag, 0) != 0)
        {
            return;
        }

        uint64_t nonce = start_nonce + i;

        // Reduce stack usage by using smaller temporary buffers
        // Step 1: Initial Blake3 hash of header (reuse msg buffer)
        uint8_t msg[192]; // Shared buffer for multiple purposes
        int len = 0;

        // Build header: prefix + nonce + suffix
        for (int j = 0; j < prefix_len; j++)
        {
            msg[len++] = prefix[j];
        }

        // Fixed-length encoding: u64 = 8 bytes little-endian
        for (int j = 0; j < 8; j++)
        {
            msg[len++] = (uint8_t)(nonce >> (8 * j));
        }

        for (int j = 0; j < suffix_len; j++)
        {
            msg[len++] = suffix[j];
        }

        uint8_t header_hash[32];
        blake3_hash_simple(msg, len, header_hash);

        // Step 2: DAG seed from header + nonce (reuse msg buffer)
        for (int j = 0; j < 32; j++)
        {
            msg[j] = header_hash[j];
        }
        for (int j = 0; j < 8; j++)
        {
            msg[32 + j] = (uint8_t)(nonce >> (8 * j));
        }

        uint8_t seed[32];
        blake3_hash_simple(msg, 40, seed);

        // Step 3: Memory-hard mixing with DAG
        uint8_t mix[128];
        for (int j = 0; j < 32; j++)
        {
            mix[j] = seed[j];
        }

        // Expand seed to 128 bytes (reuse msg buffer for expand_input)
        for (int expand_i = 1; expand_i < 4; expand_i++)
        {
            for (int j = 0; j < 32; j++)
            {
                msg[j] = mix[j];
            }
            msg[32] = (uint8_t)expand_i;
            msg[33] = 0;
            msg[34] = 0;
            msg[35] = 0;

            uint8_t expanded[32];
            blake3_hash_simple(msg, 36, expanded);

            int start_idx = expand_i * 32;
            for (int j = 0; j < 32 && (start_idx + j) < 128; j++)
            {
                mix[start_idx + j] = expanded[j];
            }
        }

        // Perform random DAG accesses (memory-hard)
        for (int iteration = 0; iteration < MIX_ITERATIONS; iteration++)
        {
            // Compute DAG index from current mix state
            uint32_t index_offset = (iteration % 4) * 32;
            uint32_t dag_index_raw =
                ((uint32_t)mix[index_offset + 0]) |
                ((uint32_t)mix[index_offset + 1] << 8) |
                ((uint32_t)mix[index_offset + 2] << 16) |
                ((uint32_t)mix[index_offset + 3] << 24);

            uint64_t dag_index = ((uint64_t)dag_index_raw) % DAG_ITEM_COUNT;
            uint64_t dag_offset = dag_index * DAG_ITEM_SIZE;

            // Fetch DAG item from global memory (4GB dataset!)
            // This is the memory-hard part - requires 4GB VRAM
            for (int j = 0; j < DAG_ITEM_SIZE; j++)
            {
                mix[j] ^= dag[dag_offset + j];
            }

            // Hash the mix for next iteration
            uint8_t mixed[32];
            blake3_hash_simple(mix, 128, mixed);
            for (int j = 0; j < 32; j++)
            {
                mix[j] = mixed[j];
            }
        }

        // Step 4: Final Blake3 DAG-PoW hash (kept for future node-side validation)
        uint8_t final_hash[32];
        blake3_hash_simple(mix, 128, final_hash);

        // Check if canonical header hash (Blake3) meets target.
        // Pool and node both validate using the canonical Blake3 header hash,
        // so shares/blocks are accepted when header_hash < target.
        if (meets_target(header_hash, target))
        {
            if (atomicCAS(found_flag, 0, 1) == 0)
            {
                *found_nonce = nonce;
                for (int j = 0; j < 32; j++)
                {
                    found_hash[j] = header_hash[j];
                }
            }
            return;
        }
    }
}
