// The wallet as this browser keeps it, in IndexedDB. One wallet per network. The only secret
// is `encryptedKey` (the account key, encrypted with the user's password by wallet-core);
// the recovery phrase is never stored.

import { del, get, set } from "idb-keyval";

export type StoredWallet = {
  walletId: string;
  network: string;
  /** Public descriptors, as registered with the API. */
  external: string;
  internal: string;
  firstAddress: string;
  encryptedKey: { salt: string; nonce: string; ciphertext: string };
  /** The server this wallet is registered with, and its API token there. */
  apiUrl: string;
  apiToken: string;
  createdAt: string;
};

const key = (network: string) => `wallet:${network}`;

export function loadStoredWallet(network: string): Promise<StoredWallet | undefined> {
  return get(key(network));
}

export function saveStoredWallet(wallet: StoredWallet): Promise<void> {
  return set(key(wallet.network), wallet);
}

export function forgetStoredWallet(network: string): Promise<void> {
  return del(key(network));
}
