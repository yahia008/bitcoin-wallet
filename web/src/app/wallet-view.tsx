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

/** The loaded wallet: balance, send, a receive address and history. Only sending needs the
 * password; the rest is public data from the server. */
export function WalletView({ wallet, onForget }: { wallet: StoredWallet; onForget: () => void }) {
  const [data, setData] = useState<Loaded>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  // Bumped by the Refresh button to fetch again.
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
    <>
      <Card title="Balance">
        {data ? (
          <>
            <p className="text-2xl font-semibold tabular-nums">
              {formatBtc(data.balance.total_sat)} <span className="text-base">BTC</span>
            </p>
            <p className="mt-1 text-sm text-zinc-500 tabular-nums">
              Confirmed {formatBtc(data.balance.confirmed_sat)} · Unconfirmed{" "}
              {formatBtc(data.balance.unconfirmed_sat)}
              {data.balance.immature_sat > 0 &&
                ` · Immature ${formatBtc(data.balance.immature_sat)}`}
            </p>
          </>
        ) : (
          !error && <p className="text-sm text-zinc-500">Loading…</p>
        )}
        {error && <p className="text-sm text-red-700 dark:text-red-400">{error}</p>}
        <div className="mt-3">
          <Button variant="secondary" onClick={refresh} disabled={loading}>
            {loading ? "Syncing…" : "Refresh"}
          </Button>
        </div>
      </Card>

      <Send wallet={wallet} onSent={refresh} />

      <Receive wallet={wallet} />

      <Card title="Transactions">
        {data && data.transactions.length === 0 && (
          <p className="text-sm text-zinc-500">No transactions yet.</p>
        )}
        {data && data.transactions.length > 0 && (
          <ul className="divide-y divide-zinc-200 text-sm dark:divide-zinc-800">
            {data.transactions.map((tx) => (
              <TxRow key={tx.txid} tx={tx} wallet={wallet} onChanged={refresh} />
            ))}
          </ul>
        )}
      </Card>

      <Card title="This wallet">
        <dl className="mb-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-zinc-500">Network</dt>
          <dd>{wallet.network}</dd>
          <dt className="text-zinc-500">Wallet id</dt>
          <dd className="font-mono">{wallet.walletId}</dd>
          <dt className="text-zinc-500">Server</dt>
          <dd className="font-mono">{wallet.apiUrl}</dd>
        </dl>
        <Button variant="secondary" onClick={forget}>
          Forget this wallet
        </Button>
      </Card>
    </>
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
  return (
    <li className="py-2">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="truncate font-mono text-xs">
            {url ? (
              <a href={url} target="_blank" rel="noreferrer" className="underline">
                {tx.txid}
              </a>
            ) : (
              tx.txid
            )}
          </p>
          <p className="text-xs text-zinc-500">
            {tx.confirmed
              ? `${tx.confirmations} confirmation${tx.confirmations === 1 ? "" : "s"}`
              : "Unconfirmed"}
            {tx.fee_sat !== null && ` · fee ${formatBtc(tx.fee_sat)}`}
          </p>
        </div>
        <p
          className={`shrink-0 font-mono tabular-nums ${
            received ? "text-green-700 dark:text-green-400" : ""
          }`}
        >
          {received ? "+" : ""}
          {formatBtc(tx.net_sat)}
        </p>
      </div>
      {canSpeedUp && !speedingUp && (
        <button className="mt-1 text-xs underline" onClick={() => setSpeedingUp(true)}>
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
  const qr = await QRCode.toDataURL(`bitcoin:${info.address}`, { margin: 1, width: 192 });
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
    <Card title="Receive">
      {address && (
        <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center">
          {/* eslint-disable-next-line @next/next/no-img-element -- a generated data: URL */}
          {qr && <img src={qr} alt={`QR code for ${address.address}`} width={192} height={192} />}
          <div className="min-w-0">
            <p className="break-all font-mono text-sm">{address.address}</p>
            <p className="mt-1 text-xs text-zinc-500">
              Address #{address.index} · checked against your own keys
            </p>
            <div className="mt-2 flex gap-2">
              <Button
                variant="secondary"
                onClick={() => navigator.clipboard.writeText(address.address)}
              >
                Copy
              </Button>
              <Button variant="secondary" onClick={fresh} disabled={busy}>
                New address
              </Button>
            </div>
          </div>
        </div>
      )}
      {!address && !error && <p className="text-sm text-zinc-500">Loading…</p>}
      {error && <p className="text-sm text-red-700 dark:text-red-400">{error}</p>}
    </Card>
  );
}
