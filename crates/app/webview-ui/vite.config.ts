import { createReadStream, cpSync, existsSync, statSync } from 'node:fs'
import { createRequire } from 'node:module'
import { extname, resolve } from 'node:path'
import { dirname, join, sep } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const webviewBackend = process.env.VITE_WEBVIEW_BACKEND ?? 'http://127.0.0.1:10924'
const require = createRequire(import.meta.url)
const scratchBlocksEntry = require.resolve('scratch-blocks')
const scratchBlocksMediaDir = resolve(dirname(scratchBlocksEntry), '..', 'media')
const scratchBlocksMediaRoute = '/scratch-blocks-media/'

function scratchBlocksMediaPlugin(): Plugin {
  let outputDir = resolve('dist')

  return {
    name: 'oqqwall-scratch-blocks-media',
    configResolved(config) {
      outputDir = resolve(config.root, config.build.outDir)
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const requestUrl = request.url ? new URL(request.url, 'http://localhost') : null
        const pathname = requestUrl?.pathname ?? ''
        if (!pathname.startsWith(scratchBlocksMediaRoute)) {
          next()
          return
        }

        const relativePath = decodeURIComponent(pathname.slice(scratchBlocksMediaRoute.length))
        const mediaPath = resolve(scratchBlocksMediaDir, relativePath)
        if (!mediaPath.startsWith(`${scratchBlocksMediaDir}${sep}`)) {
          response.statusCode = 403
          response.end()
          return
        }

        if (!existsSync(mediaPath) || !statSync(mediaPath).isFile()) {
          response.statusCode = 404
          response.end()
          return
        }

        response.setHeader('Content-Type', contentTypeFor(mediaPath))
        createReadStream(mediaPath).pipe(response)
      })
    },
    writeBundle() {
      cpSync(scratchBlocksMediaDir, join(outputDir, scratchBlocksMediaRoute.slice(1)), {
        recursive: true,
      })
    },
  }
}

function contentTypeFor(pathname: string) {
  switch (extname(pathname).toLowerCase()) {
    case '.svg':
      return 'image/svg+xml'
    case '.gif':
      return 'image/gif'
    case '.png':
      return 'image/png'
    case '.cur':
      return 'image/x-icon'
    case '.mp3':
      return 'audio/mpeg'
    case '.ogg':
      return 'audio/ogg'
    case '.wav':
      return 'audio/wav'
    default:
      return 'application/octet-stream'
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), scratchBlocksMediaPlugin()],
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
