import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'

export default defineConfig({
  plugins: [
    react(),
    VitePWA({
      registerType: 'autoUpdate',
      manifest: {
        name: 'Hatchdoor',
        short_name: 'Hatchdoor',
        description: 'Read-only Obsidian vault web frontend',
        start_url: '/',
        display: 'standalone',
        background_color: '#f4f2ec',
        theme_color: '#ece8da',
        icons: [
          {
            src: '/vite.svg',
            sizes: 'any',
            type: 'image/svg+xml',
            purpose: 'any',
          },
        ],
      },
      workbox: {
        globPatterns: ['**/*.{js,css,html,svg,png,ico,woff2}'],
      },
    }),
  ],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:42824',
      '/health': 'http://127.0.0.1:42824',
    },
  },
})
