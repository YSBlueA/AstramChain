// Injected Script - Runs in page context
window.astramWallet = {
  getBalance: async (address) => {
    return new Promise((resolve, reject) => {
      window.postMessage({
        type: 'ASTRAM_REQUEST',
        payload: {
          method: 'getBalance',
          address,
        },
      }, '*')

      window.addEventListener('message', (event) => {
        if (
          event.data.type === 'ASTRAM_RESPONSE' &&
          event.data.payload?.result
        ) {
          resolve(event.data.payload.result)
        } else if (event.data.type === 'ASTRAM_RESPONSE') {
          reject(new Error(event.data.payload?.error || 'Unknown error'))
        }
      })
    })
  },

  signTransaction: async (tx) => {
    return new Promise((resolve, reject) => {
      window.postMessage({
        type: 'ASTRAM_REQUEST',
        payload: {
          method: 'signTransaction',
          transaction: tx,
        },
      }, '*')

      window.addEventListener('message', (event) => {
        if (
          event.data.type === 'ASTRAM_RESPONSE' &&
          event.data.payload?.result
        ) {
          resolve(event.data.payload.result)
        } else if (event.data.type === 'ASTRAM_RESPONSE') {
          reject(new Error(event.data.payload?.error || 'Unknown error'))
        }
      })
    })
  },
}

console.log('Astram Wallet injected to page')
