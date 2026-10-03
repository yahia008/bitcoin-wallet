"use client";

import { useEffect, useRef, useState } from "react";

import { broadcast, buildPsbt, type FeePriority } from "@/lib/api";
import { explorerTxUrl, formatBtc, formatBtcShort, parseBtc } from "@/lib/format";
import type { StoredWallet } from "@/lib/store";
import { checkAddress, reviewSend, signPsbt, type SendReview } from "@/lib/wallet";

import {
  ArrowDownIcon,
  ArrowUpIcon,
  BackIcon,
  CheckIcon,
  ChevronRightIcon,
  CloseIcon,
  DoubleChevronDownIcon,
  FeeIcon,
  GearIcon,
  ListIcon,
  UserIcon,
} from "./icons";
import { Logo } from "./logo";
import { PasswordCard } from "./password-card";

/** What the user chose to send: an exact amount, or everything ("Max"). */
type Amount = { kind: "exact"; sats: number } | { kind: "all" };

type Step =
  | { kind: "address" }
  | { kind: "amount" }
  | { kind: "review"; psbt: string; amountSat: number; review: SendReview }
  | { kind: "sent"; txid: string; amountSat: number };

const PRIORITIES: { value: FeePriority; label: string; hint: string }[] = [
  { value: "slow", label: "Slow", hint: "~1 day, cheapest" },
  { value: "normal", label: "Normal", hint: "~1 hour" },
  { value: "fast", label: "Fast", hint: "~20 min" },
];

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Pay someone, in steps: recipient address (checked in the browser as you type), amount and
 * fee speed, then the review: the server builds an unsigned PSBT, this browser checks it with
 * the CLI's review code and signs it once the user enters their password. */
export function Send({
  wallet,
  available,
  onClose,
  onSent,
}: {
  wallet: StoredWallet;
  /** Spendable balance in sats, if known. */
  available?: number;
  onClose: () => void;
  onSent: () => void;
}) {
  const [step, setStep] = useState<Step>({ kind: "address" });
  const [to, setTo] = useState("");
  const [amountText, setAmountText] = useState("");
  const [sendAll, setSendAll] = useState(false);
  const [priority, setPriority] = useState<FeePriority>("normal");

  const address = to.trim();

  if (step.kind === "address") {
    return (
      <AddressStep
        network={wallet.network}
        value={to}
        onChange={setTo}
        onClose={onClose}
        onContinue={() => setStep({ kind: "amount" })}
      />
    );
  }

  if (step.kind === "amount") {
    return (
      <AmountStep
        wallet={wallet}
        address={address}
        available={available}
        amountText={amountText}
        sendAll={sendAll}
        priority={priority}
        onAmount={(text, all) => {
          setAmountText(text);
          setSendAll(all);
        }}
        onPriority={setPriority}
        onEditAddress={() => setStep({ kind: "address" })}
        onClose={onClose}
        onReviewed={(psbt, amountSat, review) =>
          setStep({ kind: "review", psbt, amountSat, review })
        }
      />
    );
  }

  if (step.kind === "review") {
    return (
      <ReviewStep
        wallet={wallet}
        address={address}
        step={step}
        onBack={() => setStep({ kind: "amount" })}
        onSent={(txid) => {
          setStep({ kind: "sent", txid, amountSat: step.amountSat });
          onSent();
        }}
      />
    );
  }

  const url = explorerTxUrl(wallet.network, step.txid);
  return (
    <div className="flex min-h-[560px] flex-1 flex-col">
      <Header title="Send" onClose={onClose} />
      <div className="flex flex-1 flex-col items-center justify-center text-center">
        <span className="flex h-16 w-16 items-center justify-center rounded-full bg-lime-500/15 text-lime-300">
          <CheckIcon className="h-8 w-8" />
        </span>
        <p className="mt-5 text-2xl font-semibold">{formatBtcShort(step.amountSat)} BTC sent</p>
        <p className="mt-2 text-sm text-zinc-400">
          To {short(address)}. It confirms once a miner includes it in a block.
        </p>
        <p className="mt-3 font-mono text-xs text-zinc-500">
          {url ? (
            <a href={url} target="_blank" rel="noreferrer" className="underline">
              {short(step.txid)}
            </a>
          ) : (
            short(step.txid)
          )}
        </p>
      </div>
      <PrimaryButton onClick={onClose}>Done</PrimaryButton>
    </div>
  );
}

