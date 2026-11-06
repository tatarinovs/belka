import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  use: { baseURL: "http://127.0.0.1:1422", trace: "retain-on-failure" },
  webServer: {
    command: "npm run dev -- --port 1422",
    url: "http://127.0.0.1:1422",
    reuseExistingServer: false,
  },
});
