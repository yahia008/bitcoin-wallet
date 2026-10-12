import { expect, test } from "@playwright/test";

import { importWallet, newPhrase } from "./helpers";

test("the network menu switches to another network's server and back", async ({ page }) => {
  await importWallet(page, newPhrase());
  const badge = (network: string) => page.getByRole("button", { name: `Network: ${network}` });

  await badge("regtest").click();
  await page.getByRole("menuitemradio", { name: /testnet4/i }).click();
  // Its server is down in the tests: the page says so, and still offers the menu.
  await expect(page.getByRole("heading", { name: "Can't reach the wallet server" })).toBeVisible();
  await expect(page.getByText("the testnet4 server")).toBeVisible();

  // The choice is remembered.
  await page.reload();
  await expect(badge("testnet4")).toBeVisible();

  await badge("testnet4").click();
  await page.getByRole("menuitemradio", { name: /regtest/i }).click();
  // Regtest's wallet is still there.
  await expect(page.getByRole("button", { name: /Account 1/ })).toBeVisible();
  await expect(badge("regtest")).toBeVisible();
});
