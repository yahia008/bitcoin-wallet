import { expect, test } from "@playwright/test";

import { PASSWORD } from "./helpers";

test("create a wallet: password, phrase, confirm the phrase by tapping it back", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Osok Wallet" })).toBeVisible();
  await page.getByText("Create new wallet").click();

  // Password rules show up as the user leaves the fields.
  await page.getByPlaceholder("New password").fill("short");
  await page.getByPlaceholder("Confirm password").focus();
  await expect(page.getByText("Password must be at least 8 characters")).toBeVisible();
  await page.getByPlaceholder("New password").fill(PASSWORD);
  await page.getByPlaceholder("Confirm password").fill(PASSWORD);
  const confirm = page.getByRole("button", { name: "Confirm" });
  await expect(confirm).toBeDisabled(); // Terms not accepted yet
  await page.getByRole("checkbox").check();
  await confirm.click();

  await page.getByRole("button", { name: "Show recovery phrase" }).click();
  const words = (await page.locator("ol li").allInnerTexts()).map((t) =>
    t.replace(/^\d+\.\s*/, "").trim(),
  );
  expect(words).toHaveLength(12);
  await page.getByRole("button", { name: /saved my phrase/ }).click();

  // Wrong order is refused.
  const chips = page.locator("div.grid > button");
  for (let i = 0; i < 12; i++) await chips.nth(i).click();
  await page.getByRole("button", { name: "Confirm", exact: true }).click();
  await expect(page.getByText("not the right order")).toBeVisible();

  // Unselect all, then tap them in the real order.
  for (let i = 0; i < 12; i++) await chips.nth(i).click();
  const used = new Set<number>();
  for (const word of words) {
    for (let i = 0; i < 12; i++) {
      const text = (await chips.nth(i).innerText()).replace(/^\d+\s*/, "").trim();
      if (!used.has(i) && text === word) {
        await chips.nth(i).click();
        used.add(i);
        break;
      }
    }
  }
  await page.getByRole("button", { name: "Confirm", exact: true }).click();

  await expect(page.getByText("Looking a little empty")).toBeVisible();
  await expect(page.getByRole("button", { name: /Account 1/ })).toBeVisible();
});
