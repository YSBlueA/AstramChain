import nacl from 'tweetnacl'
import * as bip39 from 'bip39'
import sha256 from 'sha256'
import { Buffer } from 'buffer'

// Make Buffer available globally for bip39
if (typeof window !== 'undefined') {
  (window as any).Buffer = Buffer
}

/**
 * Ed25519 지갑 생성 (Node과 호환)
 */
export function generateEd25519Wallet(): {
  address: string
  privateKey: string
  mnemonic: string
} {
  // 24개 단어 mnemonic 생성
  const mnemonic = bip39.generateMnemonic(256)

  // Mnemonic으로부터 seed 생성 (256비트 = 32바이트)
  const seed = bip39.mnemonicToSeedSync(mnemonic, '')
  
  // 처음 32바이트를 Ed25519 시드로 사용
  const seedBytes = seed.slice(0, 32)

  // TweetNaCl의 Ed25519 키 생성
  const keyPair = nacl.sign.keyPair.fromSeed(seedBytes)

  // Private key (32바이트)
  const privateKey = '0x' + Buffer.from(keyPair.secretKey.slice(0, 32)).toString('hex')

  // 공개키 SHA256 해시 → 주소 (처음 20바이트 = 40글자)
  const publicKeyHash = sha256(Buffer.from(keyPair.publicKey), { asBytes: false })
  const address = '0x' + publicKeyHash.slice(0, 40)

  return {
    address,
    privateKey,
    mnemonic,
  }
}

/**
 * Mnemonic으로부터 Ed25519 지갑 복원 (Node과 호환)
 */
export function createWalletFromEd25519Mnemonic(mnemonic: string): {
  address: string
  privateKey: string
} {
  // Mnemonic 유효성 검증
  if (!bip39.validateMnemonic(mnemonic)) {
    throw new Error('Invalid recovery phrase')
  }

  // Mnemonic 정규화
  const normalizedMnemonic = mnemonic
    .trim()
    .toLowerCase()
    .replace(/\s+/g, ' ')

  // Seed 생성
  const seed = bip39.mnemonicToSeedSync(normalizedMnemonic, '')
  const seedBytes = seed.slice(0, 32)

  // Ed25519 키 쌍 생성
  const keyPair = nacl.sign.keyPair.fromSeed(seedBytes)

  // Private key
  const privateKey = '0x' + Buffer.from(keyPair.secretKey.slice(0, 32)).toString('hex')

  // 주소 생성 (SHA256 해시의 처음 20바이트)
  const publicKeyHash = sha256(Buffer.from(keyPair.publicKey), { asBytes: false })
  const address = '0x' + publicKeyHash.slice(0, 40)

  return {
    address,
    privateKey,
  }
}

/**
 * Private key로부터 주소 도출 (Ed25519)
 */
export function addressFromPrivateKey(privateKey: string): string {
  // Private key에서 0x 제거
  const privKeyHex = privateKey.startsWith('0x') ? privateKey.slice(2) : privateKey

  // Private key 바이트로 변환
  const privKeyBytes = Buffer.from(privKeyHex, 'hex')

  if (privKeyBytes.length !== 32) {
    throw new Error('Invalid private key length. Must be 32 bytes.')
  }

  // Ed25519 공개키 도출
  const keyPair = nacl.sign.keyPair.fromSeed(privKeyBytes)

  // 주소 생성
  const publicKeyHash = sha256(Buffer.from(keyPair.publicKey), { asBytes: false })
  return '0x' + publicKeyHash.slice(0, 40)
}

/**
 * Mnemonic 유효성 검증
 */
export function validateMnemonic(mnemonic: string): boolean {
  return bip39.validateMnemonic(mnemonic)
}
