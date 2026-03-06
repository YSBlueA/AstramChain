import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey } from '@/utils/crypto'
import '../styles/GetStarted.css'
import astramLogo from '../../../assets/astram_logo.png'
import chromeWalletLogo from '../../../assets/chrome_wallet_logo.png'

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
    if (!password || loading) {
      return
    }

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

      initWallet({
        address,
        privateKey,
        balance: '0',
      })

      onUnlockSuccess()
    } catch (unlockError) {
      setError('Invalid password')
    } finally {
      setLoading(false)
    }
  }

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      handleUnlock()
    }
  }

  return (
    <div className="get-started-container">
      <div className="get-started-content">
        <div className="logos">
          <img src={astramLogo} alt="Astram" className="astram-logo" />
          <img src={chromeWalletLogo} alt="Chrome Wallet" className="wallet-logo" />
        </div>

        <div className="input-group">
          <input
            type="password"
            placeholder="Enter password"
            value={password}
            onChange={(event) => {
              setPassword(event.target.value)
              setError('')
            }}
            onKeyDown={handleKeyDown}
            disabled={loading}
            autoFocus
          />
        </div>

        {error && <div className="error-message">{error}</div>}

        <button
          onClick={handleUnlock}
          className="btn-unlock"
          disabled={loading || !password}
        >
          {loading ? 'Unlocking...' : 'Unlock Wallet'}
        </button>

        <button onClick={onCreateWallet} className="create-wallet-link" type="button">
          Create New Wallet
        </button>
      </div>
    </div>
  )
}
