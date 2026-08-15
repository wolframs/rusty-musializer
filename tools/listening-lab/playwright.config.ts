import { defineConfig } from '@playwright/test'
import { resolve } from 'node:path'

const repoRoot = resolve(new URL('.', import.meta.url).pathname, '../..')
const testRoot = resolve(repoRoot, 'build/listening-lab-e2e')

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  reporter: [['list']],
  globalSetup: './tests/global-setup.ts',
  use: {
    baseURL: 'http://127.0.0.1:4181',
    headless: true,
    browserName: 'chromium',
    launchOptions: {
      executablePath: process.env.CHROMIUM_PATH || '/snap/bin/chromium',
      args: ['--mute-audio', '--autoplay-policy=no-user-gesture-required'],
    },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'npx vite --host 127.0.0.1 --port 4181',
    url: 'http://127.0.0.1:4181/api/protocols',
    reuseExistingServer: false,
    timeout: 30_000,
    env: {
      LISTENING_LAB_PROTOCOLS: resolve(testRoot, 'protocols'),
      LISTENING_LAB_ANSWERS: resolve(testRoot, 'answers'),
    },
  },
})
