export interface Wallet {
  address: string
  privateKey: string
  balance: string
}

export interface Transaction {
  from: string
  to: string
  value: string
  nonce?: number
  gasPrice?: string
  gasLimit?: string
  data?: string
}

export interface SignedTransaction {
  transaction: Transaction
  signature: string
  hash: string
}

export interface AstramWalletAPI {
  getBalance: (address: string) => Promise<string>
  signTransaction: (tx: Transaction) => Promise<SignedTransaction>
}

declare global {
  interface Window {
    astramWallet: AstramWalletAPI
  }
}
