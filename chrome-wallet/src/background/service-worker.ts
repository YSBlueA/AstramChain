// Background Service Worker
chrome.runtime.onInstalled.addListener(() => {
  console.log('Astram Wallet extension installed')
})

// Open side panel when extension icon is clicked
chrome.action.onClicked.addListener((tab) => {
  chrome.sidePanel.open({ tabId: tab.id })
})

chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  if (request.type === 'GET_WALLET') {
    chrome.storage.local.get('wallet', (result) => {
      sendResponse(result.wallet || null)
    })
    return true
  }

  if (request.type === 'SIGN_TRANSACTION') {
    // TODO: Implement transaction signing
    sendResponse({ signed: false })
  }
})
