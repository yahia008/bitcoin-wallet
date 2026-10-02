import { expect, test } from "@playwright/test";

import { fundWallet, importWallet, newPhrase, PASSWORD, pastePhrase } from "./helpers";

test("add, rename and switch accounts; refuse a wrong password or another phrase", async ({
  page,
}) => {
  const words = newPhrase();
  await importWallet(page, words);
  await fundWallet(page, 0.75);

  await page.getByRole("button", { name: /Account 1/ }).click();
  await expect(page.getByRole("heading", { name: "Wallets" })).toBeVisible();
  await page.getByRole("button", { name: "Rename" }).click();
  await page.getByLabel("Account name").fill("Savings");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Savings").first()).toBeVisible();

  await page.getByText("Add wallet").click();
  await page.getByText("Create a new wallet").click();
  await page.getByLabel("Wallet password").fill("wrong password");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("That's not your wallet password.")).toBeVisible();
  await page.getByLabel("Wallet password").fill(PASSWORD);
  await page.getByRole("button", { name: "Continue" }).click();

  await pastePhrase(page, newPhrase());
  await page.getByRole("button", { name: "Add Account 2" }).click();
  await expect(page.getByText("belongs to a different wallet")).toBeVisible();
  await pastePhrase(page, words);
  await page.getByRole("button", { name: "Add Account 2" }).click();
  await expect(page.getByRole("button", { name: /Account 2/ })).toBeVisible();
  await expect(page.getByText("Looking a little empty")).toBeVisible();

  // Both accounts with their balances; switching back shows Savings' coins.
  await page.getByRole("button", { name: /Account 2/ }).click();
  await expect(page.getByRole("button", { name: /Savings/ })).toContainText("0.75 BTC");
  await page.getByRole("button", { name: /Savings/ }).click();
  await expect(page.getByText("+0.75", { exact: true })).toBeVisible();
});
