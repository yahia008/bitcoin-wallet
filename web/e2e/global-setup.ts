// Runs once before the tests: the regtest node must be up, and the funder wallet must exist
// and hold coins that can be spent.

import { FUNDER, mine, rpc } from "./bitcoind";

export default async function globalSetup() {
  try {
    await rpc("getblockcount");
  } catch (e) {
    throw new Error(`Can't reach the regtest node (${e}). Start it first: docker compose up -d`);
  }
  const loaded = await rpc<string[]>("listwallets");
  if (!loaded.includes(FUNDER)) {
    try {
      await rpc("loadwallet", [FUNDER]);
    } catch {
      await rpc("createwallet", [FUNDER]);
    }
  }
  if ((await rpc<number>("getbalance", [], FUNDER)) < 50) {
    // Coinbase outputs need 100 confirmations before they can be spent.
    await mine(101);
  }
}
