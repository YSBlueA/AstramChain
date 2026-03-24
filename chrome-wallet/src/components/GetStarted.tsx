import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey } from '@/utils/crypto'
import { useTranslation } from 'react-i18next'
import '../styles/GetStarted.css'
import walletBg from '@/assets/chrome_wallet_logo_1024_1536.png'

interface GetStartedProps {
  onCreateWallet: () => void
  onUnlockSuccess: () => void
  onRestoreWallet: () => void
}

export function GetStarted({ onCreateWallet, onUnlockSuccess, onRestoreWallet }: GetStartedProps) {
  const { initWallet } = useWalletStore()
  const { t } = useTranslation()
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
        setError(t('getStarted.noWalletFound'))
        return
      }

      const { address, encryptedPrivateKey, salt, iv } = result.encryptedWallet
      const privateKey = decryptPrivateKey(encryptedPrivateKey, password, salt, iv)

      initWallet({ address, privateKey, balance: '0' })
      onUnlockSuccess()
    } catch {
      setError(t('getStarted.invalidPassword'))
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
            placeholder={t('getStarted.enterPassword')}
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
          {loading ? t('getStarted.unlocking') : t('getStarted.unlockWallet')}
        </button>

        <button
          className="gs-btn-create"
          onClick={onCreateWallet}
          type="button"
        >
          {t('getStarted.createNewWallet')}
        </button>

        <button
          className="gs-btn-restore"
          onClick={onRestoreWallet}
          type="button"
        >
          {t('getStarted.restoreWithKey')}
        </button>
      </div>
    </div>
  )
}
