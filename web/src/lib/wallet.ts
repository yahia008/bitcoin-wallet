// Loads the Rust wallet code (wallet-core, via wallet-wasm) compiled to WebAssembly. It's the
// same key handling, PSBT review and signing the CLI uses, so the browser doesn't need a
// second implementation of the security checks. `npm run wasm` generates src/wasm/.

import init, * as wasm from "@/wasm/wallet_wasm";

let ready: Promise<typeof wasm> | null = null;

/** Fetches and starts the WASM module once; later calls reuse it. The bundler serves the
 * .wasm file next to the generated wrapper (it finds it via `new URL(..., import.meta.url)`). */
export function loadWallet(): Promise<typeof wasm> {
  ready ??= init().then(() => wasm);
  return ready;
}

/** What `createWallet` returns (wallet-wasm's `NewWallet`). */
export type NewWallet = {
  walletId: string;
  network: string;
  external: string;
  internal: string;
  firstAddress: string;
  encryptedKey: { salt: string; nonce: string; ciphertext: string };
};

/** Derives the account key from `words`, encrypts it with `password` (Argon2id, ~0.2 s),
 * and returns that plus the public parts. Throws on a bad phrase or short password. */
export async function createWallet(
  words: string,
  password: string,
  network: string,
): Promise<NewWallet> {
  return (await loadWallet()).createWallet(words, password, network);
}

/** Throws unless `address` really is the wallet's receive address at `index`, derived here
 * from the public descriptor. Never show an address the server gave us without this: a
 * compromised server could substitute its own, and payments would go to it. */
export async function verifyReceiveAddress(
  wallet: { external: string; network: string },
  info: { index: number; address: string },
): Promise<void> {
  const ours = (await loadWallet()).addressAt(wallet.external, info.index, wallet.network);
  if (ours !== info.address) {
    throw new Error(
      `the server's address for index ${info.index} isn't yours (expected ${ours}); not showing it`,
    );
  }
}
