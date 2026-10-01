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

`NEXT_PUBLIC_API_URL` sets the API server (default `http://127.0.0.1:3000`). It's read at build
time. The API server must list this page's origin in `--cors-origins`.
