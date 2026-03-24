import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey } from '@/utils/crypto'
import { useTranslation } from 'react-i18next'
import '../styles/UnlockWallet.css'

interface UnlockWalletProps {
  onSuccess: () => void
  onCancel: () => void
}

export function UnlockWallet({ onSuccess, onCancel }: UnlockWalletProps) {
  const { initWallet } = useWalletStore()
  const { t } = useTranslation()
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const handleUnlock = async () => {
    setError('')
    setLoading(true)

    try {
      const result = await chrome.storage.local.get(['encryptedWallet'])

      if (!result.encryptedWallet) {
        setError(t('unlock.noWalletFound'))
        setLoading(false)
        return
      }

      const { address, encryptedPrivateKey, salt, iv } = result.encryptedWallet
      const privateKey = decryptPrivateKey(encryptedPrivateKey, password, salt, iv)

      initWallet({ address, privateKey, balance: '0' })
      onSuccess()
    } catch (err) {
      setError(t('unlock.invalidPassword'))
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
          <h1>{t('unlock.title')}</h1>
          <p>{t('unlock.subtitle')}</p>
        </div>

        <div className="form-group">
          <label>{t('password')}</label>
          <input
            type="password"
            placeholder={t('unlock.enterPassword')}
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
            {loading ? t('unlock.unlocking') : t('unlock.unlockWallet')}
          </button>
          <button onClick={onCancel} className="btn-secondary" disabled={loading}>
            {t('unlock.useDifferentAccount')}
          </button>
        </div>

        <p className="security-note">{t('unlock.securityNote')}</p>
      </div>
    </div>
  )
}
