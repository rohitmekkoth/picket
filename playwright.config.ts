import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  forbidOnly: !!process.env.CI,
  retries: 0,
  // Serialize projects so visual artifacts do not contend for graphics resources.
  workers: 1,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  // Cover the default and minimum native window sizes in both themes, and WebKit behavior.
  projects: [
    { name: "desktop", use: { browserName: "chromium", viewport: { width: 1040, height: 740 } } },
    {
      name: "desktop-dark",
      use: { browserName: "chromium", viewport: { width: 1040, height: 740 }, colorScheme: "dark" },
    },
    { name: "webkit", use: { browserName: "webkit", viewport: { width: 1040, height: 740 } } },
    {
      name: "compact",
      use: { browserName: "chromium", viewport: { width: 640, height: 580 }, colorScheme: "dark" },
    },
    {
      name: "compact-light",
      use: { browserName: "chromium", viewport: { width: 640, height: 580 } },
    },
  ],
  // Reuse the local server for iteration but isolate every CI run.
  webServer: {
    command: "pnpm dev:ui",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
  },
});
