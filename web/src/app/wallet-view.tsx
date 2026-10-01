"use client";

import QRCode from "qrcode";
import { useEffect, useState } from "react";

import {
  getAddresses,
  getBalance,
  getTransactions,
  newAddress,
  type AddressInfo,
  type Balance,
  type Transaction,
} from "@/lib/api";
import { explorerTxUrl, formatBtc } from "@/lib/format";
import { forgetStoredWallet, type StoredWallet } from "@/lib/store";
import { verifyReceiveAddress } from "@/lib/wallet";

import { ArrowDownIcon, ArrowUpIcon, BackIcon, CloseIcon, DotsIcon, RefreshIcon } from "./icons";
import { Logo } from "./logo";
import { Send } from "./send";
import { SpeedUp } from "./speed-up";
import { Button, Card } from "./wallet-setup";

type Loaded = { balance: Balance; transactions: Transaction[] };

async function fetchOverview(wallet: StoredWallet): Promise<Loaded> {
  // One after the other: the server handles one request per wallet at a time anyway.
  const balance = await getBalance(wallet);
  const transactions = await getTransactions(wallet);
  return { balance, transactions };
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

type View = "home" | "send" | "receive" | "settings";

/** The loaded wallet, as a dashboard: balance and actions up top, activity below; Receive,
 * Send and settings open as their own screens. Only sending needs the password; the rest is
 * public data from the server. */
export function WalletView({ wallet, onForget }: { wallet: StoredWallet; onForget: () => void }) {
  const [view, setView] = useState<View>("home");
  const [data, setData] = useState<Loaded>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  // Bumped by Refresh (and after sending) to fetch again.
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let current = true; // ignore answers that arrive after a newer request or unmount
    fetchOverview(wallet)
      .then(
        (loaded) => current && (setData(loaded), setError(undefined)),
        (e) => current && setError(message(e)),
      )
      .finally(() => current && setLoading(false));
    return () => {
      current = false;
    };
  }, [wallet, version]);

  function refresh() {
    setLoading(true);
    setVersion((v) => v + 1);
  }

  const home = () => setView("home");

  if (view === "send") {
    return (
      <Screen title="Send" onBack={home}>
        <Send wallet={wallet} onSent={refresh} />
      </Screen>
    );
  }
  if (view === "receive") {
    return (
      <Screen title="Receive" onBack={home}>
        <Receive wallet={wallet} />
      </Screen>
    );
  }
  if (view === "settings") {
    return (
      <Screen title="Wallet" onBack={home}>
        <Settings wallet={wallet} onForget={onForget} />
      </Screen>
    );
  }

  const balance = data?.balance;
  return (
    <div className="flex flex-col">
      <div className="flex items-center justify-between">
        <IconButton label="Wallet settings" onClick={() => setView("settings")}>
          <DotsIcon />
        </IconButton>
        <span className="rounded-full border border-zinc-800 px-3 py-1 text-xs text-zinc-400">
          {wallet.network}
        </span>
      </div>

      <div className="mt-8 flex flex-col items-center text-center">
        <p className="flex items-center gap-2 text-sm text-zinc-400">
          <Logo className="h-4 w-4" />
          Wallet · {wallet.walletId.slice(0, 4)}…{wallet.walletId.slice(-4)}
        </p>
        <p className="mt-3 text-4xl font-semibold tabular-nums tracking-tight">
          {balance ? formatBtc(balance.total_sat) : "—"}
          <span className="ml-2 text-xl text-zinc-400">BTC</span>
        </p>
        <p className="mt-2 h-5 text-sm text-zinc-500 tabular-nums">
          {error ? (
            <span className="text-red-500">{error}</span>
          ) : balance && balance.unconfirmed_sat !== 0 ? (
            `${formatBtc(balance.unconfirmed_sat)} BTC unconfirmed`
          ) : balance && balance.immature_sat > 0 ? (
            `${formatBtc(balance.immature_sat)} BTC immature`
          ) : loading ? (
            "Syncing…"
          ) : null}
        </p>
      </div>

      <div className="mt-6 grid grid-cols-3 gap-2">
        <Tile label="Receive" onClick={() => setView("receive")}>
          <ArrowDownIcon />
        </Tile>
        <Tile label="Send" onClick={() => setView("send")}>
          <ArrowUpIcon />
        </Tile>
        <Tile label={loading ? "Syncing" : "Refresh"} onClick={refresh} disabled={loading}>
          <RefreshIcon spinning={loading} />
        </Tile>
      </div>

      <NetworkBanner network={wallet.network} />

      <Tabs
        tabs={{
          Activity: (
            <Activity
              wallet={wallet}
              transactions={data?.transactions}
              onChanged={refresh}
              onReceive={() => setView("receive")}
            />
          ),
          Addresses: <Addresses wallet={wallet} version={version} />,
        }}
      />
    </div>
  );
}

