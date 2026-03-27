// Content Script - Runs on web pages
console.log('AstramX Wallet content script loaded')

// inject.js 를 페이지 컨텍스트에 주입 (window.astramWallet 노출)
const script = document.createElement('script')
script.src = chrome.runtime.getURL('inject.js')
script.onload = () => script.remove()
;(document.head || document.documentElement).appendChild(script)

window.addEventListener('message', (event) => {
  // Only accept messages from the same frame
  if (event.source !== window) return

  if (event.data.type && event.data.type === 'ASTRAM_REQUEST') {
    chrome.runtime.sendMessage(
      { type: 'WALLET_REQUEST', payload: event.data.payload },
      (response) => {
        window.postMessage({
          type: 'ASTRAM_RESPONSE',
          payload: response,
        }, '*')
      }
    )
  }
})
