import { create } from 'zustand'

export interface Wallet {
  address: string
  privateKey: string
  balance: string
}

interface WalletStore {
  wallet: Wallet | null
  initWallet: (wallet: Wallet) => void
  updateBalance: (balance: string) => void
  clearWallet: () => void
}

export const useWalletStore = create<WalletStore>((set) => ({
  wallet: null,
  initWallet: (wallet: Wallet) => {
    set({ wallet })
    chrome.storage.local.set({ wallet })
  },
  updateBalance: (balance: string) => set((state) => ({
    wallet: state.wallet ? { ...state.wallet, balance } : null
  })),
  clearWallet: () => {
    set({ wallet: null })
    chrome.storage.local.remove('wallet')
  }
}))
