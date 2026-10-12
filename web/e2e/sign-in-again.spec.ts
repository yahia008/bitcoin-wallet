import { expect, type Page, test } from "@playwright/test";

import { PASSWORD, balance, fundWallet, importWallet, newPhrase } from "./helpers";

test("opening the wallet on a second device keeps the first one signed in", async ({
  page,
  browser,
}) => {
  const words = newPhrase();
  await importWallet(page, words);
  await fundWallet(page, 0.5);

  // The same phrase in a second browser (its own storage): the import gets its own API token.
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await importWallet(otherPage, words);
  await expect(balance(otherPage)).toHaveText("0.5BTC");
  await other.close();

  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(balance(page)).toHaveText("0.5BTC");
  await expect(page.getByRole("button", { name: /Sign in again/ })).toBeHidden();
});

test("a device whose token the server stopped accepting signs back in with its password", async ({
  page,
}) => {
  const words = newPhrase();
  await importWallet(page, words);
  await fundWallet(page, 0.5);

  // As if the token had been revoked: the server doesn't know this one.
  await replaceStoredApiTokens(page, "0".repeat(64));
  await page.reload();
  const signIn = page.getByRole("button", { name: /Sign in again/ });
  await expect(signIn).toBeVisible();

  await signIn.click();
  // A wrong password is refused before anything is asked of the server.
  await page.getByLabel("Wallet password").fill("not the password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByText("That's not your wallet password.")).toBeVisible();

  await page.getByLabel("Wallet password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(balance(page)).toHaveText("0.5BTC");
  await expect(signIn).toBeHidden();
});

/** Overwrites the API token of every regtest account in the page's storage (idb-keyval's
 * default database, see src/lib/store.ts). */
async function replaceStoredApiTokens(page: Page, token: string) {
  await page.evaluate(async (token) => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const open = indexedDB.open("keyval-store");
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(open.error);
    });
    const store = db.transaction("keyval", "readwrite").objectStore("keyval");
    await new Promise<void>((resolve, reject) => {
      const read = store.get("accounts:regtest");
      read.onsuccess = () => {
        const accounts = (read.result as { apiToken: string }[]).map((a) => ({
          ...a,
          apiToken: token,
        }));
        const write = store.put(accounts, "accounts:regtest");
        write.onsuccess = () => resolve();
        write.onerror = () => reject(write.error);
      };
      read.onerror = () => reject(read.error);
    });
    db.close();
  }, token);
}
