import React, { useState, useEffect } from 'react'
import './App.css'
import { WalletHome } from '@/components/WalletHome'
import { ImportWallet } from '@/components/ImportWallet'
import { CreateWallet } from '@/components/CreateWallet'
import { GetStarted } from '@/components/GetStarted'
import { useWalletStore } from '@/store/wallet'

export function App() {
  const { wallet, initWallet } = useWalletStore()
  const [hasWallet, setHasWallet] = useState(false)
  const [step, setStep] = useState<'getstarted' | 'creating' | 'importing' | 'unlocking' | 'wallet'>('getstarted')

  useEffect(() => {
    const checkWallet = async () => {
      try {
        // 아니면 기존 지갑 확인
        const saved = await chrome.storage.local.get('wallet')
        if (saved.wallet) {
          setHasWallet(true)
          initWallet(saved.wallet)
          setStep('wallet')
        }
      } catch (error) {
        console.error('Failed to load wallet:', error)
      }
    }

    checkWallet()
  }, [])

  const handleCreateSuccess = () => {
    setHasWallet(true)
    setStep('wallet')
  }

  const handleImportSuccess = () => {
    setHasWallet(true)
    setStep('wallet')
  }

  const handleUnlockSuccess = () => {
    setHasWallet(true)
    setStep('wallet')
  }

  return (
    <>
      {step === 'getstarted' && (
        <GetStarted
          onCreateWallet={() => setStep('creating')}
          onUnlockSuccess={handleUnlockSuccess}
        />
      )}

      {step === 'creating' && (
        <CreateWallet
          onSuccess={handleCreateSuccess}
          onCancel={() => setStep('getstarted')}
        />
      )}

      {step === 'importing' && (
        <ImportWallet 
          onSuccess={handleImportSuccess}
          onCancel={() => setStep('getstarted')}
        />
      )}
      {step === 'wallet' && hasWallet && <WalletHome />}
    </>
  )
}
