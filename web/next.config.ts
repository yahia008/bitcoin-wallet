import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Build to plain HTML/JS files (`out/`) with no Next.js server. Wallet secrets only ever
  // live in the browser, so there's nothing for a server to do, and nothing to leak them.
  output: "export",
};

export default nextConfig;
