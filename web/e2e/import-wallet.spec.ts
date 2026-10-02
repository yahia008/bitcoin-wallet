import { expect, test } from "@playwright/test";

import { choosePassword, newPhrase, pastePhrase } from "./helpers";

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await page.getByText("I already have a wallet").click();
  await choosePassword(page);
  await expect(page.getByText("Import wallet from recovery phrase")).toBeVisible();
});

test("import a 12-word phrase by pasting it", async ({ page }) => {
  await pastePhrase(page, newPhrase());
  // Hidden until Show is pressed.
  await expect(page.getByLabel("Word 1", { exact: true })).toHaveAttribute("type", "password");
  await page.getByRole("button", { name: /Show/ }).click();
  await expect(page.getByLabel("Word 1", { exact: true })).toHaveAttribute("type", "text");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByText("Looking a little empty")).toBeVisible();
});

test("pasting 24 words switches to 24 boxes and imports", async ({ page }) => {
  // The BIP39 test vector "abandon ×23 art".
  await pastePhrase(page, "abandon ".repeat(23) + "art");
  await expect(page.getByRole("switch")).toHaveAttribute("aria-checked", "true");
  await expect(page.locator("input[aria-label^='Word ']")).toHaveCount(24);
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByRole("button", { name: /Account 1/ })).toBeVisible();
});

test("a misspelled word is named, counting from 1", async ({ page }) => {
  const words = newPhrase().split(" ");
  words[4] = "notaword";
  await pastePhrase(page, words.join(" "));
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(
    page.getByText('word 5 ("notaword") isn\'t in the recovery phrase word list'),
  ).toBeVisible();
});
