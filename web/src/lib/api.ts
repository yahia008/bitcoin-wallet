// Talks to the watch-only wallet API (src/bin/server.rs). Everything sent here is public:
// descriptors, addresses, unsigned and signed PSBTs. Never mnemonics or private keys.

/** Where the API server runs. Baked in at build time (NEXT_PUBLIC_ variables are). */
export const API_URL = process.env.NEXT_PUBLIC_API_URL ?? "http://127.0.0.1:3000";

export type Health = {
  status: string;
  /** e.g. "testnet4"; the wallet must use the same network as the server. */
  network: string;
};

export async function health(): Promise<Health> {
  const res = await fetch(`${API_URL}/health`);
  if (!res.ok) {
    throw new Error(`API answered ${res.status}`);
  }
  return res.json();
}
