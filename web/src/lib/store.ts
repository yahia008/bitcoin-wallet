// The wallet as this browser keeps it, in IndexedDB: per network, a list of accounts (all
// from one recovery phrase) and which one is active. Each account's only secret is
// `encryptedKey` (its account key, encrypted with the user's password by wallet-core); the
// recovery phrase is never stored.

import { del, get, set } from "idb-keyval";

/** One account: m/84'/1'/n' of the seed, registered with the API as its own wallet. */
export type StoredWallet = {
  walletId: string;
  network: string;
  /** n in m/84'/1'/n'; 0 is the first account. */
  account: number;
  /** Shown in the switcher, e.g. "Account 1". */
  name: string;
  /** The seed's master key fingerprint; every account of this wallet has the same one. */
  masterFingerprint: string;
  /** Public descriptors, as registered with the API. */
  external: string;
  internal: string;
  firstAddress: string;
  encryptedKey: { salt: string; nonce: string; ciphertext: string };
  /** The server this account is registered with, and its API token there. */
  apiUrl: string;
  apiToken: string;
  createdAt: string;
};

export type Accounts = { accounts: StoredWallet[]; active?: StoredWallet };

const accountsKey = (network: string) => `accounts:${network}`;
const activeKey = (network: string) => `active:${network}`;
/** Where versions before accounts kept their single wallet. */
const legacyKey = (network: string) => `wallet:${network}`;

export async function loadAccounts(network: string): Promise<Accounts> {
  let accounts = await get<StoredWallet[]>(accountsKey(network));
  if (!accounts) {
    accounts = [];
    // Upgrade a wallet saved before accounts existed: it becomes Account 1.
    const legacy = await get<Omit<StoredWallet, "account" | "name" | "masterFingerprint">>(
      legacyKey(network),
    );
    if (legacy) {
      const fingerprint = /\[([0-9a-f]{8})\//.exec(legacy.external)?.[1] ?? "";
      accounts = [{ ...legacy, account: 0, name: "Account 1", masterFingerprint: fingerprint }];
      await set(accountsKey(network), accounts);
      await set(activeKey(network), legacy.walletId);
      await del(legacyKey(network));
    }
  }
  const activeId = await get<string>(activeKey(network));
  return { accounts, active: accounts.find((a) => a.walletId === activeId) ?? accounts[0] };
}

/** Adds (or updates) an account and makes it the active one. */
export async function saveAccount(wallet: StoredWallet): Promise<Accounts> {
  const { accounts } = await loadAccounts(wallet.network);
  const others = accounts.filter((a) => a.walletId !== wallet.walletId);
  const updated = [...others, wallet].sort((a, b) => a.account - b.account);
  await set(accountsKey(wallet.network), updated);
  await set(activeKey(wallet.network), wallet.walletId);
  return { accounts: updated, active: wallet };
}

export async function setActiveAccount(network: string, walletId: string): Promise<void> {
  await set(activeKey(network), walletId);
}

/** Removes every account of this network's wallet from the browser. */
export async function forgetAccounts(network: string): Promise<void> {
  await del(accountsKey(network));
  await del(activeKey(network));
}
