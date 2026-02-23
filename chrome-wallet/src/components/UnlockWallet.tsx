import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey } from '@/utils/crypto'
import '../styles/UnlockWallet.css'

interface UnlockWalletProps {
  onSuccess: () => void
  onCancel: () => void
}

export function UnlockWallet({ onSuccess, onCancel }: UnlockWalletProps) {
  const { initWallet } = useWalletStore()
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const handleUnlock = async () => {
    setError('')
    setLoading(true)

    try {
      // Chrome storage에서 암호화된 지갑 정보 로드
      const result = await chrome.storage.local.get(['encryptedWallet'])

      if (!result.encryptedWallet) {
        setError('No saved wallet found')
        setLoading(false)
        return
      }

      const { address, encryptedPrivateKey, salt, iv } = result.encryptedWallet

      // 암호로 복호화 시도
      const privateKey = decryptPrivateKey(encryptedPrivateKey, password, salt, iv)

      // 성공하면 메모리에 로드
      const wallet = {
        address,
        privateKey,
        balance: '0',
      }

      initWallet(wallet)
      onSuccess()
    } catch (err) {
      setError('Invalid password')
    } finally {
      setLoading(false)
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !loading) {
      handleUnlock()
    }
  }

  return (
    <div className="unlock-wrapper">
      <div className="unlock-container">
        <div className="unlock-header">
          <h1>🔐 Astram Wallet</h1>
          <p>Enter your password to unlock</p>
        </div>

        <div className="form-group">
          <label>Password</label>
          <input
            type="password"
            placeholder="Enter your password"
            value={password}
            onChange={(e) => {
              setPassword(e.target.value)
              setError('')
            }}
            onKeyPress={handleKeyPress}
            disabled={loading}
            autoFocus
          />
        </div>

        {error && <div className="error-message">{error}</div>}

        <div className="button-group">
          <button
            onClick={handleUnlock}
            className="btn-primary"
            disabled={loading || !password}
          >
            {loading ? 'Unlocking...' : 'Unlock Wallet'}
          </button>
          <button onClick={onCancel} className="btn-secondary" disabled={loading}>
            Use Different Account
          </button>
        </div>

        <p className="security-note">
          🔒 Your password is only used locally to decrypt your private key
        </p>
      </div>
    </div>
  )
}
