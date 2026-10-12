"use client";

import { useEffect, useState } from "react";

import { API_SERVERS, health, type ApiServer, type Health } from "@/lib/api";
import { loadAccounts, setActiveAccount, type Accounts, type StoredWallet } from "@/lib/store";
import { errorMessage } from "@/lib/errors";

import { Logo } from "./logo";
import { NetworkBadge } from "./network-menu";
import { Card, WalletSetup } from "./wallet-setup";
import { WalletView } from "./wallet-view";

type Status =
  | { kind: "checking" }
  | { kind: "ok"; server: ApiServer; health: Health; accounts: Accounts }
  | { kind: "error"; server: ApiServer; message: string };

/** The network last picked in the network menu, remembered in this browser. */
const NETWORK_KEY = "osok:network";

/** The API server for `network`, or the default (first) one. */
function serverFor(network: string | null): ApiServer {
  return API_SERVERS.find((s) => s.network === network) ?? API_SERVERS[0];
}

export default function Home() {
  const [status, setStatus] = useState<Status>({ kind: "checking" });
  // Bumped when a network is picked, to connect again.
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let current = true; // ignore a server that answers after the user switched away
    const server = serverFor(window.localStorage.getItem(NETWORK_KEY));
    (async () => {
      const h = await health(server.url);
      if (server.network && h.network !== server.network) {
        throw new Error(
          `The ${server.network} wallet server at ${server.url} is on ${h.network} instead`,
        );
      }
      // The accounts saved in this browser for the server's network, if any.
      const accounts = await loadAccounts(h.network);
      if (current) setStatus({ kind: "ok", server, health: h, accounts });
    })().catch(
      (e: unknown) => current && setStatus({ kind: "error", server, message: errorMessage(e) }),
    );
    return () => {
      current = false;
    };
  }, [version]);

  /** Shows `network`'s wallet: its own server, and its own accounts in this browser. */
  function switchNetwork(network: string) {
    window.localStorage.setItem(NETWORK_KEY, network);
    setStatus({ kind: "checking" });
    setVersion((v) => v + 1);
  }

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
    const { server } = status;
    return (
      <main className="relative mx-auto flex w-full max-w-md flex-1 flex-col items-center justify-center gap-6 px-4">
        {/* So a network whose server is down doesn't trap the user on it. */}
        {server.network && (
          <div className="absolute top-6 right-4">
            <NetworkBadge network={server.network} onSelect={switchNetwork} />
          </div>
        )}
        <Logo className="h-12 w-12 opacity-60" />
        <Card title="Can't reach the wallet server">
          <p className="text-sm text-zinc-400">{status.message}</p>
          <p className="mt-3 text-xs text-zinc-500">
            Osok looks for {server.network ? `the ${server.network} server` : "it"} at{" "}
            <span className="font-mono">{server.url}</span>. If you run the server yourself,
            check that it&apos;s started and allows this page (CORS).
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
        <WalletSetup
          // Fresh for each network: a half-done setup doesn't carry over to another one.
          key={network}
          network={network}
          apiUrl={status.server.url}
          onReady={() => reload(network)}
          onNetworkChange={switchNetwork}
        />
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
          onNetworkChange={switchNetwork}
        />
      </div>
    </main>
  );
}
