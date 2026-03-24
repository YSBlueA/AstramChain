import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { createWalletFromEd25519Mnemonic, validateMnemonic } from '@/utils/ed25519-mnemonic'
import { encryptPrivateKey } from '@/utils/crypto'
import { useTranslation } from 'react-i18next'
import '../styles/ImportWallet.css'

interface ImportWalletProps {
  onSuccess: () => void
  onCancel?: () => void
}

export function ImportWallet({ onSuccess, onCancel }: ImportWalletProps) {
  const { initWallet } = useWalletStore()
  const { t } = useTranslation()
  const [step, setStep] = useState<'mnemonic' | 'password'>('mnemonic')
  const [mnemonic, setMnemonic] = useState('')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [wallet, setWallet] = useState<{ address: string; privateKey: string; mnemonic: string } | null>(null)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const handleVerifyMnemonic = async () => {
    setError('')
    setLoading(true)

    try {
      const normalizedMnemonic = mnemonic.trim().toLowerCase().replace(/\s+/g, ' ')

      if (!validateMnemonic(normalizedMnemonic)) {
        setError(t('import.invalidPhrase'))
        setLoading(false)
        return
      }

      const recoveredWallet = createWalletFromEd25519Mnemonic(normalizedMnemonic)
      setWallet({ ...recoveredWallet, mnemonic: normalizedMnemonic })
      setStep('password')
    } catch (err: any) {
      setError(err.message || t('import.failedImport'))
    } finally {
      setLoading(false)
    }
  }

  const handleSetPassword = async () => {
    setError('')

    if (!password || !confirmPassword) {
      setError(t('import.fillAllFields'))
      return
    }
    if (password.length < 8) {
      setError(t('import.passwordTooShort'))
      return
    }
    if (password !== confirmPassword) {
      setError(t('import.passwordMismatch'))
      return
    }
    if (!wallet) {
      setError(t('import.walletNotFound'))
      return
    }

    try {
      const { encryptedPrivateKey, salt, iv } = encryptPrivateKey(wallet.privateKey, password)

      await chrome.storage.local.set({
        encryptedWallet: { address: wallet.address, encryptedPrivateKey, salt, iv, mnemonic: wallet.mnemonic },
      })

      initWallet({ address: wallet.address, privateKey: wallet.privateKey, balance: '0' })
      onSuccess()
    } catch (err) {
      setError(t('import.failedSave'))
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !loading) {
      if (step === 'mnemonic' && mnemonic.trim()) handleVerifyMnemonic()
      else if (step === 'password' && password && confirmPassword) handleSetPassword()
    }
  }

  return (
    <div className="import-wrapper">
      <div className="import-container">
        {step === 'mnemonic' && (
          <>
            <h2>{t('import.title')}</h2>
            <p className="import-subtitle">{t('import.subtitle')}</p>

            <div className="form-group">
              <label>{t('import.phraseLabel')}</label>
              <textarea
                placeholder={t('import.phrasePlaceholder')}
                value={mnemonic}
                onChange={(e) => { setMnemonic(e.target.value); setError('') }}
                onKeyPress={handleKeyPress}
                disabled={loading}
                rows={6}
              />
              <p className="helper-text">{t('import.phraseHelper')}</p>
            </div>

            {error && <div className="error-message">{error}</div>}

            <div className="button-group">
              <button
                onClick={handleVerifyMnemonic}
                className="btn-primary"
                disabled={loading || !mnemonic.trim()}
              >
                {loading ? t('import.verifying') : t('next')}
              </button>
              {onCancel && (
                <button onClick={onCancel} className="btn-secondary" disabled={loading}>
                  {t('cancel')}
                </button>
              )}
            </div>
          </>
        )}

        {step === 'password' && wallet && (
          <>
            <h2>{t('import.setPasswordTitle')}</h2>
            <p className="import-subtitle">
              {t('import.walletRecovered', { addr: wallet.address.slice(0, 10) })}
            </p>

            <div className="form-group">
              <label>{t('password')}</label>
              <input
                type="password"
                placeholder={t('import.passwordPlaceholder')}
                value={password}
                onChange={(e) => { setPassword(e.target.value); setError('') }}
                onKeyPress={handleKeyPress}
              />
            </div>

            <div className="form-group">
              <label>{t('confirmPassword')}</label>
              <input
                type="password"
                placeholder={t('import.confirmPasswordPlaceholder')}
                value={confirmPassword}
                onChange={(e) => { setConfirmPassword(e.target.value); setError('') }}
                onKeyPress={handleKeyPress}
              />
            </div>

            {error && <div className="error-message">{error}</div>}

            <div className="button-group">
              <button
                onClick={handleSetPassword}
                className="btn-primary"
                disabled={!password || !confirmPassword}
              >
                {t('import.saveWallet')}
              </button>
              <button onClick={() => setStep('mnemonic')} className="btn-secondary">
                {t('back')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
