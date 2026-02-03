import axios from 'axios'

const API_BASE_URL = 'http://localhost:8080/api'

export const explorerAPI = {
  // 블록 관련
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

  // 트랜잭션 관련
  getTransactions(page = 1, limit = 20) {
    return axios.get(`${API_BASE_URL}/transactions`, {
      params: { page, limit }
    })
  },

  getTransactionByHash(hash) {
    return axios.get(`${API_BASE_URL}/transactions/${hash}`)
  },

  // 통계
  getStats() {
    return axios.get(`${API_BASE_URL}/stats`)
  },

  // 주소
  getAddressInfo(address) {
    return axios.get(`${API_BASE_URL}/address/${address}`)
  },

  // 헬스 체크
  health() {
    return axios.get(`${API_BASE_URL}/health`)
  },
}
