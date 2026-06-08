import React from 'react'
import { createRoot } from 'react-dom/client'
import '@heroui/react/styles'
import './styles.css'
import App from './App'

createRoot(document.getElementById('app') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
