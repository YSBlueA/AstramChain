// Background Service Worker
chrome.runtime.onInstalled.addListener(() => {
  console.log('AstramX Wallet extension installed')
})

// Open side panel when extension icon is clicked
chrome.action.onClicked.addListener((tab) => {
  chrome.sidePanel.open({ tabId: tab.id })
})

chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  // 내부 직접 호출 (팝업 등)
  if (request.type === 'GET_WALLET') {
    chrome.storage.local.get('wallet', (result) => {
      sendResponse(result.wallet || null)
    })
    return true
  }

  // dApp 페이지에서 content-script 경유로 오는 요청
  if (request.type === 'WALLET_REQUEST') {
    const method = request.payload?.method

    if (method === 'getAccount') {
      chrome.storage.local.get('wallet', (result) => {
        const wallet = result.wallet
        sendResponse({
          result: wallet?.address ?? null,
        })
      })
      return true
    }

    if (method === 'getBalance') {
      chrome.storage.local.get('wallet', (result) => {
        sendResponse({ result: result.wallet?.balance ?? '0' })
      })
      return true
    }

    // 알 수 없는 method
    sendResponse({ error: `Unknown method: ${method}` })
  }

  if (request.type === 'SIGN_TRANSACTION') {
    // TODO: Implement transaction signing
    sendResponse({ signed: false })
  }
})
