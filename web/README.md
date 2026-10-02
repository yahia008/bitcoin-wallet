# Web wallet

The browser front end of the wallet: Next.js (App Router) + TypeScript + Tailwind, built as a
static export. Keys will live only in the browser; the Rust API server in `../src/bin/server.rs`
is watch-only.

```bash
npm install
npm run dev     # http://localhost:3001 (the API server uses :3000)
npm run build   # static files in out/
```

Both first run `npm run wasm`, which compiles `../wallet-wasm` (the Rust wallet code) to
WebAssembly in `src/wasm/` (generated, not committed). That needs `wasm-pack` and `clang`.
It uses the size-optimised `wasm` Cargo profile (opt-level z, LTO) and secp256k1's
`lowmemory` tables, which keep the download to about 1.3 MB, or 0.5 MB gzipped.

`NEXT_PUBLIC_API_URL` sets the API server (default `http://127.0.0.1:3000`). It's read at build
time. The API server must list this page's origin in `--cors-origins`.

## End-to-end tests

```bash
docker compose up -d bitcoind   # in the repo root: the regtest node
npm run e2e            # build, start a throwaway API server, click through every flow
```

The tests are in `e2e/`; see `../test.md` for what they cover and the one-time setup
(`npx playwright install chromium` and `sudo npx playwright install-deps chromium`).
