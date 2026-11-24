import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30000,
  retries: 0,
  use: {
    baseURL: "http://localhost:1440",
    headless: true,
  },
  webServer: {
    command: "pnpm dev",
    port: 1440,
    reuseExistingServer: true,
    timeout: 30000,
  },
});