/** A sub-screen with a back arrow and title. */
function Screen({
  title,
  onBack,
  children,
}: {
  title: string;
  onBack: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-[2.5rem_1fr_2.5rem] items-center">
        <IconButton label="Back" onClick={onBack}>
          <BackIcon />
        </IconButton>
        <h1 className="text-center text-lg font-semibold">{title}</h1>
      </div>
      {children}
    </div>
  );
}

function Tile({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex h-20 flex-col items-center justify-center gap-2 rounded-2xl bg-zinc-800/60 text-sm font-semibold transition-colors hover:bg-zinc-800 disabled:opacity-60"
    >
      <span className="text-violet-400">{children}</span>
      {label}
    </button>
  );
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={onClick}
      className="flex h-10 w-10 items-center justify-center rounded-full border border-zinc-800 text-zinc-300 transition-colors hover:border-zinc-600"
    >
      {children}
    </button>
  );
}

/** A dismissible reminder that test-network coins have no value; remembered per network. */
function NetworkBanner({ network }: { network: string }) {
  const key = `osok:banner-dismissed:${network}`;
  const [dismissed, setDismissed] = useState(() => {
    try {
      return typeof window !== "undefined" && window.localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  });
  if (dismissed) return null;
  function dismiss() {
    setDismissed(true);
    try {
      window.localStorage.setItem(key, "1");
    } catch {
      // Storage unavailable (private mode): it just shows again next time.
    }
  }
  return (
    <div className="mt-4 flex items-start gap-3 rounded-2xl bg-violet-950/60 p-4">
      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-violet-500/20 text-violet-300">
        <Logo className="h-6 w-6" />
      </span>
      <div className="flex-1">
        <p className="font-semibold">You&apos;re on {network}</p>
        <p className="text-sm text-violet-200/70">Coins here are for testing and have no value.</p>
      </div>
      <button aria-label="Dismiss" onClick={dismiss} className="text-zinc-400 hover:text-zinc-100">
        <CloseIcon />
      </button>
    </div>
  );
}

function Tabs({ tabs }: { tabs: Record<string, React.ReactNode> }) {
  const names = Object.keys(tabs);
  const [active, setActive] = useState(names[0]);
  return (
    <div className="mt-6">
      <div className="flex gap-6 border-b border-zinc-800">
        {names.map((name) => (
          <button
            key={name}
            onClick={() => setActive(name)}
            className={`-mb-px border-b-2 pb-3 text-sm font-semibold transition-colors ${
              active === name
                ? "border-violet-400 text-violet-300"
                : "border-transparent text-zinc-400 hover:text-zinc-200"
            }`}
          >
            {name}
          </button>
        ))}
      </div>
      <div className="pt-2">{tabs[active]}</div>
    </div>
  );
}

function Activity({
  wallet,
  transactions,
  onChanged,
  onReceive,
}: {
  wallet: StoredWallet;
  transactions?: Transaction[];
  onChanged: () => void;
  onReceive: () => void;
}) {
  if (!transactions) return <p className="py-8 text-center text-sm text-zinc-500">Loading…</p>;
  if (transactions.length === 0) {
    return (
      <div className="flex flex-col items-center py-10 text-center">
        <span className="flex h-12 w-12 items-center justify-center rounded-full bg-zinc-800/60 text-zinc-400">
          <ArrowDownIcon />
        </span>
        <p className="mt-4 text-lg">Looking a little empty…</p>
        <p className="mt-1 max-w-xs text-sm text-zinc-400">
          Receive some bitcoin to get started. Your transactions will show up here.
        </p>
        <div className="mt-5">
          <Button onClick={onReceive}>Receive</Button>
        </div>
      </div>
    );
  }
  return (
    <ul className="divide-y divide-zinc-800">
      {transactions.map((tx) => (
        <TxRow key={tx.txid} tx={tx} wallet={wallet} onChanged={onChanged} />
      ))}
    </ul>
  );
}

/** Receive addresses handed out so far, each re-checked against our own descriptor. */
function Addresses({ wallet, version }: { wallet: StoredWallet; version: number }) {
  const [rows, setRows] = useState<(AddressInfo & { ok: boolean })[]>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let current = true;
    getAddresses(wallet)
      .then((list) =>
        Promise.all(
          list.map((a) =>
            verifyReceiveAddress(wallet, a).then(
              () => ({ ...a, ok: true }),
              () => ({ ...a, ok: false }),
            ),
          ),
        ),
      )
      .then(
        (checked) => current && setRows(checked.reverse()),
        (e) => current && setError(message(e)),
      );
    return () => {
      current = false;
    };
  }, [wallet, version]);

  if (error) return <p className="py-8 text-center text-sm text-red-500">{error}</p>;
  if (!rows) return <p className="py-8 text-center text-sm text-zinc-500">Loading…</p>;
  if (rows.length === 0) {
    return <p className="py-8 text-center text-sm text-zinc-500">No addresses yet.</p>;
  }
  return (
    <ul className="divide-y divide-zinc-800">
      {rows.map((a) => (
        <li key={a.index} className="flex items-center justify-between gap-4 py-3">
          <div className="min-w-0">
            <p className="truncate font-mono text-xs">{a.address}</p>
            <p className="text-xs text-zinc-500">#{a.index}</p>
          </div>
          {a.ok ? (
            <span
              className={`shrink-0 rounded-full px-2 py-0.5 text-xs ${
                a.used ? "bg-zinc-800 text-zinc-400" : "bg-lime-500/15 text-lime-300"
              }`}
            >
              {a.used ? "Used" : "Unused"}
            </span>
          ) : (
            <span className="shrink-0 text-xs text-red-500">Not yours!</span>
          )}
        </li>
      ))}
    </ul>
  );
}

