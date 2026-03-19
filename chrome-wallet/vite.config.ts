import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'
import fs from 'fs'
import sharp from 'sharp'

// Plugin to copy manifest.json and other static files
const copyAssetsPlugin = {
  name: 'copy-assets',
  async writeBundle() {
    // Copy manifest.json
    fs.copyFileSync('manifest.json', 'dist/manifest.json')

    // Vite가 생성한 dist/src/side-panel.html 기반으로 경로 수정
    let sidePanelHtml = fs.readFileSync('dist/src/side-panel.html', 'utf-8')
    sidePanelHtml = sidePanelHtml
      .replace(/src="\.\.\/side-panel\.js"/g, 'src="./side-panel.js"')
      .replace(/href="\.\.\/assets\//g, 'href="./assets/')
    fs.writeFileSync('dist/side-panel.html', sidePanelHtml)

    // Create icons directory and generate resized icons
    const iconsDir = 'dist/icons'
    if (!fs.existsSync(iconsDir)) {
      fs.mkdirSync(iconsDir, { recursive: true })
    }
    const srcIcon = 'src/assets/astram_logo_no_background.png'
    for (const size of [16, 32, 48, 128]) {
      await sharp(srcIcon).resize(size, size).png().toFile(`${iconsDir}/icon${size}.png`)
    }

    // 배경 이미지 400x600으로 리사이징
    const bgSrc = 'src/assets/chrome_wallet_logo_1024_1536.png'
    const bgDest = 'dist/assets/chrome_wallet_logo_1024_1536.png'
    await sharp(bgSrc).resize(400, 600).png().toFile(bgDest)

    console.log('✓ Copied manifest.json, side-panel.html, generated icons, resized bg')
  },
}

export default defineConfig({
  plugins: [react(), copyAssetsPlugin],
  base: '',
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
    emptyOutDir: true,
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
        assetFileNames: 'assets/[name].[ext]',
      },
    },
  },
})
