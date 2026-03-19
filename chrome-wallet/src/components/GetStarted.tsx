import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey } from '@/utils/crypto'
import '../styles/GetStarted.css'
import walletBg from '@/assets/chrome_wallet_logo_1024_1536.png'

interface GetStartedProps {
  onCreateWallet: () => void
  onUnlockSuccess: () => void
}

export function GetStarted({ onCreateWallet, onUnlockSuccess }: GetStartedProps) {
  const { initWallet } = useWalletStore()
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  const handleUnlock = async () => {
    if (!password || loading) return

    setError('')
    setLoading(true)

    try {
      const result = await chrome.storage.local.get(['encryptedWallet'])

      if (!result.encryptedWallet) {
        setError('No saved wallet found')
        return
      }

      const { address, encryptedPrivateKey, salt, iv } = result.encryptedWallet
      const privateKey = decryptPrivateKey(encryptedPrivateKey, password, salt, iv)

      initWallet({ address, privateKey, balance: '0' })
      onUnlockSuccess()
    } catch {
      setError('Invalid password')
    } finally {
      setLoading(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') handleUnlock()
  }

  return (
    <div
      className="get-started-container"
      style={{ backgroundImage: `url(${walletBg})` }}
    >
      <div className="get-started-overlay">
        {error && <div className="gs-error">{error}</div>}

        <div className="gs-input-group">
          <input
            type="password"
            placeholder="Enter password"
            value={password}
            onChange={(e) => {
              setPassword(e.target.value)
              setError('')
            }}
            onKeyDown={handleKeyDown}
            disabled={loading}
            autoFocus
          />
        </div>

        <button
          className="gs-btn-unlock"
          onClick={handleUnlock}
          disabled={loading || !password}
        >
          {loading ? 'Unlocking...' : 'Unlock Wallet'}
        </button>

        <button
          className="gs-btn-create"
          onClick={onCreateWallet}
          type="button"
        >
          Create New Wallet
        </button>
      </div>
    </div>
  )
}
