#include <stdint.h>

#define SHA256_BLOCK_SIZE 64

__device__ __forceinline__ uint32_t rotr(uint32_t x, uint32_t n) {
    return (x >> n) | (x << (32 - n));
}

__device__ __forceinline__ uint32_t ch(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (~x & z);
}

__device__ __forceinline__ uint32_t maj(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (x & z) ^ (y & z);
}

__device__ __forceinline__ uint32_t bsig0(uint32_t x) {
    return rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22);
}

__device__ __forceinline__ uint32_t bsig1(uint32_t x) {
    return rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25);
}

__device__ __forceinline__ uint32_t ssig0(uint32_t x) {
    return rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3);
}

__device__ __forceinline__ uint32_t ssig1(uint32_t x) {
    return rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10);
}

__device__ void sha256(const uint8_t* data, int len, uint8_t out[32]) {
    static __device__ const uint32_t k[64] = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
        0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
        0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
    };

    uint8_t buf[256];
    int total = len + 1 + 8;
    int padded = ((total + 63) / 64) * 64;

    for (int i = 0; i < len; i++) {
        buf[i] = data[i];
    }
    buf[len] = 0x80;
    for (int i = len + 1; i < padded; i++) {
        buf[i] = 0x00;
    }

    uint64_t bit_len = (uint64_t)len * 8ULL;
    for (int i = 0; i < 8; i++) {
        buf[padded - 1 - i] = (uint8_t)(bit_len >> (8 * i));
    }

    uint32_t h0 = 0x6a09e667;
    uint32_t h1 = 0xbb67ae85;
    uint32_t h2 = 0x3c6ef372;
    uint32_t h3 = 0xa54ff53a;
    uint32_t h4 = 0x510e527f;
    uint32_t h5 = 0x9b05688c;
    uint32_t h6 = 0x1f83d9ab;
    uint32_t h7 = 0x5be0cd19;

    for (int offset = 0; offset < padded; offset += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++) {
            int idx = offset + i * 4;
            w[i] = ((uint32_t)buf[idx] << 24) |
                   ((uint32_t)buf[idx + 1] << 16) |
                   ((uint32_t)buf[idx + 2] << 8) |
                   ((uint32_t)buf[idx + 3]);
        }
        for (int i = 16; i < 64; i++) {
            w[i] = ssig1(w[i - 2]) + w[i - 7] + ssig0(w[i - 15]) + w[i - 16];
        }

        uint32_t a = h0;
        uint32_t b = h1;
        uint32_t c = h2;
        uint32_t d = h3;
        uint32_t e = h4;
        uint32_t f = h5;
        uint32_t g = h6;
        uint32_t h = h7;

        for (int i = 0; i < 64; i++) {
            uint32_t t1 = h + bsig1(e) + ch(e, f, g) + k[i] + w[i];
            uint32_t t2 = bsig0(a) + maj(a, b, c);
            h = g;
            g = f;
            f = e;
            e = d + t1;
            d = c;
            c = b;
            b = a;
            a = t1 + t2;
        }

        h0 += a;
        h1 += b;
        h2 += c;
        h3 += d;
        h4 += e;
        h5 += f;
        h6 += g;
        h7 += h;
    }

    uint32_t hs[8] = {h0, h1, h2, h3, h4, h5, h6, h7};
    for (int i = 0; i < 8; i++) {
        out[i * 4 + 0] = (uint8_t)(hs[i] >> 24);
        out[i * 4 + 1] = (uint8_t)(hs[i] >> 16);
        out[i * 4 + 2] = (uint8_t)(hs[i] >> 8);
        out[i * 4 + 3] = (uint8_t)(hs[i]);
    }
}

__device__ void sha256d(const uint8_t* data, int len, uint8_t out[32]) {
    uint8_t tmp[32];
    sha256(data, len, tmp);
    sha256(tmp, 32, out);
}

__device__ __forceinline__ int meets_target(const uint8_t hash[32], int difficulty) {
    if (difficulty <= 0) {
        return 1;
    }

    int full_bytes = difficulty / 2;
    int half = difficulty % 2;

    for (int i = 0; i < full_bytes; i++) {
        if (hash[i] != 0) {
            return 0;
        }
    }

    if (half) {
        if ((hash[full_bytes] & 0xF0) != 0) {
            return 0;
        }
    }

    return 1;
}

extern "C" __global__ void mine_kernel(
    const uint8_t* prefix,
    int prefix_len,
    const uint8_t* suffix,
    int suffix_len,
    uint64_t start_nonce,
    uint64_t total,
    int difficulty,
    unsigned int* found_flag,
    uint64_t* found_nonce,
    uint8_t* found_hash
) {
    uint64_t idx = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)blockDim.x * gridDim.x;

    for (uint64_t i = idx; i < total; i += stride) {
        if (atomicAdd(found_flag, 0) != 0) {
            return;
        }

        uint64_t nonce = start_nonce + i;
        uint8_t msg[192];
        int len = 0;

        for (int j = 0; j < prefix_len; j++) {
            msg[len++] = prefix[j];
        }

        uint64_t n = nonce;
        do {
            uint8_t byte = (uint8_t)(n & 0x7F);
            n >>= 7;
            if (n != 0) {
                byte |= 0x80;
            }
            msg[len++] = byte;
        } while (n != 0);

        for (int j = 0; j < suffix_len; j++) {
            msg[len++] = suffix[j];
        }

        uint8_t hash[32];
        sha256d(msg, len, hash);

        if (meets_target(hash, difficulty)) {
            if (atomicCAS(found_flag, 0, 1) == 0) {
                *found_nonce = nonce;
                for (int j = 0; j < 32; j++) {
                    found_hash[j] = hash[j];
                }
            }
            return;
        }
    }
}
