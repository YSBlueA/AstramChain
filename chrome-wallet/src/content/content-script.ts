// Content Script - Runs on web pages
console.log('Astram Wallet content script loaded')

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
