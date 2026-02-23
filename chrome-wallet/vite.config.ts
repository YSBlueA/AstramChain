import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'
import fs from 'fs'

// Plugin to copy manifest.json and other static files
const copyAssetsPlugin = {
  name: 'copy-assets',
  writeBundle() {
    // Copy manifest.json
    fs.copyFileSync('manifest.json', 'dist/manifest.json')
    
    // Read side-panel.html and update script src
    let sidePanelHtml = fs.readFileSync('src/side-panel.html', 'utf-8')
    sidePanelHtml = sidePanelHtml.replace(
      /src="\/src\/side-panel\.tsx"/g,
      'src="./side-panel.js"'
    )
    fs.writeFileSync('dist/side-panel.html', sidePanelHtml)
    
    // Create assets directory if it doesn't exist
    const assetsDir = 'dist/assets'
    if (!fs.existsSync(assetsDir)) {
      fs.mkdirSync(assetsDir, { recursive: true })
    }
    
    console.log('✓ Copied manifest.json, side-panel.html and created assets directory')
  },
}

export default defineConfig({
  plugins: [react(), copyAssetsPlugin],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      buffer: 'buffer',
    },
  },
  define: {
    'global': 'globalThis',
  },
  optimizeDeps: {
    esbuildOptions: {
      define: {
        global: 'globalThis',
      },
    },
  },
  build: {
    outDir: 'dist',
    minify: 'terser',
    rollupOptions: {
      input: {
        'side-panel': 'src/side-panel.html',
        background: 'src/background/service-worker.ts',
        content: 'src/content/content-script.ts',
        inject: 'src/inject/inject.js',
      },
      output: {
        entryFileNames: '[name].js',
        chunkFileNames: '[name].js',
        assetFileNames: '[name].[ext]',
      },
    },
  },
})