const short = (s: string) => `${s.slice(0, 6)}…${s.slice(-6)}`;

function Header({
  title,
  onClose,
  onBack,
}: {
  title: string;
  onClose?: () => void;
  onBack?: () => void;
}) {
  return (
    <div className="grid grid-cols-[2.5rem_1fr_2.5rem] items-center">
      <button
        aria-label={onBack ? "Back" : "Close"}
        onClick={onBack ?? onClose}
        className="flex h-10 w-10 items-center justify-center rounded-full text-zinc-300 hover:bg-zinc-800/60"
      >
        {onBack ? <BackIcon /> : <CloseIcon />}
      </button>
      <h1 className="text-center text-lg font-semibold">{title}</h1>
    </div>
  );
}

function PrimaryButton({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="h-12 w-full rounded-full bg-zinc-100 text-sm font-semibold text-zinc-900 transition-colors hover:bg-white disabled:bg-zinc-800 disabled:text-zinc-500"
    >
      {children}
    </button>
  );
}

/** Step 1: the recipient. Checked against the wallet's network on every keystroke; Continue
 * appears once it's valid. */
function AddressStep({
  network,
  value,
  onChange,
  onClose,
  onContinue,
}: {
  network: string;
  value: string;
  onChange: (value: string) => void;
  onClose: () => void;
  onContinue: () => void;
}) {
  const [check, setCheck] = useState<{ for: string; error?: string }>();
  const address = value.trim();

  useEffect(() => {
    if (!address) return;
    let current = true;
    checkAddress(address, network).then(
      () => current && setCheck({ for: address }),
      (e) => current && setCheck({ for: address, error: message(e) }),
    );
    return () => {
      current = false;
    };
  }, [address, network]);

  // Only trust a result for exactly what's in the box now.
  const result = address && check?.for === address ? check : undefined;
  const valid = result !== undefined && result.error === undefined;

  return (
    <div className="flex min-h-[560px] flex-1 flex-col">
      <Header title="Send" onClose={onClose} />
      <label
        className={`mt-6 flex h-12 items-center gap-3 rounded-xl border bg-zinc-950/60 px-4 ${
          result?.error ? "border-red-700" : valid ? "border-lime-500/50" : "border-zinc-800"
        } focus-within:border-zinc-600`}
      >
        <span className="text-zinc-500">
          <UserIcon />
        </span>
        <input
          autoFocus
          placeholder="Enter address"
          aria-label="Recipient address"
          className="flex-1 bg-transparent font-mono text-sm placeholder:font-sans placeholder:text-zinc-500 focus:outline-none"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && valid && onContinue()}
          autoComplete="off"
          spellCheck={false}
        />
        {valid && (
          <span className="text-lime-300">
            <CheckIcon />
          </span>
        )}
      </label>
      {result?.error && <p className="mt-2 text-sm text-red-500">{result.error}</p>}
      {valid && <p className="mt-2 text-sm text-zinc-400">Valid {network} address.</p>}
      <div className="flex-1" />
      {valid && <PrimaryButton onClick={onContinue}>Continue</PrimaryButton>}
    </div>
  );
}

/** Step 2: how much, and how fast. Percentages are of the spendable balance; Max sends
 * everything minus the fee, which only the server's builder can work out exactly. */
