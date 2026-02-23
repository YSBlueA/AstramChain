import React, { useState, useEffect } from 'react'
import './App.css'
import { WalletHome } from '@/components/WalletHome'
import { ImportWallet } from '@/components/ImportWallet'
import { CreateWallet } from '@/components/CreateWallet'
import { UnlockWallet } from '@/components/UnlockWallet'
import { GetStarted } from '@/components/GetStarted'
import { useWalletStore } from '@/store/wallet'

export function App() {
  const { wallet, initWallet } = useWalletStore()
  const [hasWallet, setHasWallet] = useState(false)
  const [step, setStep] = useState<'getstarted' | 'creating' | 'importing' | 'unlocking' | 'wallet'>('getstarted')

  useEffect(() => {
    const checkWallet = async () => {
      try {
        // 암호화된 지갑이 있는지 확인
        const result = await chrome.storage.local.get('encryptedWallet')
        if (result.encryptedWallet) {
          // 암호화된 지갑이 있으면 unlock 화면으로
          setStep('unlocking')
          return
        }

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
          onImportWallet={() => setStep('importing')}
          onUnlockWallet={() => setStep('unlocking')}
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

      {step === 'unlocking' && (
        <UnlockWallet
          onSuccess={handleUnlockSuccess}
          onCancel={() => setStep('getstarted')}
        />
      )}

      {step === 'wallet' && hasWallet && <WalletHome />}
    </>
  )
}
