import * as bip39 from 'bip39'
import { HDKey } from 'hdkey'
import { Wallet } from 'ethers'

/**
 * 24개 단어 mnemonic 생성
 */
export function generateMnemonic(): string {
  return bip39.generateMnemonic(256) // 256 bits = 24 words
}

/**
 * Mnemonic으로부터 지갑 생성
 * @param mnemonic 24개 단어
 * @param derivationPath HD wallet 파생 경로 (기본값: "m/44'/60'/0'/0/0")
 */
export function createWalletFromMnemonic(
  mnemonic: string,
  derivationPath: string = "m/44'/60'/0'/0/0"
): {
  address: string
  privateKey: string
  mnemonic: string
} {
  // Mnemonic 유효성 검증
  if (!bip39.validateMnemonic(mnemonic)) {
    throw new Error('Invalid mnemonic')
  }

  // Seed 생성
  const seed = bip39.mnemonicToSeedSync(mnemonic)

  // HD Key 생성
  const hdKey = HDKey.fromMasterSeed(seed)

  // 파생 경로로 child key 생성 (Ethereum standard: m/44'/60'/0'/0/0)
  const childKey = hdKey.derive(derivationPath)

  // 이더리움 지갑 생성
  if (!childKey.privateKey) {
    throw new Error('Failed to derive private key')
  }

  const wallet = new Wallet('0x' + childKey.privateKey.toString('hex'))

  return {
    address: wallet.address,
    privateKey: wallet.privateKey,
    mnemonic: mnemonic,
  }
}

/**
 * Mnemonic 유효성 검증
 */
export function validateMnemonic(mnemonic: string): boolean {
  return bip39.validateMnemonic(mnemonic)
}

/**
 * Mnemonic 정규화 (공백 처리, 소문자 변환)
 */
export function normalizeMnemonic(mnemonic: string): string {
  return mnemonic
    .trim()
    .toLowerCase()
    .replace(/\s+/g, ' ')
}
