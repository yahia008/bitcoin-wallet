// BIP21 payment links, as merchants and other wallets put in QR codes:
//   bitcoin:bc1q…?amount=0.001&label=Pizza
// https://github.com/bitcoin/bips/blob/master/bip-0021.mediawiki

import { parseBtc } from "./format";

export type PaymentRequest = {
  address: string;
  /** In sats, if the link asks for an amount. */
  amountSat?: number;
  label?: string;
  message?: string;
};

/** What a QR code or a pasted text asks to pay: a `bitcoin:` link, or a bare address. Throws
 * with a message for the user if it's a link this wallet can't honour. The address itself
 * isn't checked here; that's `checkAddress`, against the wallet's network. */
export function parsePaymentRequest(text: string): PaymentRequest {
  const trimmed = text.trim();
  if (!/^bitcoin:/i.test(trimmed)) return { address: normalizeAddress(trimmed) };

  const rest = trimmed.slice("bitcoin:".length);
  const q = rest.indexOf("?");
  const address = normalizeAddress(q < 0 ? rest : rest.slice(0, q));
  if (!address) throw new Error("That payment link has no bitcoin address");
  const request: PaymentRequest = { address };

  for (const [key, value] of new URLSearchParams(q < 0 ? "" : rest.slice(q + 1))) {
    switch (key.toLowerCase()) {
      case "amount": {
        const sats = parseBtc(value);
        if (sats === undefined) throw new Error("That payment link has an amount that isn't valid");
        request.amountSat = sats;
        break;
      }
      case "label":
        request.label = value;
        break;
      case "message":
        request.message = value;
        break;
      default:
        // BIP21: a required parameter we don't understand means we can't pay it correctly.
        if (key.toLowerCase().startsWith("req-")) {
          throw new Error("That payment link asks for something this wallet doesn't support");
        }
    }
  }
  return request;
}

/** QR codes often carry bech32 addresses in upper case (it makes the code smaller). Bech32 is
 * case-insensitive, so lower-case those; base58 addresses are case-sensitive, leave them be. */
function normalizeAddress(address: string): string {
  const lower = address.toLowerCase();
  return /^(bc|tb|bcrt)1/.test(lower) ? lower : address;
}
