import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { encryptPrivateKey } from '@/utils/crypto'
import { generateEd25519Wallet, createWalletFromEd25519Mnemonic } from '@/utils/ed25519-mnemonic'
import '../styles/CreateWallet.css'

interface CreateWalletProps {
  onSuccess: () => void
  onCancel: () => void
}

export function CreateWallet({ onSuccess, onCancel }: CreateWalletProps) {
  const { initWallet } = useWalletStore()
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
      setError('Please fill in all fields')
      return
    }

    if (password.length < 8) {
      setError('Password must be at least 8 characters')
      return
    }

    if (password !== confirmPassword) {
      setError('Passwords do not match')
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
        setWallet({
          ...newWallet,
          mnemonic,
        })
        setStep('review')
      } catch (err) {
        setError('Failed to create wallet from mnemonic')
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

      const walletData = {
        address: wallet.address,
        privateKey: wallet.privateKey,
        balance: '0'
      }

      initWallet(walletData)
      onSuccess()
    } catch (err) {
      setError('Failed to create wallet')
    }
  }

  return (
    <div className="create-wallet-container">
      {step === 'password' && (
        <div className="create-wallet-form">
          <h2>Create New Wallet</h2>
          <p className="info-text">Set a password to secure your wallet</p>

          <div className="form-group">
            <label>Password</label>
            <input
              type="password"
              placeholder="Enter password (min 8 characters)"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value)
                setError('')
              }}
            />
          </div>

          <div className="form-group">
            <label>Confirm Password</label>
            <input
              type="password"
              placeholder="Confirm password"
              value={confirmPassword}
              onChange={(e) => {
                setConfirmPassword(e.target.value)
                setError('')
              }}
            />
          </div>

          {error && <div className="error-message">{error}</div>}

          <div className="button-group">
            <button onClick={handleSetPassword} className="btn-primary">
              Next
            </button>
            <button onClick={onCancel} className="btn-secondary">
              Cancel
            </button>
          </div>
        </div>
      )}

      {step === 'mnemonic' && (
        <div className="create-wallet-form">
          <h2>Your Recovery Phrase</h2>
          <p className="warning-text">⚠️ Save these 24 words in a safe place!</p>
          <p className="info-text">Never share this phrase with anyone</p>

          <div className="mnemonic-container">
            {mnemonic.split(' ').map((word, index) => (
              <div key={index} className="mnemonic-word">
                <span className="word-number">{index + 1}.</span>
                <span className="word-text">{word}</span>
              </div>
            ))}
          </div>

          <div className="button-group">
            <button onClick={handleMnemonicReady} className="btn-primary">
              I've Saved My Recovery Phrase
            </button>
            <button onClick={onCancel} className="btn-secondary">
              Cancel
            </button>
          </div>
        </div>
      )}

      {step === 'confirm' && (
        <div className="create-wallet-form">
          <h2>Verify Recovery Phrase</h2>
          <p className="info-text">Enter these 3 words to confirm you saved correctly</p>

          <div className="verify-container">
            {[mnemonic.split(' ')[0], mnemonic.split(' ')[11], mnemonic.split(' ')[23]].map((word, idx) => (
              <div key={idx} className="verify-item">
                <label>Word #{idx === 0 ? 1 : idx === 1 ? 12 : 24}</label>
                <input
                  type="text"
                  placeholder="Enter word..."
                  onBlur={(e) => {
                    if (e.target.value === word) {
                      setConfirmedWords(prev => new Set([...prev, idx]))
                      e.target.style.borderColor = '#4CAF50'
                    } else if (e.target.value) {
                      setError('Incorrect word')
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
              {confirmedWords.size < 3 ? `Verified (${confirmedWords.size}/3)` : 'Continue'}
            </button>
            <button onClick={() => setStep('mnemonic')} className="btn-secondary">
              Back
            </button>
          </div>
        </div>
      )}

      {step === 'review' && wallet && (
        <div className="create-wallet-review">
          <h2>Wallet Created! 🎉</h2>
          <p className="success-text">✅ Your Ed25519 wallet is ready</p>

          <div className="wallet-details">
            <div className="detail-item">
              <label>Address</label>
              <code className="wallet-code">{wallet.address}</code>
            </div>

            <div className="detail-item">
              <label>Private Key</label>
              <code className="wallet-code">{wallet.privateKey}</code>
            </div>
          </div>

          <p className="compat-note">✨ Compatible with Node mining and wallet-cli</p>

          <div className="button-group">
            <button onClick={handleConfirmWallet} className="btn-primary">
              Open Wallet
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
