import { expect, test } from "@playwright/test";

import { newFunderAddress } from "./bitcoind";
import { balance, confirmSend, fundWallet, importWallet, newPhrase, startSend } from "./helpers";

test.beforeEach(async ({ page }) => {
  await importWallet(page, newPhrase());
  await fundWallet(page, 2);
});

test("the address step catches other networks and typos", async ({ page }) => {
  await page.getByRole("button", { name: "Send" }).first().click();
  const input = page.getByLabel("Recipient address");
  await input.fill("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu");
  await expect(
    page.getByText("That's a mainnet address, but this wallet is on regtest"),
  ).toBeVisible();
  const good = await newFunderAddress();
  await input.fill(good.slice(0, -1) + (good.endsWith("q") ? "p" : "q")); // one wrong character
  await expect(page.getByText("That's not a valid bitcoin address")).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);
  await input.fill(good);
  await expect(page.getByRole("button", { name: "Continue" })).toBeVisible();
});

test("send half: review sheet, wrong password refused, then sent", async ({ page }) => {
  await page.getByRole("button", { name: "Send" }).first().click();
  await page.getByLabel("Recipient address").fill(await newFunderAddress());
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "50%" }).click();
  await expect(page.getByLabel("Amount in BTC")).toHaveValue("1");
  await page.getByRole("button", { name: "Review Send" }).click();

  await expect(page.getByText("You are sending")).toBeVisible();
  await page.getByRole("button", { name: "Transaction details" }).click();
  await expect(page.getByText("Verified in this browser")).toBeVisible();

  await page.getByRole("button", { name: /^Send to / }).click();
  await page.getByLabel("Wallet password").fill("not my password");
  await page.getByRole("button", { name: "Sign and send" }).click();
  await expect(page.getByText(/wrong password/)).toBeVisible();

  await page.getByLabel("Wallet password").fill("correct horse battery");
  await page.getByRole("button", { name: "Sign and send" }).click();
  await expect(page.getByText("1 BTC sent")).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText("Sent", { exact: true })).toBeVisible();
});

test("Max sends everything: no change, balance ends at 0", async ({ page }) => {
  await startSend(page, await newFunderAddress(), "max");
  const change = page.locator("dt", { hasText: "Change back to you" }).locator("xpath=..");
  await expect(change).toContainText("0 BTC");
  await confirmSend(page);
  await page.getByRole("button", { name: "Done" }).click();
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(balance(page)).toHaveText(/^0\s*BTC$/);
});
