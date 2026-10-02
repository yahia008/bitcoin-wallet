// Steps the tests share, written the way a person uses the wallet: through the page.

import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { expect, type Page } from "@playwright/test";

import { fund } from "./bitcoind";

export const PASSWORD = "correct horse battery";

/** A fresh random recovery phrase, from the same WASM module the site uses. Run in a separate
 * Node process: the wasm-pack wrapper is an ES module (it uses import.meta), which the test
 * runner's TypeScript loader can't import. */
export function newPhrase(): string {
  const dir = join(__dirname, "..", "src", "wasm");
  const script = `
    import { readFileSync } from "node:fs";
    const w = await import(${JSON.stringify(pathToFileURL(join(dir, "wallet_wasm.js")).href)});
    w.initSync({ module: readFileSync(${JSON.stringify(join(dir, "wallet_wasm_bg.wasm"))}) });
    process.stdout.write(w.generateMnemonic());
  `;
  return execFileSync(process.execPath, ["--no-warnings", "--input-type=module", "-e", script], {
    encoding: "utf8",
  });
}

/** Types `text` into a box as a paste, like a user pasting a whole phrase. */
export async function pastePhrase(page: Page, text: string) {
  await page.getByLabel("Word 1", { exact: true }).evaluate((el, text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true }));
  }, text);
}

/** Fills the "Create a Password" step. */
export async function choosePassword(page: Page, password = PASSWORD) {
  await page.getByPlaceholder("New password").fill(password);
  await page.getByPlaceholder("Confirm password").fill(password);
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Confirm" }).click();
}

/** "I already have a wallet" with `words`, ending on the dashboard. */
export async function importWallet(page: Page, words: string, password = PASSWORD) {
  await page.goto("/");
  await page.getByText("I already have a wallet").click();
  await choosePassword(page, password);
  await pastePhrase(page, words);
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByRole("button", { name: /Account 1/ })).toBeVisible();
}

/** The current receive address, read from the Receive screen. */
export async function receiveAddress(page: Page): Promise<string> {
  await page.getByRole("button", { name: "Receive" }).first().click();
  await expect(page.getByText("checked against your own keys")).toBeVisible();
  const address = (await page.locator("p.font-mono").first().innerText()).trim();
  await page.getByRole("button", { name: "Back" }).click();
  return address;
}

/** The dashboard's big balance, e.g. "1.5BTC". */
export const balance = (page: Page) => page.locator("p.text-4xl");

/** Pays `btc` into the wallet on the page from the node, mines it, and refreshes. */
export async function fundWallet(page: Page, btc: number) {
  await fund(await receiveAddress(page), btc);
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(page.getByText(`+${btc}`, { exact: true })).toBeVisible();
}

/** Send screen, up to (not including) the review: address, then amount or Max. */
export async function startSend(page: Page, to: string, amount: string | "max") {
  await page.getByRole("button", { name: "Send" }).first().click();
  await page.getByLabel("Recipient address").fill(to);
  await page.getByRole("button", { name: "Continue" }).click();
  if (amount === "max") {
    await page.getByRole("button", { name: "Max" }).click();
  } else {
    await page.getByLabel("Amount in BTC").fill(amount);
  }
  await page.getByRole("button", { name: "Review Send" }).click();
  await expect(page.getByText("You are sending")).toBeVisible();
}

/** From the review sheet: Send to…, password, sign. Returns the "… BTC sent" text. */
export async function confirmSend(page: Page, password = PASSWORD): Promise<string> {
  await page.getByRole("button", { name: /^Send to / }).click();
  await page.getByLabel("Wallet password").fill(password);
  await page.getByRole("button", { name: "Sign and send" }).click();
  const sent = page.getByText("BTC sent");
  await expect(sent).toBeVisible();
  return sent.innerText();
}
