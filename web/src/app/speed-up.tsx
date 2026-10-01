"use client";

import { useState } from "react";

import { broadcast, bumpFee } from "@/lib/api";
import { formatBtc } from "@/lib/format";
import type { StoredWallet } from "@/lib/store";
import { reviewBump, signPsbt, type BumpReview } from "@/lib/wallet";

import { Button } from "./wallet-setup";

const input = "rounded border border-zinc-300 bg-transparent p-2 text-sm dark:border-zinc-700";

/** Replaces one of our unconfirmed transactions with a higher-fee copy (RBF). The server
 * builds it; this browser checks it against the original and signs it. */
export function SpeedUp({
  wallet,
  txid,
  onDone,
  onCancel,
}: {
  wallet: StoredWallet;
  txid: string;
  onDone: () => void;
  onCancel: () => void;
}) {
  const [rate, setRate] = useState("");
  const [prepared, setPrepared] = useState<{ psbt: string; review: BumpReview }>();
  const [password, setPassword] = useState("");
  const [allowHighFee, setAllowHighFee] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const customRate = rate.trim() === "" ? undefined : Number(rate);
  const rateInvalid = customRate !== undefined && !(Number.isInteger(customRate) && customRate > 0);

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
      const built = await bumpFee(wallet, { txid, fee_rate_sat_vb: customRate });
      // Check the replacement against the original ourselves; don't trust the server.
      const review = await reviewBump(wallet, built.psbt, built.original_tx, txid);
      setAllowHighFee(false);
      setPrepared({ psbt: built.psbt, review });
    });

  const signAndSend = (psbt: string) =>
    run(async () => {
      const signed = await signPsbt(wallet, password, psbt);
      await broadcast(wallet, signed);
      setPassword("");
      onDone();
    });

  const box = "mt-2 rounded border border-zinc-200 p-3 dark:border-zinc-800";

  if (!prepared) {
    return (
      <div className={box}>
        <p className="mb-2 text-xs text-zinc-500">
          Pay a higher fee so miners pick it up sooner. The payment stays the same; the extra fee
          comes out of your change. Leave the rate empty for a sensible default.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <input
            className={`${input} w-40`}
            placeholder="sat/vB (optional)"
            inputMode="numeric"
            value={rate}
            onChange={(e) => setRate(e.target.value)}
          />
          <Button onClick={prepare} disabled={busy || rateInvalid}>
            {busy ? "Preparing…" : "Review"}
          </Button>
          <Button variant="secondary" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
        </div>
        {rateInvalid && <p className="mt-2 text-xs text-zinc-500">Use a whole number.</p>}
        {error && <p className="mt-2 text-xs text-red-700 dark:text-red-400">{error}</p>}
      </div>
    );
  }

  const { review } = prepared;
  const blocked = !!review.feeWarning && !allowHighFee;
  return (
    <div className={box}>
      <p className="mb-2 text-xs text-green-700 dark:text-green-400">
        Verified in this browser: same payments as the original, only the fee and change differ.
      </p>
      <dl className="mb-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs tabular-nums">
        <dt className="text-zinc-500">Payments</dt>
        <dd>{formatBtc(review.paidSat)} BTC (unchanged)</dd>
        <dt className="text-zinc-500">Old fee</dt>
        <dd>
          {formatBtc(review.oldFeeSat)} BTC (~{review.oldFeeRate.toFixed(1)} sat/vB)
        </dd>
        <dt className="text-zinc-500">New fee</dt>
        <dd className="font-medium">
          {formatBtc(review.feeSat)} BTC (~{review.feeRate.toFixed(1)} sat/vB)
        </dd>
        <dt className="text-zinc-500">Change</dt>
        <dd>{formatBtc(review.changeSat)} BTC</dd>
      </dl>
      {review.feeWarning && (
        <label className="mb-3 flex items-start gap-2 text-xs text-red-700 dark:text-red-400">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={allowHighFee}
            onChange={(e) => setAllowHighFee(e.target.checked)}
          />
          <span>The fee looks like a mistake: {review.feeWarning}. Tick to send anyway.</span>
        </label>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="password"
          className={`${input} w-48`}
          placeholder="wallet password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <Button onClick={() => signAndSend(prepared.psbt)} disabled={busy || !password || blocked}>
          {busy ? "Signing…" : "Sign and replace"}
        </Button>
        <Button variant="secondary" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
      </div>
      {error && <p className="mt-2 text-xs text-red-700 dark:text-red-400">{error}</p>}
    </div>
  );
}
