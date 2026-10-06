// Loads the Rust wallet code (wallet-core, via wallet-wasm) compiled to WebAssembly. It's the
// same key handling, PSBT review and signing the CLI uses, so the browser doesn't need a
// second implementation of the security checks. `npm run wasm` generates src/wasm/.

import { recoverToken, requestChallenge } from "@/lib/api";
import { ReviewError } from "@/lib/errors";
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
  account: number;
  masterFingerprint: string;
  external: string;
  internal: string;
  firstAddress: string;
  encryptedKey: { salt: string; nonce: string; ciphertext: string };
};

/** Derives account `account`'s key (m/84'/1'/n', 0 = first) from `words`, encrypts it with
 * `password` (Argon2id, ~0.2 s), and returns that plus the public parts. Throws on a bad
 * phrase or short password. */
export async function createWallet(
  words: string,
  password: string,
  network: string,
  account = 0,
): Promise<NewWallet> {
  return (await loadWallet()).createWallet(words, password, network, account);
}

/** Throws unless `password` unlocks `wallet`'s key. */
export async function checkPassword(wallet: WalletKeys, password: string): Promise<void> {
  (await loadWallet()).checkPassword(wallet, password);
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

/** What `reviewSend` found in the PSBT. */
export type SendReview = {
  feeSat: number;
  changeSat: number;
  /** Our change addresses the change goes to, derived here from the internal descriptor at
   * the indexes the review matched; empty when there's no change. */
  changeAddresses: ChangeAddress[];
  inputs: number;
  /** sat/vB once signed (a lower bound). */
  feeRate: number;
  /** Set when the fee looks like a mistake; the user must confirm it explicitly. */
  feeWarning?: string;
};

/** One of our change (internal keychain) addresses. */
export type ChangeAddress = { index: number; address: string };

/** Derives our change address at each of `indexes` from the wallet's internal descriptor. */
function changeAddresses(
  w: typeof wasm,
  wallet: WalletKeys,
  indexes: number[],
): ChangeAddress[] {
  return indexes.map((index) => ({
    index,
    address: w.addressAt(wallet.internal, index, wallet.network),
  }));
}

type WalletKeys = { network: string; external: string; internal: string; encryptedKey: unknown };

/** Runs the CLI's `review` on a server-built PSBT: it must pay exactly `amountSat` to `to`,
 * with everything else going to our own change. Throws if not. */
export async function reviewSend(
  wallet: WalletKeys,
  psbt: string,
  to: string,
  amountSat: number,
): Promise<SendReview> {
  const w = await loadWallet();
  const { changeIndexes, ...review } = refused(() =>
    w.reviewSend(wallet, psbt, to, BigInt(amountSat)),
  );
  return { ...review, changeAddresses: changeAddresses(w, wallet, changeIndexes) };
}

/** Runs a review, turning its failure into a ReviewError: the server's transaction was refused. */
function refused<T>(review: () => T): T {
  try {
    return review();
  } catch (e) {
    throw new ReviewError(e instanceof Error ? e.message : String(e));
  }
}

/** Decrypts the account key with `password` inside WASM and signs `psbt`. The key never
 * reaches JavaScript. Throws on a wrong password. */
export async function signPsbt(
  wallet: WalletKeys,
  password: string,
  psbt: string,
): Promise<string> {
  return (await loadWallet()).signPsbt(wallet, password, psbt);
}

/** What `reviewBump` found. */
export type BumpReview = {
  paidSat: number;
  oldFeeSat: number;
  oldFeeRate: number;
  feeSat: number;
  feeRate: number;
  changeSat: number;
  /** As in `SendReview`. */
  changeAddresses: ChangeAddress[];
  inputs: number;
  feeWarning?: string;
};

/** Runs the CLI's `review_bump` on a server-built replacement of `txid`: the original (hex,
 * from the server) must really be `txid`, and the replacement must keep every payment exactly
 * and pay more fee. Throws if not. */
export async function reviewBump(
  wallet: WalletKeys,
  psbt: string,
  originalTx: string,
  txid: string,
): Promise<BumpReview> {
  const w = await loadWallet();
  const { changeIndexes, ...review } = refused(() => w.reviewBump(wallet, psbt, originalTx, txid));
  return { ...review, changeAddresses: changeAddresses(w, wallet, changeIndexes) };
}

/** Gets a new API token for an already-registered wallet by proving we hold its key: the
 * server sends a one-time challenge, we sign it in WASM (key decrypted with `password`). */
export async function recoverApiToken(
  wallet: WalletKeys & { walletId: string },
  password: string,
): Promise<string> {
  const { challenge } = await requestChallenge(wallet.walletId);
  const signature = (await loadWallet()).proveOwnership(wallet, password, challenge);
  return (await recoverToken(wallet.walletId, challenge, signature)).token;
}

/** Throws a readable reason unless `address` is a valid address for `network` (catches typos
 * via the address checksum, and addresses for another network). */
export async function checkAddress(address: string, network: string): Promise<void> {
  (await loadWallet()).checkAddress(address, network);
}
