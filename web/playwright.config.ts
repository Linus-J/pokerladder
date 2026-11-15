import { defineConfig } from '@playwright/test'

// End-to-end: the real server binary serving the built client (`npm run build` first,
// and `cargo build -p pokerladder-server`), against the dev Postgres from compose.yaml.
// Set CHROMIUM to use a system browser instead of `npx playwright install chromium`.
const port = 18080
const base = `http://127.0.0.1:${port}`

export default defineConfig({
  testDir: 'e2e',
  timeout: 120_000,
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  use: {
    baseURL: base,
    trace: 'retain-on-failure',
    launchOptions: process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {},
  },
  webServer: {
    command: '../target/debug/pokerladder-server',
    url: `${base}/api/leaderboard`,
    timeout: 60_000,
    reuseExistingServer: false,
    stdout: 'pipe',
    env: {
      DATABASE_URL:
        process.env.DATABASE_URL ?? 'postgres://pokerladder:pokerladder@localhost:55432/pokerladder',
      BIND_ADDR: `127.0.0.1:${port}`,
      PUBLIC_URL: base,
      REQUIRE_EMAIL_VERIFICATION: 'false',
      STATIC_DIR: 'dist',
      RUST_LOG: 'warn',
    },
  },
})
