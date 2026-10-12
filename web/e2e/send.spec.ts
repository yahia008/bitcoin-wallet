import { expect, test } from "@playwright/test";
import QRCode from "qrcode";

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
  // A fresh wallet's first change goes to its change address #0, derived in the browser.
  await expect(page.getByText("Change address #0")).toBeVisible();
  await expect(page.locator("dd", { hasText: /^bcrt1/ }).last()).toBeVisible();

  await page.getByRole("button", { name: /^Send to / }).click();
  await page.getByLabel("Wallet password").fill("not my password");
  await page.getByRole("button", { name: "Sign and send" }).click();
  await expect(page.getByText("That's not your wallet password.")).toBeVisible();

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

test("a pasted bitcoin: link fills in the address and amount", async ({ page }) => {
  const to = await newFunderAddress();
  await page.getByRole("button", { name: "Send" }).first().click();
  await page.getByLabel("Recipient address").fill(`bitcoin:${to}?amount=0.25&label=Pizza`);
  // Straight on to the amount step, with the link's amount.
  await expect(page.getByLabel("Amount in BTC")).toHaveValue("0.25");
  await page.getByRole("button", { name: "Review Send" }).click();
  await expect(page.getByText("You are sending")).toBeVisible();
  await expect(page.getByText("0.25 BTC", { exact: true })).toBeVisible();
});

test("a payment link this wallet can't honour is refused", async ({ page }) => {
  await page.getByRole("button", { name: "Send" }).first().click();
  const to = await newFunderAddress();
  await page.getByLabel("Recipient address").fill(`bitcoin:${to}?req-somethingnew=1`);
  await expect(page.getByText("asks for something this wallet doesn't support")).toBeVisible();
  await expect(page.getByLabel("Amount in BTC")).toHaveCount(0);
});

test("scan: a picture of a payment QR code (upper-case, as wallets print them)", async ({ page }) => {
  const to = await newFunderAddress();
  const png = await QRCode.toBuffer(`BITCOIN:${to.toUpperCase()}?amount=0.1`, { width: 400 });
  await page.getByRole("button", { name: "Send" }).first().click();
  await page.getByRole("button", { name: "Scan QR code" }).click();
  await expect(page.getByRole("heading", { name: "Scan QR code" })).toBeVisible();
  await page.getByLabel("QR code picture").setInputFiles({
    name: "invoice.png",
    mimeType: "image/png",
    buffer: png,
  });
  await expect(page.getByLabel("Amount in BTC")).toHaveValue("0.1");
  await page.getByRole("button", { name: /^bcrt1/ }).click(); // back to the address
  await expect(page.getByLabel("Recipient address")).toHaveValue(to);
});
