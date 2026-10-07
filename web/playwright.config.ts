import { defineConfig, devices } from "@playwright/test";

// The tour in the browser (CI Etappe 7, after Rocket's frontend/e2e). It
// expects a running relay-server that serves web/build — in CI the job
// "oberflaeche" starts one; locally start it yourself and run `npm run e2e`.
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "e2e-bericht" }]],
  globalSetup: "./e2e/vorbereitung.ts",
  use: {
    baseURL: process.env.RELAY_URL ?? "http://127.0.0.1:3799",
    screenshot: "only-on-failure",
    // Where Chromium is preinstalled (Claude sessions) it lies here; in CI
    // the job installs the matching browser itself.
    launchOptions: process.env.RELAY_CHROMIUM ? { executablePath: process.env.RELAY_CHROMIUM } : {},
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1400, height: 900 } } },
    { name: "handy", use: { ...devices["Pixel 7"], viewport: { width: 390, height: 844 } } },
  ],
});
