import React, { useState, useEffect } from 'react'
import { useWalletStore } from '@/store/wallet'
import axios from 'axios'
import '../styles/WalletHome.css'

const ASTRAM_RPC = 'http://localhost:19533'

export function WalletHome() {
  const { wallet, updateBalance, clearWallet } = useWalletStore()
  const [balance, setBalance] = useState('0')
  const [loading, setLoading] = useState(false)

  const fetchBalance = async () => {
    if (!wallet) return

    setLoading(true)
    try {
      const response = await axios.get(`${ASTRAM_RPC}/address/${wallet.address}/balance`)
      const balanceStr = response.data.balance?.toString() || '0'
      
      // Convert to ASRM (divide by 10^18)
      const balanceRam = BigInt(balanceStr)
      const balanceAsrm = Number(balanceRam) / 1e18
      
      setBalance(balanceAsrm.toFixed(6))
      updateBalance(balanceStr)
    } catch (error) {
      console.error('Failed to fetch balance:', error)
      setBalance('0.000000')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (!wallet?.address) return

    fetchBalance()
    const interval = setInterval(fetchBalance, 10000)
    return () => clearInterval(interval)
  }, [wallet?.address])

  const handleCopyAddress = () => {
    if (wallet) {
      navigator.clipboard.writeText(wallet.address)
      alert('Address copied!')
    }
  }

  return (
    <div className="wallet-container">
      <div className="wallet-header">
        <h2>Astram Wallet</h2>
        <button onClick={() => clearWallet()} className="btn-logout">
          Logout
        </button>
      </div>

      <div className="wallet-card">
        <div className="balance-section">
          <p className="balance-label">Balance</p>
          <h1 className="balance-amount">{balance} ASRM</h1>
        </div>

        <div className="address-section">
          <p className="address-label">Address</p>
          <div className="address-display">
            <code>{wallet?.address}</code>
            <button onClick={handleCopyAddress} className="btn-copy">
              Copy
            </button>
          </div>
        </div>

        <button
          onClick={fetchBalance}
          disabled={loading}
          className="btn-refresh"
        >
          {loading ? 'Refreshing...' : 'Refresh Balance'}
        </button>
      </div>
    </div>
  )
}
