"use client";

import { forgetStoredWallet, type StoredWallet } from "@/lib/store";

import { Button, Card } from "./wallet-setup";

/** The loaded wallet. Balance, history and sending come in later milestones. */
export function WalletView({ wallet, onForget }: { wallet: StoredWallet; onForget: () => void }) {
  async function forget() {
    const sure = window.confirm(
      "Remove this wallet from this browser? Your coins are not affected, and your recovery " +
        "phrase restores them. But this browser's API token for this server is deleted too, " +
        "and the server only issues one per wallet: restoring here against the same server " +
        "won't work.",
    );
    if (sure) {
      await forgetStoredWallet(wallet.network);
      onForget();
    }
  }

  return (
    <Card title="Your wallet">
      <dl className="mb-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-zinc-500">Network</dt>
        <dd>{wallet.network}</dd>
        <dt className="text-zinc-500">Wallet id</dt>
        <dd className="font-mono">{wallet.walletId}</dd>
        <dt className="text-zinc-500">First address</dt>
        <dd className="break-all font-mono">{wallet.firstAddress}</dd>
      </dl>
      <Button variant="secondary" onClick={forget}>
        Forget this wallet
      </Button>
    </Card>
  );
}
