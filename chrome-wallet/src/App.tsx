import React, { useState, useEffect } from 'react'
import './App.css'
import '@/i18n'
import { loadSavedLanguage } from '@/i18n'
import { WalletHome } from '@/components/WalletHome'
import { ImportWallet } from '@/components/ImportWallet'
import { CreateWallet } from '@/components/CreateWallet'
import { GetStarted } from '@/components/GetStarted'
import { TxApproval } from '@/components/TxApproval'
import { useWalletStore } from '@/store/wallet'

interface PendingTx {
  id: string
  to: string
  amount: number
}

// 패널이 닫힌 후 이 시간(ms)이 지나면 첫 페이지로 돌아감 (기본 5분)
const AUTO_LOCK_MS = 5 * 60 * 1000

export function App() {
  const { wallet, initWallet } = useWalletStore()
  const [hasWallet, setHasWallet] = useState(false)
  const [step, setStep] = useState<'getstarted' | 'creating' | 'importing' | 'unlocking' | 'wallet'>('getstarted')
  const [pendingTx, setPendingTx] = useState<PendingTx | null>(null)

  // pendingTx 감지 — dApp에서 signTransaction 요청이 오면 TxApproval 표시
  useEffect(() => {
    chrome.storage.local.get('pendingTx', (data) => {
      if (data.pendingTx) setPendingTx(data.pendingTx as PendingTx)
    })
    const listener = (changes: { [key: string]: chrome.storage.StorageChange }) => {
      if ('pendingTx' in changes) {
        setPendingTx((changes.pendingTx.newValue as PendingTx) ?? null)
      }
    }
    chrome.storage.onChanged.addListener(listener)
    return () => chrome.storage.onChanged.removeListener(listener)
  }, [])

  // 패널이 숨겨질 때(닫힘/탭 전환) 타임스탬프 저장
  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') {
        chrome.storage.local.set({ panelHiddenAt: Date.now() })
      }
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [])

  useEffect(() => {
    loadSavedLanguage()
    const checkWallet = async () => {
      try {
        const saved = await chrome.storage.local.get(['wallet', 'panelHiddenAt'])

        // 마지막으로 패널이 숨겨진 시점이 있고, AUTO_LOCK_MS 이상 지났으면 첫 페이지
        const hiddenAt = saved.panelHiddenAt as number | undefined
        const timedOut = hiddenAt != null && (Date.now() - hiddenAt) > AUTO_LOCK_MS

        if (saved.wallet && !timedOut) {
          setHasWallet(true)
          initWallet(saved.wallet)
          setStep('wallet')
        }
        // timedOut이면 getstarted(기본값) 유지
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

  // pendingTx가 있으면 현재 step에 관계없이 승인 UI 표시
  if (pendingTx) {
    return <TxApproval pendingTx={pendingTx} onDone={() => setPendingTx(null)} />
  }

  return (
    <>
      {step === 'getstarted' && (
        <GetStarted
          onCreateWallet={() => setStep('creating')}
          onUnlockSuccess={handleUnlockSuccess}
          onRestoreWallet={() => setStep('importing')}
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
      {step === 'wallet' && hasWallet && (
        <WalletHome
          onLogout={() => { setHasWallet(false); setStep('getstarted') }}
        />
      )}
    </>
  )
}
