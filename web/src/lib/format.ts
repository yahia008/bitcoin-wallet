// Display helpers. Amounts stay integer satoshis everywhere else; only text is BTC.

const SATS_PER_BTC = 100_000_000;

/** 123456 → "0.00123456". Exact: splits the integer, no floating point. */
export function formatBtc(sats: number): string {
  const sign = sats < 0 ? "-" : "";
  const abs = Math.abs(sats);
  const whole = Math.floor(abs / SATS_PER_BTC);
  const frac = String(abs % SATS_PER_BTC).padStart(8, "0");
  return `${sign}${whole}.${frac}`;
}

/** "0.001" → 100000. Exact (string arithmetic, no floating point); undefined if it isn't a
 * positive amount with at most 8 decimals. */
export function parseBtc(text: string): number | undefined {
  const match = /^(\d+)(?:\.(\d{1,8}))?$/.exec(text.trim());
  if (!match) return undefined;
  const sats = Number(match[1]) * SATS_PER_BTC + Number((match[2] ?? "").padEnd(8, "0"));
  return Number.isSafeInteger(sats) && sats > 0 ? sats : undefined;
}

const EXPLORERS: Record<string, string> = {
  testnet4: "https://mempool.space/testnet4",
  testnet: "https://blockstream.info/testnet",
  signet: "https://mempool.space/signet",
};

/** A block explorer page for the transaction, or undefined (e.g. on regtest). */
export function explorerTxUrl(network: string, txid: string): string | undefined {
  const base = EXPLORERS[network];
  return base && `${base}/tx/${txid}`;
}