function Settings({ wallet, onForget }: { wallet: StoredWallet; onForget: () => void }) {
  async function forget() {
    const sure = window.confirm(
      "Remove this wallet from this browser? Your coins are not affected: restore it any " +
        "time with your recovery phrase.",
    );
    if (sure) {
      await forgetStoredWallet(wallet.network);
      onForget();
    }
  }
  return (
    <Card>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
        <dt className="text-zinc-500">Network</dt>
        <dd>{wallet.network}</dd>
        <dt className="text-zinc-500">Wallet id</dt>
        <dd className="font-mono">{wallet.walletId}</dd>
        <dt className="text-zinc-500">Server</dt>
        <dd className="break-all font-mono">{wallet.apiUrl}</dd>
        <dt className="text-zinc-500">Created</dt>
        <dd>{new Date(wallet.createdAt).toLocaleString()}</dd>
      </dl>
      <div className="mt-6">
        <Button variant="secondary" onClick={forget}>
          Forget this wallet
        </Button>
      </div>
    </Card>
  );
}

function TxRow({
  tx,
  wallet,
  onChanged,
}: {
  tx: Transaction;
  wallet: StoredWallet;
  onChanged: () => void;
}) {
  const [speedingUp, setSpeedingUp] = useState(false);
  const url = explorerTxUrl(wallet.network, tx.txid);
  const received = tx.net_sat >= 0;
  // Only transactions we paid the fee for (we sent them), and only while unconfirmed.
  const canSpeedUp = !tx.confirmed && tx.fee_sat !== null;
  const status = tx.confirmed
    ? `${tx.confirmations} confirmation${tx.confirmations === 1 ? "" : "s"}`
    : "Unconfirmed";
  return (
    <li className="py-3">
      <div className="flex items-center gap-3">
        <span
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full ${
            received ? "bg-lime-500/15 text-lime-300" : "bg-violet-500/15 text-violet-300"
          }`}
        >
          {received ? <ArrowDownIcon /> : <ArrowUpIcon />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold">{received ? "Received" : "Sent"}</p>
          <p className="truncate text-xs text-zinc-500">
            {status}
            {tx.fee_sat !== null && ` · fee ${formatBtc(tx.fee_sat)}`}
            {" · "}
            {url ? (
              <a href={url} target="_blank" rel="noreferrer" className="underline">
                {tx.txid.slice(0, 8)}…
              </a>
            ) : (
              <span className="font-mono">{tx.txid.slice(0, 8)}…</span>
            )}
          </p>
        </div>
        <p
          className={`shrink-0 text-sm font-semibold tabular-nums ${
            received ? "text-lime-300" : ""
          }`}
        >
          {received ? "+" : ""}
          {formatBtc(tx.net_sat)}
        </p>
      </div>
      {canSpeedUp && !speedingUp && (
        <button
          className="mt-2 ml-12 text-xs font-semibold text-violet-300 hover:underline"
          onClick={() => setSpeedingUp(true)}
        >
          Speed up
        </button>
      )}
      {speedingUp && (
        <SpeedUp
          wallet={wallet}
          txid={tx.txid}
          onCancel={() => setSpeedingUp(false)}
          onDone={() => {
            setSpeedingUp(false);
            onChanged();
          }}
        />
      )}
    </li>
  );
}

/** A receive address checked against our own descriptor, plus its QR code. Reuses the newest
 * unused address unless `fresh`: wallets restoring from the phrase stop looking after 20
 * unused addresses in a row, so we don't reveal more than needed. */
async function fetchReceive(
  wallet: StoredWallet,
  fresh: boolean,
): Promise<{ info: AddressInfo; qr: string }> {
  const unused = fresh ? undefined : (await getAddresses(wallet)).filter((a) => !a.used).at(-1);
  const info = unused ?? (await newAddress(wallet));
  await verifyReceiveAddress(wallet, info);
  // BIP21 URI, so phone wallets scanning it know it's a bitcoin address.
  const qr = await QRCode.toDataURL(`bitcoin:${info.address}`, { margin: 2, width: 440 });
  return { info, qr };
}

/** Shows an unused receive address (checked against our own descriptor) with a QR code. */
function Receive({ wallet }: { wallet: StoredWallet }) {
  const [shown, setShown] = useState<{ info: AddressInfo; qr: string }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Bumped by "New address"; 0 means reuse an unused one.
  const [freshCount, setFreshCount] = useState(0);

  useEffect(() => {
    let current = true;
    fetchReceive(wallet, freshCount > 0)
      .then(
        (r) => current && (setShown(r), setError(undefined)),
        (e) => current && setError(message(e)),
      )
      .finally(() => current && setBusy(false));
    return () => {
      current = false;
    };
  }, [wallet, freshCount]);

  function fresh() {
    setBusy(true);
    setFreshCount((n) => n + 1);
  }

  const address = shown?.info;
  const qr = shown?.qr;

  return (
    <Card>
      {address && (
        <div className="flex flex-col items-center text-center">
          {qr && (
            // eslint-disable-next-line @next/next/no-img-element -- a generated data: URL
            <img
              src={qr}
              alt={`QR code for ${address.address}`}
              width={220}
              height={220}
              className="rounded-xl"
            />
          )}
          <p className="mt-4 break-all font-mono text-sm">{address.address}</p>
          <p className="mt-1 text-xs text-zinc-500">
            Address #{address.index} · checked against your own keys
          </p>
          <div className="mt-4 flex gap-2">
            <Button onClick={() => navigator.clipboard.writeText(address.address)}>Copy</Button>
            <Button variant="secondary" onClick={fresh} disabled={busy}>
              New address
            </Button>
          </div>
        </div>
      )}
      {!address && !error && <p className="text-center text-sm text-zinc-500">Loading…</p>}
      {error && <p className="text-sm text-red-500">{error}</p>}
    </Card>
  );
}
