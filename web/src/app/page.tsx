"use client";

import { useEffect, useState } from "react";

import { API_URL, health, type Health } from "@/lib/api";
import { loadAccounts, setActiveAccount, type Accounts, type StoredWallet } from "@/lib/store";

import { Logo } from "./logo";
import { Card, WalletSetup } from "./wallet-setup";
import { WalletView } from "./wallet-view";

type Status =
  | { kind: "checking" }
  | { kind: "ok"; health: Health; accounts: Accounts }
  | { kind: "error"; message: string };

export default function Home() {
  const [status, setStatus] = useState<Status>({ kind: "checking" });

  useEffect(() => {
    (async () => {
      const h = await health();
      // The accounts saved in this browser for the server's network, if any.
      setStatus({ kind: "ok", health: h, accounts: await loadAccounts(h.network) });
    })().catch((e: unknown) =>
      setStatus({ kind: "error", message: e instanceof Error ? e.message : String(e) }),
    );
  }, []);

  function setAccounts(accounts: Accounts) {
    setStatus((s) => (s.kind === "ok" ? { ...s, accounts } : s));
  }

  /** Reloads from storage, e.g. after an account was added or the wallet forgotten. */
  async function reload(network: string) {
    setAccounts(await loadAccounts(network));
  }

  async function switchTo(wallet: StoredWallet) {
    await setActiveAccount(wallet.network, wallet.walletId);
    setStatus((s) => (s.kind === "ok" ? { ...s, accounts: { ...s.accounts, active: wallet } } : s));
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

  const { network } = status.health;
  const { accounts, active } = status.accounts;
  if (!active) {
    return (
      <main className="flex flex-1 flex-col">
        <WalletSetup network={network} onReady={() => reload(network)} />
      </main>
    );
  }

  return (
    // On wider screens the wallet sits in its own dark panel, a little down from the top, on
    // a lighter backdrop; on a phone it simply fills the screen.
    <main className="flex flex-1 justify-center sm:bg-zinc-800/60 sm:px-4 sm:py-16">
      <div className="flex w-full max-w-md flex-col bg-background px-4 py-6 sm:min-h-[640px] sm:self-start sm:rounded-3xl sm:border sm:border-zinc-800 sm:px-6 sm:shadow-2xl">
        {/* Keyed by account, so switching starts the dashboard fresh for that account. */}
        <WalletView
          key={active.walletId}
          wallet={active}
          accounts={accounts}
          onSwitch={switchTo}
          onAccountsChanged={() => reload(network)}
        />
      </div>
    </main>
  );
}
