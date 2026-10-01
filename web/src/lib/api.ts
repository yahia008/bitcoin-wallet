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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_URL}${path}`, init);
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
