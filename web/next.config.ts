import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Build to plain HTML/JS files (`out/`) with no Next.js server. Wallet secrets only ever
  // live in the browser, so there's nothing for a server to do, and nothing to leak them.
  output: "export",
  // Write each page as <route>/index.html (e.g. terms/index.html), so any plain static file
  // server finds /terms/ without extra configuration.
  trailingSlash: true,
  // The end-to-end tests build into their own folder (see e2e/), so they never overwrite the
  // normal build in out/.
  ...(process.env.NEXT_DIST_DIR ? { distDir: process.env.NEXT_DIST_DIR } : {}),
};

export default nextConfig;
