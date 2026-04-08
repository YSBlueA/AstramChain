import React, { useState, useEffect } from 'react'
import { sendTransaction } from '@/utils/transaction'
import '../styles/TxApproval.css'

const DEFAULT_RPC = 'https://rpc.astramchain.com'

interface PendingTx {
  id: string
  to: string
  amount: number
}

interface StoredWallet {
  address: string
  privateKey: string
}

interface TxApprovalProps {
  pendingTx: PendingTx
  onDone: () => void
}

export function TxApproval({ pendingTx, onDone }: TxApprovalProps) {
  const [wallet, setWallet] = useState<StoredWallet | null>(null)
  const [rpcUrl, setRpcUrl] = useState(DEFAULT_RPC)
  const [status, setStatus] = useState<'idle' | 'loading' | 'done' | 'error'>('idle')
  const [message, setMessage] = useState('')

  useEffect(() => {
    chrome.storage.local.get(['wallet', 'rpcUrl']).then((data) => {
      const w = data.wallet as StoredWallet | undefined
      if (w) setWallet(w)
      if (data.rpcUrl) setRpcUrl(data.rpcUrl as string)
    })
  }, [])

  const handleApprove = async () => {
    if (!wallet) return
    setStatus('loading')
    try {
      const result = await sendTransaction(
        rpcUrl,
        wallet.address,
        wallet.privateKey,
        pendingTx.to,
        pendingTx.amount,
      )
      if (result.success) {
        await chrome.storage.local.set({
          txResult: { id: pendingTx.id, approved: true, hash: result.txid },
        })
        setStatus('done')
        setMessage(`전송 완료! 수수료: ${result.fee} ASRM`)
        setTimeout(onDone, 2000)
      } else {
        await chrome.storage.local.set({
          txResult: { id: pendingTx.id, approved: false },
        })
        setStatus('error')
        setMessage(result.error || '전송 실패')
        setTimeout(onDone, 2500)
      }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '알 수 없는 오류'
      await chrome.storage.local.set({
        txResult: { id: pendingTx.id, approved: false },
      })
      setStatus('error')
      setMessage(msg)
      setTimeout(onDone, 2500)
    }
  }

  const handleReject = async () => {
    await chrome.storage.local.set({
      txResult: { id: pendingTx.id, approved: false },
    })
    onDone()
  }

  const shortAddr = (addr: string) =>
    addr.length > 12 ? `${addr.slice(0, 8)}…${addr.slice(-6)}` : addr

  return (
    <div className="tx-approval-container">
      <div className="tx-approval-header">
        <div className="tx-approval-icon">📤</div>
        <h2 className="tx-approval-title">트랜잭션 서명 요청</h2>
        <p className="tx-approval-subtitle">dApp이 다음 트랜잭션 서명을 요청합니다</p>
      </div>

      {!wallet ? (
        <div className="tx-approval-no-wallet">
          <p>지갑이 잠겨 있습니다. 지갑을 먼저 열어주세요.</p>
          <button className="tx-btn-reject" onClick={handleReject}>거절</button>
        </div>
      ) : (
        <>
          <div className="tx-detail-card">
            <div className="tx-detail-row">
              <span className="tx-detail-label">보내는 주소</span>
              <span className="tx-detail-value tx-detail-mono">{shortAddr(wallet.address)}</span>
            </div>
            <div className="tx-detail-divider" />
            <div className="tx-detail-row">
              <span className="tx-detail-label">받는 주소</span>
              <span className="tx-detail-value tx-detail-mono">{shortAddr(pendingTx.to)}</span>
            </div>
            <div className="tx-detail-divider" />
            <div className="tx-detail-row">
              <span className="tx-detail-label">금액</span>
              <span className="tx-detail-value tx-detail-amount">{pendingTx.amount} ASRM</span>
            </div>
          </div>

          {status === 'idle' && (
            <div className="tx-action-buttons">
              <button className="tx-btn-reject" onClick={handleReject}>거절</button>
              <button className="tx-btn-approve" onClick={handleApprove}>승인</button>
            </div>
          )}

          {status === 'loading' && (
            <div className="tx-status-msg tx-status-loading">
              <span className="tx-spinner" />
              트랜잭션 전송 중...
            </div>
          )}

          {status === 'done' && (
            <div className="tx-status-msg tx-status-success">{message}</div>
          )}

          {status === 'error' && (
            <div className="tx-status-msg tx-status-error">{message}</div>
          )}
        </>
      )}
    </div>
  )
}
