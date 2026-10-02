// End-to-end tests: a real browser clicking through the wallet, against a throwaway API
// server and the regtest node. Run with `npm run e2e` (the node must be up:
// `docker compose up -d bitcoind` in the repo root). See e2e/README.md.

import { defineConfig } from "@playwright/test";

const API = "http://127.0.0.1:3925";
const SITE = "http://127.0.0.1:3002";

export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  // One at a time: the tests share one node and mine blocks.
  workers: 1,
  fullyParallel: false,
  timeout: 120_000,
  expect: { timeout: 30_000 },
  reporter: [["list"], ["html", { open: "never", outputFolder: "e2e-report" }]],
  outputDir: "e2e-results",
  use: {
    baseURL: SITE,
    viewport: { width: 1280, height: 900 },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: [
    {
      // A fresh API server on its own port and data dir, so test wallets never mix with yours.
      // Limits are lifted: the tests register many wallets quickly.
      command:
        "rm -rf web/e2e-server-data && cargo run -q --bin server -- --network regtest " +
        "--data-dir web/e2e-server-data --listen 127.0.0.1:3925 " +
        `--cors-origins ${SITE} --sync-interval-secs 0 ` +
        "--rate-limit-per-minute 100000 --register-limit-per-hour 100000",
      cwd: "..",
      url: `${API}/health`,
      timeout: 600_000,
      reuseExistingServer: false,
    },
    {
      // The site built against that server, into e2e-build/ (not out/), served statically.
      command: "npm run build && node e2e/serve.mjs e2e-build 3002",
      env: { NEXT_DIST_DIR: "e2e-build", NEXT_PUBLIC_API_URL: API },
      url: SITE,
      timeout: 900_000,
      reuseExistingServer: false,
    },
  ],
});
