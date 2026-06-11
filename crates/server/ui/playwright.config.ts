import { defineConfig, devices } from "@playwright/test";

// End-to-end tests drive the real production bundle in a browser. The hub's
// `/v1` + `/auth` API is mocked per-test (route interception) — the SPA can't
// complete the cookie/SSO login flow headlessly, so this is frontend E2E
// (routing, rendering, i18n/theme/density, command palette, role gating), not a
// full-stack test. `vite preview` serves the built assets under the `/app/` base.
const PORT = 4173;

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "list" : "line",
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `pnpm build && pnpm preview --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}/app/`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
