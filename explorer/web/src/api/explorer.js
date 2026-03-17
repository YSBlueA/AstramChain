import axios from 'axios'

const runtimeConfig =
  typeof window !== 'undefined' && window.ASTRAM_EXPLORER_CONF
    ? window.ASTRAM_EXPLORER_CONF
    : {}

const API_BASE_URL =
  runtimeConfig.apiBaseUrl ||
  import.meta.env.VITE_API_BASE_URL ||
  'http://localhost:8080/api'

export const explorerAPI = {
  // Block endpoints
  getBlocks(page = 1, limit = 20) {
    return axios.get(`${API_BASE_URL}/blocks`, {
      params: { page, limit }
    })
  },

  getBlockByHeight(height) {
    return axios.get(`${API_BASE_URL}/blocks/${height}`)
  },

  getBlockByHash(hash) {
    return axios.get(`${API_BASE_URL}/blocks/hash/${hash}`)
  },

  // Transaction endpoints
  getTransactions(page = 1, limit = 20) {
    return axios.get(`${API_BASE_URL}/transactions`, {
      params: { page, limit }
    })
  },

  getTransactionByHash(hash) {
    return axios.get(`${API_BASE_URL}/transactions/${hash}`)
  },

  // Statistics
  getStats() {
    return axios.get(`${API_BASE_URL}/stats`)
  },

  // Address
  getAddressInfo(address) {
    return axios.get(`${API_BASE_URL}/address/${address}`)
  },

  // Health check
  health() {
    return axios.get(`${API_BASE_URL}/health`)
  },

  // Node status
  getNodeStatus() {
    return axios.get(`${API_BASE_URL}/node/status`)
  },

  // Rich list
  getRichlist(limit = 50) {
    return axios.get(`${API_BASE_URL}/richlist`, { params: { limit } })
  },
}
