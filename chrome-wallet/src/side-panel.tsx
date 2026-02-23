import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'

console.log('Side panel loading...')

const root = document.getElementById('root')
console.log('Root element:', root)

if (!root) throw new Error('Root element not found')

console.log('Creating React root...')
ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
console.log('React app mounted')
