// Turning a recovery phrase into a registered account, shared by first-time setup and
// "Add account".

import { API_URL, ApiError, register } from "@/lib/api";
import type { StoredWallet } from "@/lib/store";
import { createWallet, recoverApiToken } from "@/lib/wallet";

/** Derives account `account` from `words`, encrypts its key with `password`, and registers
 * its public descriptors with the API. Returns the account to store; doesn't store it.
 * With `sameWalletAs` (a master key fingerprint), refuses a phrase from another wallet
 * before anything is sent to the server. */
export async function setUpAccount(
  words: string,
  password: string,
  network: string,
  account: number,
  sameWalletAs?: string,
): Promise<StoredWallet> {
  // Derives the account key and encrypts it, all in this browser.
  const created = await createWallet(words, password, network, account);
  if (sameWalletAs !== undefined && created.masterFingerprint !== sameWalletAs) {
    throw new Error(
      "This recovery phrase belongs to a different wallet. Use the phrase of this wallet.",
    );
  }
  // Only the public descriptors go to the server.
  let apiToken: string;
  try {
    const registration = await register(created.external, created.internal);
    if (registration.id !== created.walletId) {
      throw new Error("the server computed a different wallet id; not saving");
    }
    apiToken = registration.token;
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 409)) throw e;
    // Already registered (e.g. restored after "Forget"): the server won't hand out the old
    // token, so prove we hold the key and get a new one.
    apiToken = await recoverApiToken(created, password);
  }
  return {
    walletId: created.walletId,
    network: created.network,
    account: created.account,
    name: `Account ${created.account + 1}`,
    masterFingerprint: created.masterFingerprint,
    external: created.external,
    internal: created.internal,
    firstAddress: created.firstAddress,
    encryptedKey: created.encryptedKey,
    apiUrl: API_URL,
    apiToken,
    createdAt: new Date().toISOString(),
  };
}
