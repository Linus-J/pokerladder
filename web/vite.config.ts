import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

// In development the Vite server proxies the API and WebSocket to the Rust server.
// Run the server with PUBLIC_URL=http://localhost:5173 so the WebSocket Origin check
// and email links match this dev origin.
const backend = process.env.BACKEND ?? 'http://127.0.0.1:8080'

export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': backend,
      '/ws': { target: backend, ws: true },
    },
  },
  build: { sourcemap: true },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
})
