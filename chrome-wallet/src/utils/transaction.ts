/**
 * AstramChain transaction builder, signer, and broadcaster.
 *
 * Replicates the Rust wallet-cli logic:
 *  1. Fetch UTXOs  →  select coins
 *  2. Build transaction (bincode)
 *  3. SHA-256 the serialized-for-hash payload
 *  4. Sign with Ed25519 (tweetnacl)
 *  5. POST binary to /tx
 *
 * Bincode "standard" wire format (all LE):
 *   u8 / u32 / u64 / i64 → fixed-width little-endian
 *   String   → u64 len + UTF-8 bytes
 *   Vec<T>   → u64 len + elements
 *   Option<T>→ u8 (0=None, 1=Some) [+ value]
 *   Tuple    → fields concatenated
 */

import nacl from 'tweetnacl'
import sha256 from 'sha256'
import axios from 'axios'

// ─── Fee constants (mirror core/src/config.rs) ─────────────────────────────
const BASE_MIN_FEE = 100_000_000_000_000n          // 100 Twei
const DEFAULT_FEE_PER_BYTE = 300_000_000_000n       // 300 Gwei / byte

function calculateFee(sizeBytes: number): bigint {
  return BASE_MIN_FEE + DEFAULT_FEE_PER_BYTE * BigInt(sizeBytes)
}

// ─── Bincode writer ─────────────────────────────────────────────────────────
// Implements bincode v2 `config::standard()` = varint integer encoding.
// Varint thresholds: ≤250 → 1 byte; ≤0xFFFF → [251,lo,hi];
//   ≤0xFFFFFFFF → [252,b0-b3]; else → [253,b0-b7]
class BincodeWriter {
  private buf: number[] = []

  u8(v: number) { this.buf.push(v & 0xff) }

  // Varint-encoded unsigned integer (bincode v2 standard)
  private varUint(v: bigint) {
    if (v <= 250n) {
      this.buf.push(Number(v))
    } else if (v <= 0xffffn) {
      this.buf.push(251, Number(v & 0xffn), Number((v >> 8n) & 0xffn))
    } else if (v <= 0xffff_ffffn) {
      this.buf.push(252)
      for (let i = 0; i < 4; i++) { this.buf.push(Number((v >> BigInt(i * 8)) & 0xffn)) }
    } else {
      this.buf.push(253)
      for (let i = 0; i < 8; i++) { this.buf.push(Number((v >> BigInt(i * 8)) & 0xffn)) }
    }
  }

  u32(v: number) { this.varUint(BigInt(v)) }
  u64(v: bigint) { this.varUint(v) }

  // i64 → zigzag encode → varUint  (bincode v2 standard for signed integers)
  i64(v: number) {
    const n = BigInt(v)
    const zigzag = BigInt.asUintN(64, (n << 1n) ^ (n >> 63n))
    this.varUint(zigzag)
  }

  str(s: string) {
    const bytes = new TextEncoder().encode(s)
    this.varUint(BigInt(bytes.length))
    bytes.forEach(b => this.buf.push(b))
  }

  optStr(s: string | null) {
    if (s === null) { this.u8(0) } else { this.u8(1); this.str(s) }
  }

  bytes(): Uint8Array { return new Uint8Array(this.buf) }
  size(): number { return this.buf.length }
}

// ─── U256 helpers ───────────────────────────────────────────────────────────
/** Convert BigInt amount (wei) → [u64; 4] little-endian word array */
function amountToWords(amount: bigint): [bigint, bigint, bigint, bigint] {
  const mask = (1n << 64n) - 1n
  return [
    amount & mask,
    (amount >> 64n) & mask,
    (amount >> 128n) & mask,
    (amount >> 192n) & mask,
  ]
}

/** Convert [u64; 4] JSON array from API → BigInt */
export function wordsToAmount(words: number[]): bigint {
  return (
    BigInt(words[0]) |
    (BigInt(words[1]) << 64n) |
    (BigInt(words[2]) << 128n) |
    (BigInt(words[3]) << 192n)
  )
}

// ─── Bincode serialization helpers ─────────────────────────────────────────

interface TxInput {
  txid: string
  vout: number
  pubkey: string
  signature: string | null
}

interface TxOutput {
  to: string
  amount: bigint
}

/**
 * Mirrors Transaction::serialize_for_hash():
 *   bincode( (Vec<(String, u32)>, Vec<TransactionOutput>, i64) )
 */
function serializeForHash(
  inputs: TxInput[],
  outputs: TxOutput[],
  timestamp: number,
): Uint8Array {
  const w = new BincodeWriter()

  // Vec<(String, u32)>
  w.u64(BigInt(inputs.length))
  for (const inp of inputs) {
    w.str(inp.txid)
    w.u32(inp.vout)
  }

  // Vec<TransactionOutput>
  w.u64(BigInt(outputs.length))
  for (const out of outputs) {
    w.str(out.to)
    const words = amountToWords(out.amount)
    for (const word of words) w.u64(word)
  }

  // i64 timestamp
  w.i64(timestamp)

  return w.bytes()
}

/**
 * Serializes the complete Transaction struct for broadcast.
 */
