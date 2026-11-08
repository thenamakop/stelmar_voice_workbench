import { defineConfig, devices } from "@playwright/test";

// Runs the web UI (Vite build with a mocked Tauri bridge, see src/test/e2e-entry.ts) in Chromium
// at the 1536x864 screenshot size used for design review.
const PORT = 4173;

export default defineConfig({
  testDir: "e2e",
  outputDir: "test-results",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    viewport: { width: 1536, height: 864 },
    deviceScaleFactor: 1,
    locale: "en-IN",
    timezoneId: "Asia/Kolkata",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1536, height: 864 } } },
  ],
  webServer: {
    command: `node scripts/copy-icons.mjs && pnpm exec vite build --mode e2e &&pnpm exec vite preview --mode e2e --outDir dist-e2e --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}/e2e/index.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
