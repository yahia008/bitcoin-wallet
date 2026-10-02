import { expect, test } from "@playwright/test";

import { fund } from "./bitcoind";
import { importWallet, newPhrase, receiveAddress } from "./helpers";

test("receive: the address moves on once used, and Addresses shows used / unused", async ({
  page,
}) => {
  await importWallet(page, newPhrase());
  const first = await receiveAddress(page);
  expect(first).toMatch(/^bcrt1q/);
  // Not used yet: the same address is shown again.
  expect(await receiveAddress(page)).toBe(first);

  await fund(first, 0.1);
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(page.getByText("+0.1", { exact: true })).toBeVisible();
  // Used now: Receive hands out the next one.
  expect(await receiveAddress(page)).not.toBe(first);

  await page.getByRole("tab", { name: "Addresses" }).click();
  await expect(page.getByText("Used", { exact: true })).toBeVisible();
  await expect(page.getByText("Unused", { exact: true })).toBeVisible();
});
