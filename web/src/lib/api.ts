// Talks to the watch-only wallet API (src/bin/server.rs). Everything sent here is public:
// descriptors, addresses, unsigned and signed PSBTs. Never mnemonics or private keys.

/** Where the API server runs. Baked in at build time (NEXT_PUBLIC_ variables are). */
export const API_URL = process.env.NEXT_PUBLIC_API_URL ?? "http://127.0.0.1:3000";

/** An error answer from the API: its HTTP status plus the `{"error": ...}` message. */
export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit, base = API_URL): Promise<T> {
  const res = await fetch(`${base}${path}`, init);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new ApiError(res.status, body?.error ?? `API answered ${res.status}`);
  }
  return res.json();
}

export type Health = {
  status: string;
  /** e.g. "testnet4"; the wallet must use the same network as the server. */
  network: string;
};

export function health(): Promise<Health> {
  return request("/health");
}

export type Registration = {
  id: string;
  /** Bearer token for this wallet's endpoints. The server shows it only this once. */
  token: string;
};

/** Registers a wallet's public descriptors, making the server watch it. */
export function register(external: string, internal: string): Promise<Registration> {
  return request("/wallets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    // Birthday 0: scan from genesis. Esplora backends ignore it anyway.
    body: JSON.stringify({ external, internal, birthday: 0 }),
  });
}

// Per-wallet endpoints. They need the wallet's API token, and use the server it registered
// with. Each call makes the server sync the wallet first, so answers are current.

export type Balance = {
  confirmed_sat: number;
  unconfirmed_sat: number;
  /** Coinbase outputs not yet spendable (regtest mining). */
  immature_sat: number;
  total_sat: number;
};

export type AddressInfo = { index: number; address: string; used: boolean };

export type Transaction = {
  txid: string;
  /** Effect on the balance: positive = received, negative = sent (fee included). */
  net_sat: number;
  /** null unless this wallet paid the fee. */
  fee_sat: number | null;
  confirmed: boolean;
  confirmations: number;
  block_height: number | null;
};

export type WalletAuth = { walletId: string; apiUrl: string; apiToken: string };

function walletRequest<T>(
  wallet: WalletAuth,
  path: string,
  method = "GET",
  body?: unknown,
): Promise<T> {
  const headers: Record<string, string> = { authorization: `Bearer ${wallet.apiToken}` };
  if (body !== undefined) headers["content-type"] = "application/json";
  return request(
    `/wallets/${wallet.walletId}${path}`,
    { method, headers, body: body === undefined ? undefined : JSON.stringify(body) },
    wallet.apiUrl,
  );
}

export const getBalance = (w: WalletAuth) => walletRequest<Balance>(w, "/balance");
/** Receive addresses handed out so far, and whether each has been paid to. */
export const getAddresses = (w: WalletAuth) => walletRequest<AddressInfo[]>(w, "/addresses");
/** Reveals the next receive address. */
export const newAddress = (w: WalletAuth) => walletRequest<AddressInfo>(w, "/addresses", "POST");
/** Newest first. */
export const getTransactions = (w: WalletAuth) => walletRequest<Transaction[]>(w, "/transactions");

export type FeePriority = "fast" | "normal" | "slow";

export type PsbtRequest = {
  address: string;
  amount_sat: number;
  fee_priority: FeePriority;
};

/** The server's unsigned PSBT and its own summary. Don't trust the summary: check the PSBT
 * with `reviewSend` (lib/wallet.ts) before signing. */
export type PsbtResponse = {
  psbt: string;
  amount_sat: number;
  fee_sat: number;
  change_sat: number;
  fee_rate_sat_vb: number;
};

export const buildPsbt = (w: WalletAuth, req: PsbtRequest) =>
  walletRequest<PsbtResponse>(w, "/psbt", "POST", req);

/** Sends a signed PSBT; the server finalizes and broadcasts it. */
export const broadcast = (w: WalletAuth, psbt: string) =>
  walletRequest<{ txid: string }>(w, "/broadcast", "POST", { psbt });

export type BumpRequest = {
  txid: string;
  /** Exact sat/vB; omit for the server's choice (fast estimate, at least old rate + 1). */
  fee_rate_sat_vb?: number;
};

/** An unsigned replacement PSBT plus the original transaction (hex), so the browser can check
 * one against the other with `reviewBump` (lib/wallet.ts). */
export type BumpResponse = {
  psbt: string;
  original_tx: string;
  fee_sat: number;
  fee_rate_sat_vb: number;
};

/** Asks for a fee bump (RBF) of one of our unconfirmed transactions. */
export const bumpFee = (w: WalletAuth, req: BumpRequest) =>
  walletRequest<BumpResponse>(w, "/bump", "POST", req);

// Token recovery: for a wallet whose API token was lost (e.g. "Forget" then restore). These
// need no token; the signature over the challenge proves we hold the wallet's key.

/** A one-time challenge to sign with the wallet's key. */
export const requestChallenge = (walletId: string) =>
  request<{ challenge: string; expires_in_secs: number }>(`/wallets/${walletId}/challenge`, {
    method: "POST",
  });

/** Trades a signed challenge for a new API token; the old token stops working. */
export const recoverToken = (walletId: string, challenge: string, signature: string) =>
  request<{ token: string }>(`/wallets/${walletId}/token`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ challenge, signature }),
  });
