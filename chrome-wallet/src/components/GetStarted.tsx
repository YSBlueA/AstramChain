import React from 'react'
import '../styles/GetStarted.css'

interface GetStartedProps {
  onCreateWallet: () => void
  onImportWallet: () => void
  onUnlockWallet: () => void
}

export function GetStarted({ onCreateWallet, onImportWallet, onUnlockWallet }: GetStartedProps) {
  return (
    <div className="get-started-container">
      <div className="get-started-content">
        <div className="logo">
          <h1>🪙 Astram Wallet</h1>
        </div>
        
        <p className="subtitle">Your gateway to Astram blockchain</p>

        <div className="button-group">
          <button onClick={onUnlockWallet} className="btn-unlock btn-large">
            <span className="icon">🔐</span>
            Unlock Wallet
          </button>

          <button onClick={onCreateWallet} className="btn-primary btn-large">
            <span className="icon">✨</span>
            Create New Wallet
          </button>

          <button onClick={onImportWallet} className="btn-secondary btn-large">
            <span className="icon">📥</span>
            Import Wallet
          </button>
        </div>

        <p className="footer-text">
          Secure, decentralized wallet for Astram blockchain
        </p>
      </div>
    </div>
  )
}
