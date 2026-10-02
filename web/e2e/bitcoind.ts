// Talks to the regtest Bitcoin Core node (docker-compose.yml) over JSON-RPC, to fund wallets
// and mine blocks during the tests.

const RPC_URL = process.env.BITCOIN_RPC_URL ?? "http://127.0.0.1:18443";
const RPC_AUTH = Buffer.from(process.env.BITCOIN_RPC_AUTH ?? "wallet:wallet").toString("base64");
/** The node wallet the tests pay from. */
export const FUNDER = "e2e-funder";

export async function rpc<T = unknown>(
  method: string,
  params: unknown[] = [],
  wallet?: string,
): Promise<T> {
  const url = wallet ? `${RPC_URL}/wallet/${wallet}` : RPC_URL;
  const res = await fetch(url, {
    method: "POST",
    headers: { authorization: `Basic ${RPC_AUTH}`, "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "1.0", id: "e2e", method, params }),
  });
  const body = await res.json();
  if (body.error) throw new Error(`${method}: ${body.error.message}`);
  return body.result as T;
}

export async function mine(blocks = 1): Promise<void> {
  const address = await rpc<string>("getnewaddress", [], FUNDER);
  await rpc("generatetoaddress", [blocks, address]);
}

/** Pays `btc` to `address` from the funder wallet and mines it into a block. */
export async function fund(address: string, btc: number): Promise<string> {
  const txid = await rpc<string>("sendtoaddress", [address, btc], FUNDER);
  await mine(1);
  return txid;
}

/** A fresh address of the funder wallet, for the wallet under test to pay to. */
export const newFunderAddress = () => rpc<string>("getnewaddress", [], FUNDER);

export async function inMempool(txid: string): Promise<boolean> {
  return (await rpc<string[]>("getrawmempool")).includes(txid);
}
