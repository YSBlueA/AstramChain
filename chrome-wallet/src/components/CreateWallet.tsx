import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { encryptPrivateKey } from '@/utils/crypto'
import { generateEd25519Wallet, createWalletFromEd25519Mnemonic } from '@/utils/ed25519-mnemonic'
import { useTranslation } from 'react-i18next'
import '../styles/CreateWallet.css'

interface CreateWalletProps {
  onSuccess: () => void
  onCancel: () => void
}

export function CreateWallet({ onSuccess, onCancel }: CreateWalletProps) {
  const { initWallet } = useWalletStore()
  const { t } = useTranslation()
  const [step, setStep] = useState<'password' | 'mnemonic' | 'confirm' | 'review'>('password')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [error, setError] = useState('')
  const [mnemonic, setMnemonic] = useState('')
  const [wallet, setWallet] = useState<{ address: string; privateKey: string; mnemonic: string } | null>(null)
  const [confirmedWords, setConfirmedWords] = useState<Set<number>>(new Set())

  const handleSetPassword = async () => {
    setError('')

    if (!password || !confirmPassword) {
      setError(t('create.fillAllFields'))
      return
    }

    if (password.length < 8) {
      setError(t('create.passwordTooShort'))
      return
    }

    if (password !== confirmPassword) {
      setError(t('create.passwordMismatch'))
      return
    }

    const newWallet = generateEd25519Wallet()
    setMnemonic(newWallet.mnemonic)
    setStep('mnemonic')
  }

  const handleMnemonicReady = () => {
    setStep('confirm')
  }

  const handleConfirmMnemonic = () => {
    if (confirmedWords.size >= 3) {
      try {
        const newWallet = createWalletFromEd25519Mnemonic(mnemonic)
        setWallet({ ...newWallet, mnemonic })
        setStep('review')
      } catch (err) {
        setError(t('create.failedCreateMnemonic'))
      }
    }
  }

  const handleConfirmWallet = async () => {
    if (!wallet) return

    try {
      const { encryptedPrivateKey, salt, iv } = encryptPrivateKey(wallet.privateKey, password)

      const encryptedWallet = {
        address: wallet.address,
        encryptedPrivateKey,
        salt,
        iv,
        mnemonic: wallet.mnemonic,
      }

      await chrome.storage.local.set({ encryptedWallet })
      initWallet({ address: wallet.address, privateKey: wallet.privateKey, balance: '0' })
      onSuccess()
    } catch (err) {
      setError(t('create.failedCreate'))
    }
  }

  return (
    <div className="create-wallet-container">
      {step === 'password' && (
        <div className="create-wallet-form">
          <h2>{t('create.title')}</h2>
          <p className="info-text">{t('create.passwordSubtitle')}</p>

          <div className="form-group">
            <label>{t('password')}</label>
            <input
              type="password"
              placeholder={t('create.passwordPlaceholder')}
              value={password}
              onChange={(e) => { setPassword(e.target.value); setError('') }}
            />
          </div>

          <div className="form-group">
            <label>{t('confirmPassword')}</label>
            <input
              type="password"
              placeholder={t('create.confirmPasswordPlaceholder')}
              value={confirmPassword}
              onChange={(e) => { setConfirmPassword(e.target.value); setError('') }}
            />
          </div>

          {error && <div className="error-message">{error}</div>}

          <div className="button-group">
            <button onClick={handleSetPassword} className="btn-primary">{t('next')}</button>
            <button onClick={onCancel} className="btn-secondary">{t('cancel')}</button>
          </div>
        </div>
      )}

      {step === 'mnemonic' && (
        <div className="create-wallet-form">
          <h2>{t('create.mnemonicTitle')}</h2>
          <p className="warning-text">{t('create.mnemonicWarning')}</p>
          <p className="info-text">{t('create.mnemonicNote')}</p>

          <div className="mnemonic-container">
            {mnemonic.split(' ').map((word, index) => (
              <div key={index} className="mnemonic-word">
                <span className="word-number">{index + 1}.</span>
                <span className="word-text">{word}</span>
              </div>
            ))}
          </div>

          <div className="button-group">
            <button onClick={handleMnemonicReady} className="btn-primary">{t('create.savedPhrase')}</button>
            <button onClick={onCancel} className="btn-secondary">{t('cancel')}</button>
          </div>
        </div>
      )}

      {step === 'confirm' && (
        <div className="create-wallet-form">
          <h2>{t('create.verifyTitle')}</h2>
          <p className="info-text">{t('create.verifyNote')}</p>

          <div className="verify-container">
            {[mnemonic.split(' ')[0], mnemonic.split(' ')[11], mnemonic.split(' ')[23]].map((word, idx) => (
              <div key={idx} className="verify-item">
                <label>{t('create.wordLabel', { n: idx === 0 ? 1 : idx === 1 ? 12 : 24 })}</label>
                <input
                  type="text"
                  placeholder={t('create.wordPlaceholder')}
                  onBlur={(e) => {
                    if (e.target.value === word) {
                      setConfirmedWords(prev => new Set([...prev, idx]))
                      e.target.style.borderColor = '#4CAF50'
                    } else if (e.target.value) {
                      setError(t('create.incorrectWord'))
                      e.target.style.borderColor = '#d32f2f'
                    }
                  }}
                />
              </div>
            ))}
          </div>

          {error && <div className="error-message">{error}</div>}

          <div className="button-group">
            <button
              onClick={handleConfirmMnemonic}
              className="btn-primary"
              disabled={confirmedWords.size < 3}
            >
              {confirmedWords.size < 3
                ? t('create.verifiedCount', { n: confirmedWords.size })
                : t('create.continue')}
            </button>
            <button onClick={() => setStep('mnemonic')} className="btn-secondary">{t('back')}</button>
          </div>
        </div>
      )}

      {step === 'review' && wallet && (
        <div className="create-wallet-review">
          <h2>{t('create.successTitle')}</h2>
          <p className="success-text">{t('create.successNote')}</p>

          <div className="wallet-details">
            <div className="detail-item">
              <label>{t('address')}</label>
              <code className="wallet-code">{wallet.address}</code>
            </div>
            <div className="detail-item">
              <label>{t('privateKey')}</label>
              <code className="wallet-code">{wallet.privateKey}</code>
            </div>
          </div>

          <p className="compat-note">{t('create.compatNote')}</p>

          <div className="button-group">
            <button onClick={handleConfirmWallet} className="btn-primary">{t('create.openWallet')}</button>
          </div>
        </div>
      )}
    </div>
  )
}