function AmountStep({
  wallet,
  address,
  available,
  amountText,
  sendAll,
  priority,
  onAmount,
  onPriority,
  onEditAddress,
  onClose,
  onReviewed,
}: {
  wallet: StoredWallet;
  address: string;
  available?: number;
  amountText: string;
  sendAll: boolean;
  priority: FeePriority;
  onAmount: (text: string, all: boolean) => void;
  onPriority: (p: FeePriority) => void;
  onEditAddress: () => void;
  onClose: () => void;
  onReviewed: (psbt: string, amountSat: number, review: SendReview) => void;
}) {
  const [showFees, setShowFees] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const sats = parseBtc(amountText);
  const amount: Amount | undefined = sendAll
    ? { kind: "all" }
    : sats !== undefined
      ? { kind: "exact", sats }
      : undefined;
  const tooMuch = amount?.kind === "exact" && available !== undefined && amount.sats > available;
  const problem =
    (amountText.trim() !== "" && !sendAll && sats === undefined && "Enter an amount in BTC") ||
    (tooMuch && "That's more than you have") ||
    (available === 0 && "Nothing to send yet") ||
    undefined;

  function setFraction(fraction: number) {
    if (!available) return;
    if (fraction === 1) {
      onAmount(formatBtcShort(available), true);
    } else {
      onAmount(formatBtcShort(Math.floor(available * fraction)), false);
    }
  }

  async function review() {
    if (!amount) return;
    setBusy(true);
    setError(undefined);
    try {
      const built = await buildPsbt(wallet, {
        address,
        amount_sat: amount.kind === "exact" ? amount.sats : 0,
        fee_priority: priority,
        send_all: amount.kind === "all",
      });
      // For Max, the amount is what the server's PSBT pays; the review below still checks
      // that every other output is ours, and the fee guard checks the fee.
      const amountSat = amount.kind === "exact" ? amount.sats : built.amount_sat;
      const checked = await reviewSend(wallet, built.psbt, address, amountSat);
      onReviewed(built.psbt, amountSat, checked);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const speed = PRIORITIES.find((p) => p.value === priority)!;
  return (
    <div className="flex min-h-[560px] flex-1 flex-col">
      <Header title="Send" onClose={onClose} />

      <button
        onClick={onEditAddress}
        className="mt-6 flex items-center gap-3 rounded-2xl bg-zinc-800/50 px-4 py-3 text-left hover:bg-zinc-800"
      >
        <span className="flex h-9 w-9 items-center justify-center rounded-full bg-violet-500/15 text-violet-300">
          <ArrowUpIcon />
        </span>
        <span className="flex-1 font-mono text-sm">{short(address)}</span>
        <span className="flex h-8 w-8 items-center justify-center rounded-full bg-zinc-950/70 text-zinc-300">
          <ChevronRightIcon />
        </span>
      </button>

      <div className="mt-2 rounded-2xl bg-zinc-800/50 px-4 py-4">
        <div className="flex justify-between text-sm text-zinc-400">
          <span>Sending</span>
          <span>
            {available === undefined ? "…" : `${formatBtcShort(available)} BTC available`}
          </span>
        </div>
        <div className="mt-2 flex items-center gap-3">
          <input
            autoFocus
            inputMode="decimal"
            placeholder="0"
            aria-label="Amount in BTC"
            className="min-w-0 flex-1 bg-transparent text-3xl font-semibold tabular-nums placeholder:text-zinc-600 focus:outline-none"
            value={amountText}
            onChange={(e) => onAmount(e.target.value, false)}
          />
          <span className="flex items-center gap-2 rounded-full bg-zinc-950/70 px-3 py-1.5 text-sm font-semibold">
            <Logo className="h-4 w-4" /> BTC
          </span>
        </div>
        <p className="mt-1 text-sm text-zinc-500 tabular-nums">
          {sendAll
            ? "Everything, minus the network fee"
            : sats !== undefined
              ? `≈ ${sats.toLocaleString()} sats`
              : " "}
        </p>
      </div>

      <div className="mt-3 grid grid-cols-4 gap-2">
        {[
          ["25%", 0.25],
          ["50%", 0.5],
          ["75%", 0.75],
          ["Max", 1],
        ].map(([label, fraction]) => (
          <button
            key={label}
            onClick={() => setFraction(fraction as number)}
            disabled={!available}
            className={`h-10 rounded-full border text-sm font-semibold transition-colors disabled:opacity-40 ${
              label === "Max" && sendAll
                ? "border-violet-500 bg-violet-500/15 text-violet-200"
                : "border-zinc-800 hover:border-zinc-600"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {(problem || error) && <p className="mt-3 text-sm text-red-500">{problem ?? error}</p>}

      <div className="flex-1" />

      {showFees && (
        <div className="mb-3 grid grid-cols-3 gap-2">
          {PRIORITIES.map((p) => (
            <button
              key={p.value}
              onClick={() => {
                onPriority(p.value);
                setShowFees(false);
              }}
              className={`rounded-xl border px-2 py-2 text-left text-sm ${
                p.value === priority
                  ? "border-violet-500 bg-violet-500/15"
                  : "border-zinc-800 hover:border-zinc-600"
              }`}
            >
              <span className="block font-semibold">{p.label}</span>
              <span className="block text-xs text-zinc-400">{p.hint}</span>
            </button>
          ))}
        </div>
      )}
      <div className="mb-3 flex items-center justify-between text-sm">
        <span className="text-zinc-400">
          Fee: <span className="text-zinc-100">{speed.label}</span> ({speed.hint})
        </span>
        <button
          aria-label="Fee speed"
          title="Fee speed"
          onClick={() => setShowFees((v) => !v)}
          className="flex h-9 w-9 items-center justify-center rounded-full border border-zinc-800 text-zinc-300 hover:border-zinc-600"
        >
          <GearIcon />
        </button>
      </div>
      <PrimaryButton onClick={review} disabled={!amount || !!problem || busy}>
        {busy ? "Preparing…" : "Review Send"}
      </PrimaryButton>
    </div>
  );
}

/** Step 3: a bottom sheet over the dimmed send screen: what the PSBT really does (already
 * verified in this browser), then the password, then sign. */
function ReviewStep({
  wallet,
  address,
  step,
  onBack,
  onSent,
}: {
  wallet: StoredWallet;
  address: string;
  step: Extract<Step, { kind: "review" }>;
  onBack: () => void;
  onSent: (txid: string) => void;
}) {
  const [asking, setAsking] = useState(false); // false: summary; true: password
  const [details, setDetails] = useState(false);
  const detailsRef = useRef<HTMLDListElement>(null);
  const [allowHighFee, setAllowHighFee] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const { review } = step;
  const blocked = !!review.feeWarning && !allowHighFee;

  // Opening the details scrolls them into view, so it's clear more appeared below.
  useEffect(() => {
    if (details) detailsRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, [details]);

  async function signAndSend(password: string) {
    setBusy(true);
    setError(undefined);
    try {
      const signed = await signPsbt(wallet, password, step.psbt);
      const { txid } = await broadcast(wallet, signed);
      onSent(txid);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const row = "flex items-center justify-between gap-4 py-3";
  return (
    <div className="relative flex min-h-[560px] flex-1 flex-col">
      {/* The send screen behind the sheet, dimmed. */}
      <div className="opacity-40">
        <Header title="Send" onClose={onBack} />
      </div>
      <div
        className={
          asking
            ? // The password: a compact card floating over the dimmed screen.
              "absolute inset-0 flex items-start justify-center pt-20"
            : // The summary: a sheet up from the bottom of the panel.
              "absolute inset-x-0 bottom-0 top-12 -mx-4 -mb-6 flex flex-col rounded-t-3xl border-t border-zinc-800 bg-zinc-900 px-5 pt-6 pb-6 shadow-2xl sm:-mx-6 sm:rounded-b-3xl"
        }
      >
        {asking ? (
          <PasswordCard
            subtitle="Enter your wallet password to sign this transaction."
            actionLabel="Sign and send"
            busyLabel="Signing…"
            busy={busy}
            error={error}
            onSubmit={signAndSend}
            onCancel={() => {
              setAsking(false);
              setError(undefined);
            }}
          />
        ) : (
          <>
            <div className="-mx-1 min-h-0 flex-1 overflow-y-auto px-1 pb-4">
              <p className="text-lg">You are sending</p>
              <div className="mt-4 flex items-center gap-3">
                <span className="flex h-11 w-11 items-center justify-center rounded-full bg-violet-500/15 text-violet-300">
                  <Logo className="h-6 w-6" />
                </span>
                <span>
                  <span className="block text-lg font-semibold tabular-nums">
                    {formatBtcShort(step.amountSat)} BTC
                  </span>
                  <span className="block text-sm text-zinc-400 tabular-nums">
                    ≈ {step.amountSat.toLocaleString()} sats
                  </span>
                </span>
              </div>
              <span className="my-2 ml-3 text-zinc-500">
                <DoubleChevronDownIcon />
              </span>
              <div className="flex items-center gap-3">
                <span className="flex h-11 w-11 items-center justify-center rounded-full border border-zinc-700 text-lime-300">
                  <ArrowUpIcon />
                </span>
                <span className="font-mono text-lg">{short(address)}</span>
              </div>

              <dl className="mt-6 divide-y divide-zinc-800 rounded-2xl bg-zinc-800/50 px-4 text-sm tabular-nums">
                <div className={row}>
                  <dt className="flex items-center gap-2 text-zinc-400">
                    <FeeIcon /> Network fee
                  </dt>
                  <dd className="text-right">
                    <span className="font-semibold">{formatBtcShort(review.feeSat)} BTC</span>
                    <span className="block text-xs text-zinc-500">
                      ~{review.feeRate.toFixed(1)} sat/vB
                    </span>
                  </dd>
                </div>
                <div className={row}>
                  <dt className="flex items-center gap-2 text-zinc-400">
                    <ArrowDownIcon className="h-4 w-4" /> Change back to you
                  </dt>
                  <dd className="font-semibold">{formatBtcShort(review.changeSat)} BTC</dd>
                </div>
              </dl>

              <button
                onClick={() => setDetails((d) => !d)}
                aria-expanded={details}
                className="mt-2 flex w-full items-center gap-2 rounded-2xl bg-zinc-800/50 px-4 py-3 text-left text-sm font-semibold text-violet-300 hover:bg-zinc-800"
              >
                <ListIcon /> Transaction details
              </button>
              {details && (
                <dl
                  ref={detailsRef}
                  className="mt-2 space-y-2 rounded-2xl bg-zinc-800/30 px-4 py-3 text-xs tabular-nums"
                >
                  <p className="flex items-center gap-2 text-lime-300">
                    <CheckIcon className="h-3.5 w-3.5" /> Verified in this browser: pays exactly
                    this, the rest comes back to you.
                  </p>
                  <div>
                    <dt className="text-zinc-500">To</dt>
                    <dd className="break-all font-mono">{address}</dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-zinc-500">Amount</dt>
                    <dd>{formatBtc(step.amountSat)} BTC</dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-zinc-500">Network fee</dt>
                    <dd>{formatBtc(review.feeSat)} BTC</dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-zinc-500">Change</dt>
                    <dd>{formatBtc(review.changeSat)} BTC</dd>
                  </div>
                  {review.changeAddresses.map((c) => (
                    <div key={c.index}>
                      <dt className="text-zinc-500">Change address #{c.index}</dt>
                      <dd className="break-all font-mono">{c.address}</dd>
                    </div>
                  ))}
                  <div className="flex justify-between">
                    <dt className="text-zinc-500">Total out</dt>
                    <dd className="font-semibold">
                      {formatBtc(step.amountSat + review.feeSat)} BTC
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-zinc-500">Inputs</dt>
                    <dd>{review.inputs}</dd>
                  </div>
                </dl>
              )}

              {review.feeWarning && (
                <label className="mt-3 flex items-start gap-2 text-sm text-red-400">
                  <input
                    type="checkbox"
                    className="mt-1 accent-violet-500"
                    checked={allowHighFee}
                    onChange={(e) => setAllowHighFee(e.target.checked)}
                  />
                  <span>
                    The fee looks like a mistake: {review.feeWarning}. Tick to send anyway.
                  </span>
                </label>
              )}
            </div>
            <PrimaryButton onClick={() => setAsking(true)} disabled={blocked}>
              Send to {short(address)}
            </PrimaryButton>
            <button
              onClick={onBack}
              className="mt-3 h-12 w-full rounded-full border border-zinc-800 text-sm font-semibold"
            >
              Cancel
            </button>
          </>
        )}
      </div>
    </div>
  );
}
