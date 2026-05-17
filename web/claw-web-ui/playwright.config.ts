import { defineConfig } from "@playwright/test";

const webPort = process.env.CLAW_WEB_UI_PORT ?? "4100";
const baseURL = process.env.CLAW_WEB_UI_BASE_URL ?? `http://127.0.0.1:${webPort}`;

/** E2E uses system Google Chrome — no Playwright browser download. Author: kejiqing */
export default defineConfig({
  testDir: "./e2e",
  timeout: 120_000,
  expect: { timeout: 90_000 },
  fullyParallel: false,
  retries: 0,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL,
    channel: "chrome",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chrome", use: { channel: "chrome" } }],
});
