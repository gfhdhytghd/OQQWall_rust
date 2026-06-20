import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const webviewBackend = process.env.VITE_WEBVIEW_BACKEND ?? 'http://127.0.0.1:10924'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/auth': {
        target: webviewBackend,
        changeOrigin: true,
      },
      '/api': {
        target: webviewBackend,
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
  },
})
