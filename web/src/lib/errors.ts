// Turns errors from the API, the network and WASM into sentences for people. The server's
// messages are written for developers (and the CLI shows them as they are), so they're
// reworded here, at the one place every screen gets its error text from.

import { ApiError } from "@/lib/api";
import { formatBtcShort } from "@/lib/format";

/** The wallet's own check refused a server-built transaction (wallet-core's review). Nothing
 * was signed. */
export class ReviewError extends Error {}

/** A sentence to show the user for `e`. */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return apiMessage(e);
  if (e instanceof ReviewError) {
    return (
      "Osok refused this transaction because it doesn't match what you asked for, so nothing " +
      `was signed. If this keeps happening, don't trust this server. Details: ${e.message}`
    );
  }
  // fetch() rejects with a TypeError when the server can't be reached at all.
  if (e instanceof TypeError && /fetch|network|load failed/i.test(e.message)) {
    return "Can't reach the wallet server. Check your connection and that the server is running, then try again.";
  }
  const text = e instanceof Error ? e.message : String(e);
  if (/wrong password/i.test(text)) return "That's not your wallet password.";
  return sentence(text);
}

function apiMessage(e: ApiError): string {
  const text = e.message;
  switch (e.status) {
    case 401:
      if (/invalid API token/i.test(text)) {
        return "This device was signed out because the wallet was opened somewhere else.";
      }
      if (/challenge/i.test(text)) return "That took too long. Please try again.";
      if (/signature/i.test(text)) return "The server didn't accept this wallet's signature.";
      return "The server didn't accept this device's sign-in.";
    case 404:
      if (/wallet not found/i.test(text)) {
        return (
          "The server doesn't know this wallet anymore (its data may have been reset). Your " +
          "coins are safe: forget the wallet in Settings and import your recovery phrase again."
        );
      }
      if (/transaction not found/i.test(text)) {
        return "This transaction isn't in your wallet anymore. It may have been sped up (replaced) or dropped.";
      }
      return "The server didn't find what was asked for.";
    case 409:
      return "This wallet is already registered with the server.";
    case 429: {
      const seconds = /(\d+)\s*s\b/.exec(text)?.[1];
      return `Too many requests. Wait ${seconds ? `${seconds} seconds` : "a moment"} and try again.`;
    }
    case 502:
    case 503:
    case 504:
      if (/chain backend is unreachable/i.test(text)) {
        return "The server can't reach the Bitcoin network right now. Try again in a minute.";
      }
      if (/chain backend/i.test(text)) {
        return "The Bitcoin network service the server uses returned an error. Try again in a minute.";
      }
      // No answer from the API itself, e.g. the proxy in front of it found it down.
      return "The wallet server isn't responding right now. Try again in a minute.";
    case 400:
      return badRequest(text);
    default:
      if (e.status >= 500) {
        return "Something went wrong on the server. Try again, and if it keeps happening, check the server log.";
      }
      return sentence(text);
  }
}

function badRequest(text: string): string {
  const funds = /insufficient funds.*?([\d.]+) BTC available of ([\d.]+) BTC needed/i.exec(text);
  if (funds) {
    const [available, needed] = [funds[1], funds[2]].map((btc) =>
      formatBtcShort(Math.round(Number(btc) * 1e8)),
    );
    let message = `Not enough bitcoin. This needs ${needed} BTC including the fee, but only ${available} BTC is available.`;
    if (/reserved/i.test(text)) {
      message +=
        " Some coins are held by a transaction you started but didn't send. They're freed when it's sent, or after 10 minutes.";
    }
    return message;
  }
  if (/insufficient funds/i.test(text)) return "Not enough bitcoin to cover the amount and the fee.";
  if (/dust/i.test(text)) return "That amount is too small to send. Send a bit more.";
  if (/transaction not found/i.test(text)) {
    return "This transaction isn't in your wallet anymore. It may have been sped up (replaced) or dropped.";
  }
  if (/can't bump/i.test(text)) {
    return "This transaction can't be sped up. It may already be confirmed or replaced.";
  }
  if (/fee rate too low for a replacement/i.test(text)) {
    return "That fee is too low to replace the original. Choose a higher fee rate.";
  }
  const rejected = /the transaction was rejected: (.*)/i.exec(text);
  if (rejected) return `The Bitcoin network rejected the transaction: ${rejected[1]}.`;
  if (/different network/i.test(text)) return "That address is for a different network.";
  if (/invalid address/i.test(text)) return "That's not a valid Bitcoin address.";
  return sentence(text);
}

/** "fee too low" -> "Fee too low." */
function sentence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return "Something went wrong. Please try again.";
  const capital = trimmed[0].toUpperCase() + trimmed.slice(1);
  return /[.!?)]$/.test(capital) ? capital : `${capital}.`;
}