function serializeTransaction(
  txid: string,
  inputs: TxInput[],
  outputs: TxOutput[],
  timestamp: number,
): Uint8Array {
  const w = new BincodeWriter()

  // txid: String
  w.str(txid)

  // inputs: Vec<TransactionInput>
  w.u64(BigInt(inputs.length))
  for (const inp of inputs) {
    w.str(inp.txid)
    w.u32(inp.vout)
    w.str(inp.pubkey)
    w.optStr(inp.signature)
  }

  // outputs: Vec<TransactionOutput>
  w.u64(BigInt(outputs.length))
  for (const out of outputs) {
    w.str(out.to)
    const words = amountToWords(out.amount)
    for (const word of words) w.u64(word)
  }

  // timestamp: i64
  w.i64(timestamp)

  return w.bytes()
}

/**
 * Compute txid: SHA256(SHA256(serialize_for_hash()))
 */
function computeTxid(inputs: TxInput[], outputs: TxOutput[], timestamp: number): string {
  const hashBytes = serializeForHash(inputs, outputs, timestamp)
  const h1 = sha256(Array.from(hashBytes), { asBytes: true }) as number[]
  const h2 = sha256(h1, { asBytes: true }) as number[]
  return h2.map(b => b.toString(16).padStart(2, '0')).join('')
}

// ─── Main send function ─────────────────────────────────────────────────────

export interface SendResult {
  success: boolean
  txid?: string
  fee?: string  // in ASRM
  error?: string
}

export async function sendTransaction(
  rpcUrl: string,
  fromAddress: string,
  privateKeyHex: string,   // "0x..." or raw hex, 32 bytes
  toAddress: string,
  amountAsrm: number,
): Promise<SendResult> {
  const amountRam = BigInt(Math.round(amountAsrm * 1e18))

  // ── 1. Derive keypair from seed ──────────────────────────────────────────
  const privHex = privateKeyHex.startsWith('0x')
    ? privateKeyHex.slice(2)
    : privateKeyHex
  const seed = Uint8Array.from(Buffer.from(privHex, 'hex'))
  const keypair = nacl.sign.keyPair.fromSeed(seed)
  const pubkeyHex = Buffer.from(keypair.publicKey).toString('hex')

  // ── 2. Fetch UTXOs ───────────────────────────────────────────────────────
  let utxos: Array<{ txid: string; vout: number; amount: number[] }>
  try {
    const res = await axios.get(`${rpcUrl}/address/${fromAddress}/utxos`)
    utxos = res.data
  } catch (e: any) {
    return { success: false, error: `Failed to fetch UTXOs: ${e.message}` }
  }

  if (!utxos || utxos.length === 0) {
    return { success: false, error: '잔액이 없습니다 (UTXO 없음)' }
  }

  // Build (input, amount) pool
  const inputPool: Array<{ input: TxInput; amount: bigint }> = utxos.map(u => ({
    input: { txid: u.txid, vout: u.vout, pubkey: fromAddress, signature: null },
    amount: wordsToAmount(u.amount),
  }))

  // ── 3. Iterate fee convergence (mirrors CLI) ─────────────────────────────
  let fee = 0n
  let selectedInputs: TxInput[] = []
  let inputSum = 0n
  let cursor = 0

  for (let iter = 0; iter < 16; iter++) {
    // Select UTXOs until amount + fee is covered
    while (inputSum < amountRam + fee) {
      if (cursor >= inputPool.length) {
        const have = (Number(inputSum) / 1e18).toFixed(6)
        const need = (Number(amountRam + fee) / 1e18).toFixed(6)
        return {
          success: false,
          error: `잔액 부족: ${have} ASRM 보유, ${need} ASRM 필요 (수수료 포함)`,
        }
      }
      const { input, amount } = inputPool[cursor++]
      selectedInputs.push({ ...input })
      inputSum += amount
    }

    const change = inputSum - amountRam - fee
    const outputs: TxOutput[] = [{ to: toAddress.toLowerCase(), amount: amountRam }]
    if (change > 0n) {
      outputs.push({ to: fromAddress.toLowerCase(), amount: change })
    }

    const timestamp = Math.floor(Date.now() / 1000)

    // Sign
    const hashPayload = serializeForHash(selectedInputs, outputs, timestamp)
    const msgHash = Uint8Array.from(
      sha256(Array.from(hashPayload), { asBytes: true }) as number[],
    )
    const sigBytes = nacl.sign.detached(msgHash, keypair.secretKey)
    const sigHex = Buffer.from(sigBytes).toString('hex')

    const signedInputs = selectedInputs.map(inp => ({
      ...inp,
      pubkey: pubkeyHex,
      signature: sigHex,
    }))

    const txid = computeTxid(selectedInputs, outputs, timestamp)
    const txBytes = serializeTransaction(txid, signedInputs, outputs, timestamp)

    const newFee = calculateFee(txBytes.length)
    if (newFee > fee) {
      fee = newFee
      // reset selections and retry
      selectedInputs = []
      inputSum = 0n
      cursor = 0
      continue
    }

    // ── 4. Broadcast ──────────────────────────────────────────────────────
    try {
      const res = await axios.post(`${rpcUrl}/tx`, txBytes, {
        headers: { 'Content-Type': 'application/octet-stream' },
        responseType: 'json',
      })
      if (res.data?.status === 'ok') {
        const feeAsrm = (Number(fee) / 1e18).toFixed(8)
        return { success: true, txid, fee: feeAsrm }
      }
      return {
        success: false,
        error: res.data?.message || '트랜잭션 전송 실패',
      }
    } catch (e: any) {
      const msg = e.response?.data?.message || e.message
      return { success: false, error: `전송 실패: ${msg}` }
    }
  }

  return { success: false, error: '수수료 계산 실패 (수렴 오류)' }
}
