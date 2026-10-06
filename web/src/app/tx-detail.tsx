"use client";

import { useEffect, useState } from "react";

import { getTransaction, type TransactionDetail, type TxIo } from "@/lib/api";
import { explorerTxUrl, formatBtc, formatBtcShort } from "@/lib/format";
import type { StoredWallet } from "@/lib/store";
import { errorMessage } from "@/lib/errors";

import { ArrowDownIcon, ArrowUpIcon, CheckIcon, CopyIcon, ExternalLinkIcon } from "./icons";
import { SpeedUp } from "./speed-up";
import { Button, Card } from "./wallet-setup";

/** One transaction in full: amount, status, fee, its inputs and outputs (ours marked), and
 * Speed up while it's unconfirmed. Everything here comes from the server, so it's for
 * information only; anything we sign is still checked in WASM first. */
export function TxDetail({
  wallet,
  txid,
  onChanged,
}: {
  wallet: StoredWallet;
  txid: string;
  /** It was replaced by a sped-up copy: the list needs reloading and this txid is gone. */
  onChanged: () => void;
}) {
  const [tx, setTx] = useState<TransactionDetail>();
  const [error, setError] = useState<string>();
  const [speedingUp, setSpeedingUp] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let current = true;
    getTransaction(wallet, txid).then(
      (t) => current && setTx(t),
      (e) => current && setError(errorMessage(e)),
    );
    return () => {
      current = false;
    };
  }, [wallet, txid]);

  async function copy() {
    await navigator.clipboard.writeText(txid);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  if (error) return <p className="py-8 text-center text-sm text-red-500">{error}</p>;
  if (!tx) return <p className="py-8 text-center text-sm text-zinc-500">Loading…</p>;

  const received = tx.net_sat >= 0;
  const url = explorerTxUrl(wallet.network, tx.txid);
  // Only transactions we paid the fee for (we sent them), and only while unconfirmed.
  const ours = tx.fee_sat !== null;
  const status = tx.confirmed
    ? `${tx.confirmations} confirmation${tx.confirmations === 1 ? "" : "s"}`
    : "Unconfirmed";

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col items-center pt-2 text-center">
        <span
          className={`flex h-12 w-12 items-center justify-center rounded-full ${
            received ? "bg-lime-500/15 text-lime-300" : "bg-violet-500/15 text-violet-300"
          }`}
        >
          {received ? <ArrowDownIcon /> : <ArrowUpIcon />}
        </span>
        <p className="mt-3 text-sm text-zinc-400">{received ? "Received" : "Sent"}</p>
        <p
          className={`mt-1 text-3xl font-semibold tabular-nums ${received ? "text-lime-300" : ""}`}
        >
          {received ? "+" : ""}
          {formatBtcShort(tx.net_sat)}
          <span className="ml-2 text-lg text-zinc-400">BTC</span>
        </p>
        <span
          className={`mt-3 rounded-full px-3 py-1 text-xs ${
            tx.confirmed ? "bg-zinc-800 text-zinc-300" : "bg-amber-500/15 text-amber-300"
          }`}
        >
          {status}
        </span>
      </div>

      {!tx.confirmed && ours && !speedingUp && (
        <div className="flex flex-col items-center gap-1">
          {tx.rbf ? (
            <Button onClick={() => setSpeedingUp(true)}>Speed up</Button>
          ) : (
            <p className="text-xs text-zinc-500">
              This transaction doesn&apos;t allow fee bumps (no replace-by-fee signal).
            </p>
          )}
        </div>
      )}
      {speedingUp && (
        <SpeedUp
          wallet={wallet}
          txid={tx.txid}
          onCancel={() => setSpeedingUp(false)}
          onDone={onChanged}
        />
      )}

      <Card>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm tabular-nums">
          <dt className="text-zinc-500">{tx.confirmed ? "Date" : "First seen"}</dt>
          <dd>{tx.time ? new Date(tx.time * 1000).toLocaleString() : "—"}</dd>
          {tx.block_height !== null && (
            <>
              <dt className="text-zinc-500">Block</dt>
              <dd>{tx.block_height.toLocaleString()}</dd>
            </>
          )}
          {tx.fee_sat !== null && (
            <>
              <dt className="text-zinc-500">Fee</dt>
              <dd>
                {formatBtc(tx.fee_sat)} BTC
                {tx.fee_rate_sat_vb !== null && (
                  <span className="text-zinc-500"> · {tx.fee_rate_sat_vb.toFixed(1)} sat/vB</span>
                )}
              </dd>
            </>
          )}
          <dt className="text-zinc-500">Size</dt>
          <dd>{tx.vsize} vB</dd>
          <dt className="text-zinc-500">Transaction ID</dt>
          <dd className="flex items-start gap-2">
            <span className="min-w-0 break-all font-mono text-xs">{tx.txid}</span>
            <button
              aria-label={copied ? "Copied" : "Copy transaction ID"}
              title={copied ? "Copied" : "Copy transaction ID"}
              onClick={copy}
              className="shrink-0 text-zinc-400 hover:text-zinc-100"
            >
              {copied ? <CheckIcon /> : <CopyIcon />}
            </button>
          </dd>
        </dl>
        {url && (
          <a
            href={url}
            target="_blank"
            rel="noreferrer"
            className="mt-4 inline-flex items-center gap-2 text-sm font-semibold text-violet-300 hover:underline"
          >
            View on block explorer <ExternalLinkIcon />
          </a>
        )}
      </Card>

      <IoList title={`From (${tx.inputs.length})`} items={tx.inputs} />
      <IoList title={`To (${tx.outputs.length})`} items={tx.outputs} />
    </div>
  );
}

function IoList({ title, items }: { title: string; items: TxIo[] }) {
  return (
    <Card title={title}>
      <ul className="divide-y divide-zinc-800">
        {items.map((io, i) => (
          <li key={i} className="flex items-center justify-between gap-4 py-2">
            <div className="min-w-0">
              <p className="truncate font-mono text-xs">
                {io.address ?? (io.value_sat === null ? "External input" : "No address")}
              </p>
              {io.owner !== "external" && (
                <span className="mt-1 inline-block rounded-full bg-violet-500/15 px-2 py-0.5 text-[11px] text-violet-300">
                  {io.owner === "change" ? "Your change" : "You"}
                </span>
              )}
            </div>
            <p className="shrink-0 text-sm tabular-nums">
              {io.value_sat === null ? "—" : `${formatBtcShort(io.value_sat)} BTC`}
            </p>
          </li>
        ))}
      </ul>
    </Card>
  );
}
