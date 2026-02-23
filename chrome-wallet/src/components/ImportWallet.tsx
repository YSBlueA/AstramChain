import React, { useState } from 'react'
import { useWalletStore } from '@/store/wallet'
import { createWalletFromEd25519Mnemonic, validateMnemonic } from '@/utils/ed25519-mnemonic'
import { encryptPrivateKey } from '@/utils/crypto'
import '../styles/ImportWallet.css'

interface ImportWalletProps {
  onSuccess: () => void
  onCancel?: () => void
}

export function ImportWallet({ onSuccess, onCancel }: ImportWalletProps) {
  const { initWallet } = useWalletStore()
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
      const normalizedMnemonic = mnemonic
        .trim()
        .toLowerCase()
        .replace(/\s+/g, ' ')

      // Mnemonic 유효성 검증
      if (!validateMnemonic(normalizedMnemonic)) {
        setError('Invalid recovery phrase. Please check and try again.')
        setLoading(false)
        return
      }

      // Mnemonic으로부터 Ed25519 지갑 생성
      const recoveredWallet = createWalletFromEd25519Mnemonic(normalizedMnemonic)
      
      setWallet({
        ...recoveredWallet,
        mnemonic: normalizedMnemonic,
      })
      setStep('password')
    } catch (err: any) {
      setError(err.message || 'Failed to import wallet')
    } finally {
      setLoading(false)
    }
  }

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

    if (!wallet) {
      setError('Wallet not found')
      return
    }

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
      setError('Failed to save wallet')
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !loading) {
      if (step === 'mnemonic' && mnemonic.trim()) {
        handleVerifyMnemonic()
      } else if (step === 'password' && password && confirmPassword) {
        handleSetPassword()
      }
    }
  }

  return (
    <div className="import-wrapper">
      <div className="import-container">
        {step === 'mnemonic' && (
          <>
            <h2>Import Wallet</h2>
            <p className="import-subtitle">Paste your 24-word recovery phrase (Ed25519)</p>

            <div className="form-group">
              <label>Recovery Phrase</label>
              <textarea
                placeholder="Enter your 24 words separated by spaces..."
                value={mnemonic}
                onChange={(e) => {
                  setMnemonic(e.target.value)
                  setError('')
                }}
                onKeyPress={handleKeyPress}
                disabled={loading}
                rows={6}
              />
              <p className="helper-text">Enter all 24 words in order, separated by spaces</p>
            </div>

            {error && <div className="error-message">{error}</div>}

            <div className="button-group">
              <button
                onClick={handleVerifyMnemonic}
                className="btn-primary"
                disabled={loading || !mnemonic.trim()}
              >
                {loading ? 'Verifying...' : 'Next'}
              </button>
              {onCancel && (
                <button onClick={onCancel} className="btn-secondary" disabled={loading}>
                  Cancel
                </button>
              )}
            </div>
          </>
        )}

        {step === 'password' && wallet && (
          <>
            <h2>Set Password</h2>
            <p className="import-subtitle">✅ Wallet recovered: {wallet.address.slice(0, 10)}...</p>

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
                onKeyPress={handleKeyPress}
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
                Save Wallet
              </button>
              <button 
                onClick={() => setStep('mnemonic')} 
                className="btn-secondary"
              >
                Back
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
