import React, { useState, useEffect, useRef, useTransition } from 'react'
import { useWalletStore } from '@/store/wallet'
import axios from 'axios'
import { sendTransaction } from '@/utils/transaction'
import { useTranslation } from 'react-i18next'
import i18n, { LANGUAGES } from '@/i18n'
import '../styles/WalletHome.css'

const RPC_TIMEOUT_MS = 8_000

const DEFAULT_RPC = 'https://rpc.astramchain.com'

interface WalletHomeProps {
  onLogout: () => void
}

export function WalletHome({ onLogout }: WalletHomeProps) {
  const { wallet, updateBalance, clearWallet } = useWalletStore()
  const { t } = useTranslation()
  const [, startTransition] = useTransition()

  const [balance, setBalance] = useState('0')
  const [rpcOnline, setRpcOnline] = useState<boolean | null>(null)
  const [rpcUrl, setRpcUrl] = useState(DEFAULT_RPC)

  interface TxEntry {
    txid: string
    block_height: number
    timestamp: number
    direction: 'send' | 'receive'
    amount: string
    counterpart: string
  }
  const [txHistory, setTxHistory] = useState<TxEntry[]>([])

  const txHistoryRendered = React.useMemo(() => txHistory.map((tx, i) => {
    const amountAsrm = (Number(BigInt(tx.amount)) / 1e18).toFixed(6)
    const date = new Date(tx.timestamp * 1000).toLocaleDateString(i18n.language, {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit'
    })
    const short = (addr: string) =>
      addr.length > 12 ? `${addr.slice(0, 6)}…${addr.slice(-4)}` : addr
    return (
      <li key={`${tx.txid}-${i}`} className={`tx-item tx-item-${tx.direction}`}>
        <span className="tx-direction-icon">{tx.direction === 'send' ? '↑' : '↓'}</span>
        <div className="tx-info">
          <span className="tx-counterpart">{short(tx.counterpart)}</span>
          <span className="tx-date">{date} · #{tx.block_height}</span>
        </div>
        <span className={`tx-amount ${tx.direction === 'send' ? 'tx-amount-send' : 'tx-amount-receive'}`}>
          {tx.direction === 'send' ? '-' : '+'}{amountAsrm} ASRM
        </span>
      </li>
    )
  }), [txHistory, i18n.language])

  const [menuOpen, setMenuOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  type ModalType = 'recovery' | 'privatekey' | 'rpc' | 'addwallet' | 'send' | 'receive' | 'language' | null
  const [modal, setModal] = useState<ModalType>(null)
  const [addrCopied, setAddrCopied] = useState(false)
  const addrCopyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [showSecret, setShowSecret] = useState(false)
  const [mnemonic, setMnemonic] = useState('')
  const [newRpc, setNewRpc] = useState('')
  const [rpcSaved, setRpcSaved] = useState(false)

  // Send form state
  const [sendTo, setSendTo] = useState('')
  const [sendAmount, setSendAmount] = useState('')
  const [sending, setSending] = useState(false)
  const [sendResult, setSendResult] = useState<{ ok: boolean; msg: string } | null>(null)

  // Load saved RPC
  useEffect(() => {
    chrome.storage.local.get('rpcUrl').then((data: Record<string, any>) => {
      if (data.rpcUrl) setRpcUrl(data.rpcUrl as string)
    })
  }, [])

  // Close menu on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  // AbortController refs — cancel in-flight requests before issuing new ones
  const balanceAbortRef = useRef<AbortController | null>(null)
  const txAbortRef = useRef<AbortController | null>(null)

  const fetchBalance = async () => {
    if (!wallet) return
    balanceAbortRef.current?.abort()
    const controller = new AbortController()
    balanceAbortRef.current = controller
    try {
      const response = await axios.get(`${rpcUrl}/address/${wallet.address}/balance`, {
        signal: controller.signal,
        timeout: RPC_TIMEOUT_MS,
      })
      const balanceStr = response.data.balance?.toString() || '0'
      const balanceRam = BigInt(balanceStr)
      const balanceAsrm = Number(balanceRam) / 1e18
      setBalance(balanceAsrm.toFixed(6))
      updateBalance(balanceStr)
      setRpcOnline(true)
    } catch (error: any) {
      if (axios.isCancel(error) || error?.code === 'ERR_CANCELED') return
      console.error('Failed to fetch balance:', error)
      setBalance('0.000000')
      setRpcOnline(false)
    }
  }

  const fetchTxHistory = async () => {
    if (!wallet) return
    txAbortRef.current?.abort()
    const controller = new AbortController()
    txAbortRef.current = controller
    try {
      const response = await axios.get(
        `${rpcUrl}/address/${wallet.address}/transactions?limit=20`,
        { signal: controller.signal, timeout: RPC_TIMEOUT_MS },
      )
      const txs: TxEntry[] = response.data.transactions || []
      // startTransition: 거래내역 업데이트는 긴급하지 않은 렌더링으로 처리
      // → React가 다른 입력 이벤트에 먼저 응답하고 여유 있을 때 렌더링
      startTransition(() => setTxHistory(txs.slice(0, 20)))
    } catch (error: any) {
      if (axios.isCancel(error) || error?.code === 'ERR_CANCELED') return
      console.error('Failed to fetch tx history:', error)
    }
  }

  useEffect(() => {
    if (!wallet?.address) return
    fetchBalance()
    fetchTxHistory()
    const balanceInterval = setInterval(fetchBalance, 10_000)
    const txInterval = setInterval(fetchTxHistory, 30_000)
    return () => {
      clearInterval(balanceInterval)
      clearInterval(txInterval)
      balanceAbortRef.current?.abort()
      txAbortRef.current?.abort()
    }
  }, [wallet?.address, rpcUrl])

  const openMenu = (item: ModalType) => {
    setMenuOpen(false)
    setShowSecret(false)
    setModal(item)
  }

  const handleViewRecovery = async () => {
    const data = await chrome.storage.local.get('encryptedWallet') as Record<string, any>
    setMnemonic(data.encryptedWallet?.mnemonic || '')
    openMenu('recovery')
  }

  const handleCopyReceive = () => {
    navigator.clipboard.writeText(wallet?.address || '')
    setAddrCopied(true)
    if (addrCopyTimerRef.current) clearTimeout(addrCopyTimerRef.current)
    addrCopyTimerRef.current = setTimeout(() => setAddrCopied(false), 2000)
  }

  const handleOpenRpc = () => {
    setNewRpc(rpcUrl)
    setRpcSaved(false)
    openMenu('rpc')
  }

  const handleSaveRpc = async () => {
    const trimmed = newRpc.trim()
    if (!trimmed) return
    await chrome.storage.local.set({ rpcUrl: trimmed })
    setRpcUrl(trimmed)
    setRpcSaved(true)
  }

  const handleResetRpc = async () => {
    await chrome.storage.local.remove('rpcUrl')
    setNewRpc(DEFAULT_RPC)
    setRpcUrl(DEFAULT_RPC)
    setRpcSaved(true)
  }

  const closeModal = () => {
    setModal(null)
    setShowSecret(false)
    setMnemonic('')
    setRpcSaved(false)
    setSendTo('')
    setSendAmount('')
    setSendResult(null)
  }

  const handleSend = async () => {
    if (!wallet) return
    const amount = parseFloat(sendAmount)
    if (!sendTo.trim() || isNaN(amount) || amount <= 0) return
    setSending(true)
    setSendResult(null)
    try {
      const result = await sendTransaction(rpcUrl, wallet.address, wallet.privateKey, sendTo.trim(), amount)
      if (result.success) {
        setSendResult({ ok: true, msg: t('home.sendSuccess', { fee: result.fee }) })
        fetchBalance()
        fetchTxHistory()
      } else {
        setSendResult({ ok: false, msg: result.error || t('home.sendTitle') })
      }
    } catch (e: any) {
      setSendResult({ ok: false, msg: e.message || 'Unknown error' })
    } finally {
      setSending(false)
    }
  }

  const handleChangeLanguage = async (code: string) => {
    await i18n.changeLanguage(code)
    await chrome.storage.local.set({ language: code })
    closeModal()
  }

  return (
    <div className="wallet-container">
      <div className="wallet-header">
        <h2>{t('home.title')}</h2>
        <div className="menu-wrapper" ref={menuRef}>
          <button className="btn-menu" onClick={() => setMenuOpen(!menuOpen)}>
            <span /><span /><span />
          </button>
          {menuOpen && (
            <div className="menu-dropdown">
              <button className="menu-item" onClick={handleViewRecovery}>
                <span className="menu-icon">🔑</span> {t('home.menuRecovery')}
              </button>
              <button className="menu-item" onClick={() => openMenu('privatekey')}>
                <span className="menu-icon">🗝️</span> {t('home.menuPrivateKey')}
              </button>
              <button className="menu-item" onClick={handleOpenRpc}>
                <span className="menu-icon">🌐</span> {t('home.menuRpc')}
              </button>
<button className="menu-item" onClick={() => openMenu('language')}>
                <span className="menu-icon">🌍</span> {t('language')}
              </button>
              <div className="menu-divider" />
              <button className="menu-item menu-item-danger" onClick={() => { clearWallet(); onLogout() }}>
                <span className="menu-icon">🚪</span> {t('home.menuLogout')}
              </button>
            </div>
          )}
        </div>
      </div>

      <div className="wallet-card">
        <div className="balance-section">
          <p className="balance-label">{t('home.balance')}</p>
          <h1 className="balance-amount">{balance} <span>ASRM</span></h1>
          <div className="network-badge">
            <span className="network-dot" style={{
              background: rpcOnline === false ? '#ef4444' : '#22c55e',
              boxShadow: `0 0 6px ${rpcOnline === false ? '#ef4444' : '#22c55e'}`
            }} />
            {t('home.network')}
          </div>
        </div>

        <div className="action-buttons">
          <button className="btn-receive" onClick={() => setModal('receive')}>
            <span className="btn-action-icon">↓</span>
            {t('home.receiveTitle')}
          </button>
          <button className="btn-send" onClick={() => { setModal('send'); setSendResult(null) }}>
            <span className="btn-action-icon">↑</span>
            {t('home.sendTitle')}
          </button>
        </div>
      </div>

      {/* Transaction History */}
      <div className="tx-history-section">
        <h3 className="tx-history-title">{t('home.txHistoryTitle')}</h3>
        {txHistory.length === 0 ? (
          <p className="tx-history-empty">{t('home.txHistoryEmpty')}</p>
        ) : (
          <ul className="tx-history-list">
            {txHistoryRendered}
          </ul>
        )}
      </div>

      {/* Modal overlay */}
      {modal && (
        <div className="modal-overlay" onClick={closeModal}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>

            {modal === 'recovery' && (
              <>
                <h3 className="modal-title">{t('home.recoveryTitle')}</h3>
                <p className="modal-warning">{t('home.recoveryWarning')}</p>
                {!showSecret ? (
                  <button className="btn-reveal" onClick={() => setShowSecret(true)}>
                    {t('reveal')}
                  </button>
                ) : mnemonic ? (
                  <div className="mnemonic-grid">
                    {mnemonic.split(' ').map((word, i) => (
                      <div key={i} className="mnemonic-word-item">
                        <span className="word-num">{i + 1}</span>
                        <span className="word-val">{word}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="modal-empty">{t('home.recoveryNotFound')}</p>
                )}
                <button className="btn-modal-close" onClick={closeModal}>{t('close')}</button>
              </>
            )}

            {modal === 'privatekey' && (
              <>
                <h3 className="modal-title">{t('home.privateKeyTitle')}</h3>
                <p className="modal-warning">{t('home.privateKeyWarning')}</p>
                {!showSecret ? (
                  <button className="btn-reveal" onClick={() => setShowSecret(true)}>
                    {t('reveal')}
                  </button>
                ) : (
                  <div className="secret-box">
                    <code className="secret-text">{wallet?.privateKey}</code>
                    <button className="btn-copy-small" onClick={() => {
                      navigator.clipboard.writeText(wallet?.privateKey || '')
                      alert(t('home.privateKeyCopied'))
                    }}>{t('copy')}</button>
                  </div>
                )}
                <button className="btn-modal-close" onClick={closeModal}>{t('close')}</button>
              </>
            )}

            {modal === 'rpc' && (
              <>
                <h3 className="modal-title">{t('home.rpcTitle')}</h3>
                <p className="modal-label">{t('home.rpcCurrentLabel')}</p>
                <p className="modal-current-rpc">{rpcUrl}</p>
                <p className="modal-label">{t('home.rpcNewLabel')}</p>
                <input
                  className="rpc-input"
                  value={newRpc}
                  onChange={(e) => { setNewRpc(e.target.value); setRpcSaved(false) }}
                  placeholder="https://rpc.example.com"
                />
                {rpcSaved && <p className="modal-success">{t('home.rpcSaved')}</p>}
                <div className="modal-buttons">
                  <button className="btn-primary-sm" onClick={handleSaveRpc}>{t('home.rpcSaveBtn')}</button>
                  <button className="btn-secondary-sm" onClick={handleResetRpc}>{t('home.rpcResetBtn')}</button>
                  <button className="btn-secondary-sm" onClick={closeModal}>{t('close')}</button>
                </div>
              </>
            )}

            {modal === 'receive' && (
              <>
                <h3 className="modal-title">{t('home.receiveTitle')}</h3>
                <p className="receive-hint">{t('home.receiveHint')}</p>
                <div className="receive-address-card">
                  <div className="receive-address-chunks">
                    {wallet?.address.match(/.{1,6}/g)?.map((chunk, i) => (
                      <span key={i} className="receive-address-chunk">{chunk}</span>
                    ))}
                  </div>
                  <div className="receive-address-full">{wallet?.address}</div>
                </div>
                <button
                  className={`btn-copy-address${addrCopied ? ' btn-copy-address--copied' : ''}`}
                  onClick={handleCopyReceive}
                >
                  {addrCopied ? t('home.receiveCopied') : t('home.receiveCopyBtn')}
                </button>
                <button className="btn-modal-close" onClick={closeModal}>{t('close')}</button>
              </>
            )}

            {modal === 'send' && (
              <>
                <h3 className="modal-title">{t('home.sendTitle')}</h3>
                {sendResult ? (
                  <>
                    <p className={sendResult.ok ? 'modal-success' : 'modal-error'}>
                      {sendResult.msg}
                    </p>
                    {sendResult.ok ? (
                      <button className="btn-modal-close" onClick={closeModal}>{t('close')}</button>
                    ) : (
                      <div className="modal-buttons">
                        <button className="btn-primary-sm" onClick={() => setSendResult(null)}>{t('home.sendRetry')}</button>
                        <button className="btn-secondary-sm" onClick={closeModal}>{t('close')}</button>
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    <p className="modal-label">{t('home.sendToLabel')}</p>
                    <input
                      className="rpc-input"
                      value={sendTo}
                      onChange={(e) => setSendTo(e.target.value)}
                      placeholder="0x..."
                      disabled={sending}
                    />
                    <p className="modal-label">{t('home.sendAmountLabel')}</p>
                    <input
                      className="rpc-input"
                      type="number"
                      min="0"
                      step="0.000001"
                      value={sendAmount}
                      onChange={(e) => setSendAmount(e.target.value)}
                      placeholder={t('home.sendAmountPlaceholder')}
                      disabled={sending}
                    />
                    <p className="modal-fee-note">{t('home.sendFeeNote')}</p>
                    <div className="modal-buttons">
                      <button
                        className="btn-primary-sm btn-send-submit"
                        onClick={handleSend}
                        disabled={sending || !sendTo.trim() || !sendAmount}
                      >
                        {sending ? t('home.sending') : t('home.sendBtn')}
                      </button>
                      <button className="btn-secondary-sm" onClick={closeModal} disabled={sending}>{t('cancel')}</button>
                    </div>
                  </>
                )}
              </>
            )}

            {modal === 'language' && (
              <>
                <h3 className="modal-title">🌍 {t('language')}</h3>
                <ul className="language-list">
                  {LANGUAGES.map((lang) => (
                    <li key={lang.code}>
                      <button
                        className={`language-item ${i18n.language === lang.code ? 'language-item-active' : ''}`}
                        onClick={() => handleChangeLanguage(lang.code)}
                      >
                        <span className="language-flag">{lang.flag}</span>
                        <span className="language-label">{lang.label}</span>
                        {i18n.language === lang.code && <span className="language-check">✓</span>}
                      </button>
                    </li>
                  ))}
                </ul>
                <button className="btn-modal-close" onClick={closeModal}>{t('close')}</button>
              </>
            )}


          </div>
        </div>
      )}
    </div>
  )
}
