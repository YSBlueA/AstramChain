import CryptoJS from 'crypto-js'

// 좀 더 강한 암호화를 위해 PBKDF2 사용
export function deriveKey(password: string, salt: string, iterations: number = 1000): string {
  const derived = CryptoJS.PBKDF2(password, salt, {
    keySize: 256 / 32,
    iterations: iterations,
    hasher: CryptoJS.algo.SHA256,
  })
  return derived.toString()
}

// 암호화
export function encryptPrivateKey(privateKey: string, password: string): {
  encryptedPrivateKey: string
  salt: string
  iv: string
} {
  // 랜덤 salt와 iv 생성
  const salt = CryptoJS.lib.WordArray.random(16).toString()
  const iv = CryptoJS.lib.WordArray.random(16).toString()

  // 암호로부터 키 파생
  const key = deriveKey(password, salt)

  // AES 암호화
  const encrypted = CryptoJS.AES.encrypt(privateKey, key, {
    iv: CryptoJS.enc.Hex.parse(iv),
    mode: CryptoJS.mode.CBC,
    padding: CryptoJS.pad.Pkcs7,
  })

  return {
    encryptedPrivateKey: encrypted.toString(),
    salt,
    iv,
  }
}

// 복호화
export function decryptPrivateKey(
  encryptedPrivateKey: string,
  password: string,
  salt: string,
  iv: string
): string {
  try {
    // 암호로부터 키 파생
    const key = deriveKey(password, salt)

    // AES 복호화
    const decrypted = CryptoJS.AES.decrypt(encryptedPrivateKey, key, {
      iv: CryptoJS.enc.Hex.parse(iv),
      mode: CryptoJS.mode.CBC,
      padding: CryptoJS.pad.Pkcs7,
    })

    const privateKey = decrypted.toString(CryptoJS.enc.Utf8)

    if (!privateKey) {
      throw new Error('Invalid password')
    }

    return privateKey
  } catch (err) {
    throw new Error('Failed to decrypt private key')
  }
}
