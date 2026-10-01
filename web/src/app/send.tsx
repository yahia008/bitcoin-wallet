"use client";

import { useState } from "react";

import { broadcast, buildPsbt, type FeePriority } from "@/lib/api";
import { explorerTxUrl, formatBtc, parseBtc } from "@/lib/format";
import type { StoredWallet } from "@/lib/store";
import { reviewSend, signPsbt, type SendReview } from "@/lib/wallet";

import { Button, Card } from "./wallet-setup";

type Step =
  | { kind: "form" }
  | { kind: "review"; to: string; amountSat: number; psbt: string; review: SendReview }
  | { kind: "sent"; txid: string };

const input = "w-full rounded-lg border border-zinc-700 bg-transparent p-2 text-sm";

/** Pay someone: the server builds an unsigned PSBT, this browser checks it with the CLI's
 * review code, and signs it locally once the user enters their password. */
export function Send({ wallet, onSent }: { wallet: StoredWallet; onSent: () => void }) {
  const [step, setStep] = useState<Step>({ kind: "form" });
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [priority, setPriority] = useState<FeePriority>("normal");
  const [password, setPassword] = useState("");
  const [allowHighFee, setAllowHighFee] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const amountSat = parseBtc(amount);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const prepare = () =>
    run(async () => {
      if (amountSat === undefined) throw new Error("Enter an amount in BTC, e.g. 0.001");
      const address = to.trim();
      const built = await buildPsbt(wallet, {
        address,
        amount_sat: amountSat,
        fee_priority: priority,
      });
      // Don't trust the server's summary: check what the PSBT really does.
      const review = await reviewSend(wallet, built.psbt, address, amountSat);
      setAllowHighFee(false);
      setStep({ kind: "review", to: address, amountSat, psbt: built.psbt, review });
    });

  const signAndSend = (s: Extract<Step, { kind: "review" }>) =>
    run(async () => {
      const signed = await signPsbt(wallet, password, s.psbt);
      const { txid } = await broadcast(wallet, signed);
      setPassword("");
      setStep({ kind: "sent", txid });
      onSent();
    });

  function reset() {
    setStep({ kind: "form" });
    setTo("");
    setAmount("");
    setPassword("");
    setError(undefined);
  }

  if (step.kind === "sent") {
    const url = explorerTxUrl(wallet.network, step.txid);
    return (
      <Card title="Send">
        <p className="text-sm text-green-700 dark:text-green-400">Sent. It confirms once mined.</p>
        <p className="mt-1 break-all font-mono text-xs">
          {url ? (
            <a href={url} target="_blank" rel="noreferrer" className="underline">
              {step.txid}
            </a>
          ) : (
            step.txid
          )}
        </p>
        <div className="mt-3">
          <Button variant="secondary" onClick={reset}>
            Send another
          </Button>
        </div>
      </Card>
    );
  }

  if (step.kind === "review") {
    const { review } = step;
    const blocked = !!review.feeWarning && !allowHighFee;
    return (
      <Card title="Check and sign">
        <p className="mb-2 text-sm text-green-700 dark:text-green-400">
          Verified in this browser: pays exactly the recipient, everything else comes back to you.
        </p>
        <dl className="mb-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm tabular-nums">
          <dt className="text-zinc-500">To</dt>
          <dd className="break-all font-mono">{step.to}</dd>
          <dt className="text-zinc-500">Amount</dt>
          <dd>{formatBtc(step.amountSat)} BTC</dd>
          <dt className="text-zinc-500">Fee</dt>
          <dd>
            {formatBtc(review.feeSat)} BTC (~{review.feeRate.toFixed(1)} sat/vB)
          </dd>
          <dt className="text-zinc-500">Change</dt>
          <dd>{formatBtc(review.changeSat)} BTC</dd>
          <dt className="text-zinc-500">Total out</dt>
          <dd className="font-medium">{formatBtc(step.amountSat + review.feeSat)} BTC</dd>
        </dl>
        {review.feeWarning && (
          <label className="mb-3 flex items-start gap-2 text-sm text-red-700 dark:text-red-400">
            <input
              type="checkbox"
              className="mt-1 accent-violet-500"
              checked={allowHighFee}
              onChange={(e) => setAllowHighFee(e.target.checked)}
            />
            <span>The fee looks like a mistake: {review.feeWarning}. Tick to send anyway.</span>
          </label>
        )}
        <input
          type="password"
          className={`${input} mb-3`}
          placeholder="wallet password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {error && <p className="mb-2 text-sm text-red-700 dark:text-red-400">{error}</p>}
        <div className="flex gap-2">
          <Button onClick={() => signAndSend(step)} disabled={busy || !password || blocked}>
            {busy ? "Signing…" : "Sign and send"}
          </Button>
          <Button variant="secondary" onClick={reset} disabled={busy}>
            Cancel
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <Card title="Send">
      <div className="flex flex-col gap-2">
        <input
          className={`${input} font-mono`}
          placeholder="recipient address"
          value={to}
          onChange={(e) => setTo(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        <input
          className={input}
          placeholder="amount in BTC, e.g. 0.001"
          inputMode="decimal"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
        />
        <select
          className={input}
          value={priority}
          onChange={(e) => setPriority(e.target.value as FeePriority)}
        >
          <option value="fast">Fast (~20 min)</option>
          <option value="normal">Normal (~1 hour)</option>
          <option value="slow">Slow (~1 day, cheapest)</option>
        </select>
      </div>
      {error && <p className="mt-2 text-sm text-red-700 dark:text-red-400">{error}</p>}
      <div className="mt-3">
        <Button onClick={prepare} disabled={busy || !to.trim() || amountSat === undefined}>
          {busy ? "Preparing…" : "Review"}
        </Button>
      </div>
    </Card>
  );
}
