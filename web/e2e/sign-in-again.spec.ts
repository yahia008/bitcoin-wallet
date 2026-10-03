import { expect, test } from "@playwright/test";

import { PASSWORD, balance, fundWallet, importWallet, newPhrase } from "./helpers";

test("a copy signed out by another copy of the wallet signs back in with its password", async ({
  page,
  browser,
}) => {
  const words = newPhrase();
  await importWallet(page, words);
  await fundWallet(page, 0.5);

  // The same phrase in a second browser (its own storage): the import recovers a new API
  // token, which signs the first copy out.
  const other = await browser.newContext();
  await importWallet(await other.newPage(), words);
  await other.close();

  await page.getByRole("button", { name: "Refresh" }).click();
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
