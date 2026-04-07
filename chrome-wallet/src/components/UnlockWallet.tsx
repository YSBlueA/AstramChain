import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { decryptPrivateKey, encryptMnemonic } from '@/utils/crypto'
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

      const { address, encryptedPrivateKey, salt, iv, mnemonic, encryptedMnemonic } = result.encryptedWallet
      const privateKey = decryptPrivateKey(encryptedPrivateKey, password, salt, iv)

      // 마이그레이션: 평문 mnemonic이 있으면 암호화해서 재저장
      if (mnemonic && !encryptedMnemonic) {
        const { encryptedMnemonic: em, mnemonicSalt, mnemonicIv } = encryptMnemonic(mnemonic, password)
        const { mnemonic: _removed, ...rest } = result.encryptedWallet
        await chrome.storage.local.set({
          encryptedWallet: { ...rest, encryptedMnemonic: em, mnemonicSalt, mnemonicIv },
        })
      }

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
