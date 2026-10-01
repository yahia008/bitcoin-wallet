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
