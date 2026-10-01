"use client";

import { useEffect, useState } from "react";

import { API_URL, health, type Health } from "@/lib/api";
import { loadStoredWallet, type StoredWallet } from "@/lib/store";

import { Logo } from "./logo";
import { Card, WalletSetup } from "./wallet-setup";
import { WalletView } from "./wallet-view";

type Status =
  | { kind: "checking" }
  | { kind: "ok"; health: Health; wallet?: StoredWallet }
  | { kind: "error"; message: string };

export default function Home() {
  const [status, setStatus] = useState<Status>({ kind: "checking" });

  useEffect(() => {
    (async () => {
      const h = await health();
      // The wallet saved in this browser for the server's network, if any.
      setStatus({ kind: "ok", health: h, wallet: await loadStoredWallet(h.network) });
    })().catch((e: unknown) =>
      setStatus({ kind: "error", message: e instanceof Error ? e.message : String(e) }),
    );
  }, []);

  function setWallet(wallet?: StoredWallet) {
    setStatus((s) => (s.kind === "ok" ? { ...s, wallet } : s));
  }

  if (status.kind === "checking") {
    return (
      <main className="flex flex-1 items-center justify-center">
        <Logo className="h-12 w-12 animate-pulse" />
      </main>
    );
  }

  if (status.kind === "error") {
    return (
      <main className="mx-auto flex w-full max-w-md flex-1 flex-col items-center justify-center gap-6 px-4">
        <Logo className="h-12 w-12 opacity-60" />
        <Card title="Can't reach the wallet server">
          <p className="text-sm text-zinc-400">
            {status.message}. Is the server running at <span className="font-mono">{API_URL}</span>,
            with CORS allowing this page?
          </p>
        </Card>
      </main>
    );
  }

  if (!status.wallet) {
    return (
      <main className="flex flex-1 flex-col">
        <WalletSetup network={status.health.network} onReady={setWallet} />
      </main>
    );
  }

  return (
    <main className="mx-auto w-full max-w-md flex-1 px-4 py-6">
      <WalletView wallet={status.wallet} onForget={() => setWallet(undefined)} />
    </main>
  );
}
