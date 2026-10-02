import { expect, test } from "@playwright/test";

import { inMempool, mine, newFunderAddress, rpc } from "./bitcoind";
import { confirmSend, fundWallet, importWallet, newPhrase, PASSWORD, startSend } from "./helpers";

test("speed up an unconfirmed send: the node swaps in the replacement", async ({ page }) => {
  await importWallet(page, newPhrase());
  await fundWallet(page, 1);
  await startSend(page, await newFunderAddress(), "0.3");
  await confirmSend(page);
  await page.getByRole("button", { name: "Done" }).click();
  const before = await rpc<string[]>("getrawmempool");

  await page.getByRole("button", { name: "Speed up" }).click();
  await page.getByRole("button", { name: "Review" }).click();
  await expect(page.getByText("same payments as the original")).toBeVisible();
  await page.getByPlaceholder("wallet password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign and replace" }).click();
  await expect(page.getByRole("button", { name: "Speed up" })).toBeVisible();

  // The original left the mempool; exactly one new transaction took its place.
  const after = await rpc<string[]>("getrawmempool");
  const replaced = before.filter((t) => !after.includes(t));
  const replacement = after.filter((t) => !before.includes(t));
  expect(replaced).toHaveLength(1);
  expect(replacement).toHaveLength(1);
  expect(await inMempool(replaced[0])).toBe(false);
  await mine(1);
});
