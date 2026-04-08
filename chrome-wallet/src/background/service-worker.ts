// Background Service Worker

interface StoredWallet {
  address: string
  privateKey: string
  balance: string
}

interface TxResult {
  id: string
  approved: boolean
  hash?: string
}

chrome.runtime.onInstalled.addListener(() => {
  console.log('AstramX Wallet extension installed')
})

// Open side panel when extension icon is clicked
chrome.action.onClicked.addListener((tab) => {
  if (tab.id !== undefined) {
    chrome.sidePanel.open({ tabId: tab.id, windowId: tab.windowId })
  }
})

chrome.runtime.onMessage.addListener((request, _sender, sendResponse) => {
  // 내부 직접 호출 (팝업 등)
  if (request.type === 'GET_WALLET') {
    chrome.storage.local.get('wallet', (result) => {
      sendResponse((result.wallet as StoredWallet) || null)
    })
    return true
  }

  // dApp 페이지에서 content-script 경유로 오는 요청
  if (request.type === 'WALLET_REQUEST') {
    const method = request.payload?.method

    if (method === 'getAccount') {
      chrome.storage.local.get('wallet', (result) => {
        const wallet = result.wallet as StoredWallet | undefined
        sendResponse({ result: wallet?.address ?? null })
      })
      return true
    }

    if (method === 'getBalance') {
      chrome.storage.local.get('wallet', (result) => {
        const wallet = result.wallet as StoredWallet | undefined
        sendResponse({ result: wallet?.balance ?? '0' })
      })
      return true
    }

    if (method === 'signTransaction') {
      const tx = request.payload.transaction

      chrome.storage.local.get('pendingTx', (existing) => {
        if (existing.pendingTx) {
          sendResponse({ error: 'Another transaction is pending approval' })
          return
        }

        const requestId = Date.now().toString()
        chrome.storage.local.set({ pendingTx: { id: requestId, ...tx } })

        // 사이드패널 열기
        chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
          const tab = tabs[0]
          if (tab?.id !== undefined && tab.windowId !== undefined) {
            chrome.sidePanel.open({ tabId: tab.id, windowId: tab.windowId })
          }
        })

        // 5분 타임아웃
        const timeoutId = setTimeout(() => {
          chrome.storage.onChanged.removeListener(listener)
          chrome.storage.local.remove('pendingTx')
          sendResponse({ error: 'Transaction approval timed out' })
        }, 5 * 60 * 1000)

        const listener = (changes: { [key: string]: chrome.storage.StorageChange }) => {
          if (!changes.txResult) return
          clearTimeout(timeoutId)
          chrome.storage.onChanged.removeListener(listener)
          const result = changes.txResult.newValue as TxResult | undefined
          if (!result || result.id !== requestId) return
          chrome.storage.local.remove(['pendingTx', 'txResult'])
          if (result.approved) {
            sendResponse({ result: { hash: result.hash } })
          } else {
            sendResponse({ error: 'User rejected the transaction' })
          }
        }

        chrome.storage.onChanged.addListener(listener)
      })
      return true
    }

    // 알 수 없는 method
    sendResponse({ error: `Unknown method: ${method}` })
  }
})
