"use client";

import { useState } from "react";

import { setUpAccount } from "@/lib/accounts";
import { saveAccount, type StoredWallet } from "@/lib/store";
import { loadWallet } from "@/lib/wallet";
import { errorMessage } from "@/lib/errors";

import { Logo } from "./logo";

const MIN_PASSWORD_LEN = 8; // same as wallet-core's crypto::MIN_PASSWORD_LEN

type Intent = "create" | "restore";

type Step =
  | { kind: "welcome" }
  | { kind: "password"; intent: Intent }
  | { kind: "phrase"; intent: Intent; password: string; words?: string };

/** First visit: welcome, then a password, then the recovery phrase (shown when creating,
 * typed when restoring). Then the wallet is created and registered. */
export function WalletSetup({
  network,
  onReady,
}: {
  network: string;
  onReady: (wallet: StoredWallet) => void;
}) {
  const [step, setStep] = useState<Step>({ kind: "welcome" });
  // Kept here so Back from the phrase step returns to a filled-in password step.
  const [password, setPassword] = useState("");

  if (step.kind === "welcome") {
    return (
      <div className="flex flex-1 flex-col items-center justify-center px-4 py-16">
        <Logo className="h-16 w-16" />
        <h1 className="mt-6 text-4xl tracking-tight">Osok Wallet</h1>
        <div className="mt-8 flex w-full max-w-xs flex-col gap-3">
          <Button size="large" onClick={() => setStep({ kind: "password", intent: "create" })}>
            Create new wallet
          </Button>
          <Button
            size="large"
            variant="secondary"
            onClick={() => setStep({ kind: "password", intent: "restore" })}
          >
            I already have a wallet
          </Button>
        </div>
        <p className="mt-16 text-center text-xs text-zinc-500">
          Your keys stay in this browser · {network}
        </p>
      </div>
    );
  }

  if (step.kind === "password") {
    return (
      <SetupScreen>
        <PasswordStep
          initial={password}
          onBack={() => {
            setPassword("");
            setStep({ kind: "welcome" });
          }}
          onConfirm={async (chosen) => {
            setPassword(chosen);
            // A new wallet gets fresh words now; a restored one has the user type theirs.
            const words =
              step.intent === "create" ? (await loadWallet()).generateMnemonic() : undefined;
            setStep({ kind: "phrase", intent: step.intent, password: chosen, words });
          }}
        />
      </SetupScreen>
    );
  }

  return (
    <SetupScreen>
      <PhraseStep
        network={network}
        password={step.password}
        generatedWords={step.words}
        onBack={() => setStep({ kind: "password", intent: step.intent })}
        onReady={onReady}
      />
    </SetupScreen>
  );
}

/** Centred column with the logo above a setup card. */
function SetupScreen({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center px-4 py-12">
      <Logo className="mb-6 h-12 w-12" />
      {children}
    </div>
  );
}

function SetupCard({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <section className="w-full max-w-md rounded-xl border border-zinc-800 bg-zinc-800/40 p-6">
      <h1 className="text-2xl font-bold tracking-tight">{title}</h1>
      <p className="mt-1 mb-6 text-zinc-400">{subtitle}</p>
      {children}
    </section>
  );
}

const field =
  "h-11 w-full rounded-lg border bg-zinc-950/60 px-3 text-sm placeholder:text-zinc-500 focus:outline-none";

/** A password input that shows its error only once the user has been in and out of it. */
function PasswordField({
  placeholder,
  value,
  onChange,
  error,
}: {
  placeholder: string;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}) {
  const [touched, setTouched] = useState(false);
  const shown = touched ? error : undefined;
  return (
    <div>
      <input
        type="password"
        autoComplete="new-password"
        placeholder={placeholder}
        className={`${field} ${shown ? "border-red-700" : "border-zinc-800 focus:border-zinc-600"}`}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={() => setTouched(true)}
      />
      {shown && <p className="mt-1.5 text-xs text-red-500">{shown}</p>}
    </div>
  );
}

function PasswordStep({
  initial,
  onBack,
  onConfirm,
}: {
  initial: string;
  onBack: () => void;
  onConfirm: (password: string) => Promise<void>;
}) {
  const [password, setPassword] = useState(initial);
  const [repeat, setRepeat] = useState(initial);
  const [agreed, setAgreed] = useState(initial !== "");
  const [busy, setBusy] = useState(false);

  const passwordError =
    (password === "" && "Password is required") ||
    (password.length < MIN_PASSWORD_LEN &&
      `Password must be at least ${MIN_PASSWORD_LEN} characters`) ||
    undefined;
  const repeatError =
    (repeat === "" && "Please confirm your password") ||
    (repeat !== password && "Passwords don't match") ||
    undefined;
  const valid = !passwordError && !repeatError && agreed;

  async function confirm() {
    setBusy(true);
    try {
      await onConfirm(password);
    } finally {
      setBusy(false);
    }
  }

  return (
    <SetupCard title="Create a Password" subtitle="This will be used to unlock your wallet">
      <div className="flex flex-col gap-4">
        <PasswordField
          placeholder="New password"
          value={password}
          onChange={setPassword}
          error={passwordError}
        />
        <PasswordField
          placeholder="Confirm password"
          value={repeat}
          onChange={setRepeat}
          error={repeatError}
        />
      </div>
      <label className="mt-6 flex items-center gap-3 text-sm text-zinc-400">
        <input
          type="checkbox"
          className="h-4 w-4 accent-violet-500"
          checked={agreed}
          onChange={(e) => setAgreed(e.target.checked)}
        />
        <span>
          I have read and agree to{" "}
          <a
            href="/terms"
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 font-medium text-zinc-100 hover:underline"
          >
            Terms of Use <ExternalIcon />
          </a>
        </span>
      </label>
      <StepButtons
        onBack={onBack}
        onNext={confirm}
        nextLabel={busy ? "One moment…" : "Confirm"}
        nextDisabled={!valid || busy}
        backDisabled={busy}
      />
    </SetupCard>
  );
}

function PhraseStep({
  network,
  password,
  generatedWords,
  onBack,
  onReady,
}: {
  network: string;
  password: string;
  /** Set when creating: the new phrase to show. Unset when restoring: the user types it. */
  generatedWords?: string;
  onBack: () => void;
  onReady: (wallet: StoredWallet) => void;
}) {
  const creating = generatedWords !== undefined;
  // When creating: explain the phrase, show it, then have the user confirm it.
  const [stage, setStage] = useState<"intro" | "show" | "confirm">("intro");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function submit(words: string) {
    setBusy(true);
    setError(undefined);
    try {
      // The first account, m/84'/1'/0'. More can be added later from the dashboard.
      const wallet = await setUpAccount(words, password, network, 0);
      await saveAccount(wallet);
      onReady(wallet);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  if (creating && stage === "intro") {
    return <PhraseIntro onShow={() => setStage("show")} />;
  }

  if (creating && stage === "confirm") {
    return (
      <ConfirmPhrase
        words={generatedWords.split(" ")}
        busy={busy}
        error={error}
        onConfirmed={() => submit(generatedWords)}
        onShowAgain={() => {
          setError(undefined);
          setStage("show");
        }}
      />
    );
  }

  return creating ? (
    <SetupCard
      title="Your Recovery Phrase"
      subtitle="Write these 12 words down, in order, on paper"
    >
      <ol className="grid grid-cols-3 gap-2 font-mono text-sm">
        {generatedWords.split(" ").map((word, i) => (
          <li key={i} className="rounded-lg border border-zinc-800 bg-zinc-950/60 px-2 py-2">
            <span className="text-zinc-500">{i + 1}.</span> {word}
          </li>
        ))}
      </ol>
      <p className="mt-4 text-sm text-zinc-400">
        They are the <strong className="text-zinc-100">only</strong> way to recover your coins.
        Anyone who sees them can take your coins, and this browser won&apos;t keep them.
      </p>
      <button
        className="mt-8 h-11 w-full rounded-full bg-zinc-100 text-sm font-semibold text-zinc-900 transition-colors hover:bg-white"
        onClick={() => setStage("confirm")}
      >
        I’ve saved my phrase somewhere safe
      </button>
    </SetupCard>
  ) : (
    <ImportPhrase busy={busy} error={error} onImport={submit} onBack={onBack} />
  );
}

/** "I already have a wallet" (and "Add account"): one box per word (12 or 24), hidden unless
 * shown. Pasting the whole phrase into a box fills them all. `extra` goes under the boxes,
 * and `extraReady` must be true too before the action is enabled. */
export function ImportPhrase({
  busy,
  error,
  onImport,
  onBack,
  title = "Import wallet from recovery phrase",
  subtitle = "Enter your mnemonic phrase to restore your wallet. You can also paste the phrase in the first box.",
  actionLabel = "Import",
  busyLabel = "Importing…",
  extra,
  extraReady = true,
}: {
  busy: boolean;
  error?: string;
  onImport: (words: string) => void;
  onBack: () => void;
  title?: string;
  subtitle?: string;
  actionLabel?: string;
  busyLabel?: string;
  extra?: React.ReactNode;
  extraReady?: boolean;
}) {
  const [count, setCount] = useState<12 | 24>(12);
  const [words, setWords] = useState<string[]>(() => Array(24).fill(""));
  const [visible, setVisible] = useState(false);

  const shown = words.slice(0, count);
  const complete = shown.every((w) => w !== "");

  function setWord(index: number, value: string) {
    setWords((ws) => ws.map((w, i) => (i === index ? value.trim().toLowerCase() : w)));
  }

  function paste(e: React.ClipboardEvent<HTMLInputElement>) {
    const pasted = e.clipboardData.getData("text").trim().toLowerCase().split(/\s+/);
    if (pasted.length < 2) return; // a single word: let the box take it normally
    e.preventDefault();
    if (pasted.length > 12) setCount(24);
    setWords(Array.from({ length: 24 }, (_, i) => pasted[i] ?? ""));
  }

  return (
    <SetupCard title={title} subtitle={subtitle}>
      <div className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-xl border border-zinc-800 bg-zinc-950/60 p-5">
        {shown.map((word, i) => (
          <input
            key={i}
            type={visible ? "text" : "password"}
            aria-label={`Word ${i + 1}`}
            placeholder={String(i + 1).padStart(2, "0")}
            className="h-9 rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm placeholder:text-zinc-500 focus:border-zinc-600 focus:outline-none"
            value={word}
            onChange={(e) => setWord(i, e.target.value)}
            onPaste={paste}
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={busy}
          />
        ))}
      </div>
      <div className="mt-3 flex items-center justify-between">
        <div className="flex items-center gap-3 text-sm">
          <span className={count === 12 ? "text-zinc-100" : "text-zinc-500"}>12 word</span>
          <button
            role="switch"
            aria-checked={count === 24}
            aria-label="Use a 24-word phrase"
            onClick={() => setCount((c) => (c === 12 ? 24 : 12))}
            className={`relative h-6 w-11 rounded-full transition-colors ${
              count === 24 ? "bg-violet-500" : "bg-zinc-800"
            }`}
          >
            <span
              className={`absolute top-1 left-1 h-4 w-4 rounded-full bg-zinc-950 transition-transform ${
                count === 24 ? "translate-x-5" : ""
              }`}
            />
          </button>
          <span className={count === 24 ? "text-zinc-100" : "text-zinc-500"}>24 word</span>
        </div>
        <button
          className="flex items-center gap-1.5 rounded-md border border-zinc-800 bg-zinc-950 px-2.5 py-1 text-xs font-semibold"
          onClick={() => setVisible((v) => !v)}
        >
          {visible ? "Hide" : "Show"} <EyeIcon />
        </button>
      </div>
      {extra}
      {error && <p className="mt-4 text-sm text-red-500">{error}</p>}
      <button
        className="mt-8 h-11 w-full rounded-xl text-sm font-semibold transition-colors enabled:bg-zinc-100 enabled:text-zinc-900 enabled:hover:bg-white disabled:border disabled:border-zinc-800 disabled:text-zinc-500"
        onClick={() => onImport(shown.join(" "))}
        disabled={!complete || !extraReady || busy}
      >
        {busy ? busyLabel : actionLabel}
      </button>
      <button
        className="mt-3 w-full text-center text-sm text-zinc-400 hover:text-zinc-100 disabled:opacity-40"
        onClick={onBack}
        disabled={busy}
      >
        Back
      </button>
    </SetupCard>
  );
}

function EyeIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden
    >
      <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8Z" />
      <circle cx="8" cy="8" r="2" />
    </svg>
  );
}

/** Fisher–Yates shuffle; positions, not words, so a phrase with a repeated word still works. */
function shuffledPositions(count: number): number[] {
  const order = Array.from({ length: count }, (_, i) => i);
  for (let i = count - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [order[i], order[j]] = [order[j], order[i]];
  }
  return order;
}

/** The user taps the shuffled words in their original order, proving they wrote the phrase
 * down before the wallet is created. There's deliberately no way to skip this. */
function ConfirmPhrase({
  words,
  busy,
  error,
  onConfirmed,
  onShowAgain,
}: {
  words: string[];
  busy: boolean;
  error?: string;
  onConfirmed: () => void;
  onShowAgain: () => void;
}) {
  const [order] = useState(() => shuffledPositions(words.length));
  // Positions in `words` of the chips tapped so far, in tap order.
  const [picked, setPicked] = useState<number[]>([]);
  const [wrong, setWrong] = useState(false);

  function toggle(position: number) {
    setWrong(false);
    setPicked((p) => (p.includes(position) ? p.filter((x) => x !== position) : [...p, position]));
  }

  function confirm() {
    // Compare words, not positions: if a word appears twice, either chip is right.
    const correct = picked.every((position, i) => words[position] === words[i]);
    if (correct) {
      onConfirmed();
    } else {
      setWrong(true);
    }
  }

  return (
    <SetupCard
      title="Confirm Your Recovery Phrase"
      subtitle="Please select each word in the same order you have them noted to confirm you got them right."
    >
      <div className="grid grid-cols-2 gap-2 rounded-xl border border-zinc-800 bg-zinc-950/60 p-4">
        {order.map((position) => {
          const number = picked.indexOf(position) + 1;
          return (
            <button
              key={position}
              onClick={() => toggle(position)}
              disabled={busy}
              className={`relative h-9 rounded-lg border text-sm font-semibold transition-colors ${
                number
                  ? "border-violet-500 bg-violet-500/15 text-violet-100"
                  : "border-zinc-800 bg-zinc-900 hover:border-zinc-600"
              }`}
            >
              {number > 0 && (
                <span className="absolute top-1/2 left-2 -translate-y-1/2 text-xs text-violet-300">
                  {number}
                </span>
              )}
              {words[position]}
            </button>
          );
        })}
      </div>
      {wrong && (
        <p className="mt-4 text-sm text-red-500">
          That&apos;s not the right order. Tap a word again to unselect it, and check against your
          written phrase.
        </p>
      )}
      {error && <p className="mt-4 text-sm text-red-500">{error}</p>}
      <button
        className="mt-6 h-11 w-full rounded-xl text-sm font-semibold transition-colors enabled:bg-zinc-100 enabled:text-zinc-900 enabled:hover:bg-white disabled:border disabled:border-zinc-800 disabled:text-zinc-500"
        onClick={confirm}
        disabled={picked.length !== words.length || busy}
      >
        {busy ? "Creating…" : "Confirm"}
      </button>
      <p className="mt-4 text-center text-sm text-zinc-400">
        Osok never stores your phrase, so this is your last chance to see it.{" "}
        <button className="font-medium text-zinc-100 hover:underline" onClick={onShowAgain}>
          Show my phrase again
        </button>
      </p>
    </SetupCard>
  );
}

/** Before the 12 words appear: what they are and how to keep them safe. */
function PhraseIntro({ onShow }: { onShow: () => void }) {
  const points: [React.ReactNode, string][] = [
    [
      <LockIcon key="lock" />,
      "Your recovery phrase gives you full access to your wallet and funds",
    ],
    [
      <PasswordIcon key="password" />,
      "If you forget your password, you can use the recovery phrase to access your wallet",
    ],
    [<HiddenIcon key="hidden" />, "NEVER share this phrase with anyone"],
    [<InfoIcon key="info" />, "No one from Osok will ever ask for your recovery phrase"],
  ];
  return (
    <section className="w-full max-w-md rounded-xl border border-zinc-800 bg-zinc-800/40 p-6">
      <h1 className="text-2xl font-bold tracking-tight">Recovery Phrase</h1>
      <p className="mt-3 text-zinc-400">
        Your recovery phrase gives you access to your wallet and is the only way to access it in a
        new browser. <span className="text-zinc-100">Keep it in a safe place.</span>
      </p>
      <ul className="mt-6 flex flex-col gap-4">
        {points.map(([icon, text]) => (
          <li key={text} className="flex items-start gap-3">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-zinc-950/70">
              {icon}
            </span>
            <span className="pt-1 text-sm leading-6">{text}</span>
          </li>
        ))}
      </ul>
      <button
        className="mt-8 h-11 w-full rounded-xl bg-zinc-100 text-sm font-semibold text-zinc-900 transition-colors hover:bg-white"
        onClick={onShow}
      >
        Show recovery phrase
      </button>
    </section>
  );
}

const icon = "h-4 w-4";

function LockIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className={`${icon} text-lime-400`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden
    >
      <rect x="3" y="7" width="10" height="7" rx="1.5" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </svg>
  );
}

function PasswordIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className={`${icon} text-amber-400`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden
    >
      <rect x="1.5" y="4.5" width="13" height="7" rx="1.5" />
      <path d="M5 8h.01M8 8h.01M11 8h.01" strokeLinecap="round" strokeWidth="2" />
    </svg>
  );
}

function HiddenIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className={`${icon} text-pink-500`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden
    >
      <path d="M2 8s2.2-4 6-4 6 4 6 4-2.2 4-6 4-6-4-6-4Z" />
      <circle cx="8" cy="8" r="1.8" />
      <path d="M2.5 2.5l11 11" strokeLinecap="round" />
    </svg>
  );
}

function InfoIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className={`${icon} text-sky-500`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden
    >
      <circle cx="8" cy="8" r="6" />
      <path d="M8 4.8v3.7M8 11h.01" strokeLinecap="round" />
    </svg>
  );
}

/** Back and the step's main action, side by side. */
function StepButtons({
  onBack,
  onNext,
  nextLabel,
  nextDisabled,
  backDisabled,
}: {
  onBack: () => void;
  onNext: () => void;
  nextLabel: string;
  nextDisabled: boolean;
  backDisabled: boolean;
}) {
  return (
    <div className="mt-8 grid grid-cols-2 gap-3">
      <button
        className="h-11 rounded-xl border border-zinc-800 bg-zinc-950 text-sm font-semibold disabled:opacity-40"
        onClick={onBack}
        disabled={backDisabled}
      >
        Back
      </button>
      <button
        className="h-11 rounded-xl text-sm font-semibold transition-colors enabled:bg-zinc-100 enabled:text-zinc-900 enabled:hover:bg-white disabled:border disabled:border-zinc-800 disabled:text-zinc-500"
        onClick={onNext}
        disabled={nextDisabled}
      >
        {nextLabel}
      </button>
    </div>
  );
}

function ExternalIcon() {
  return (
    <svg viewBox="0 0 16 16" className="h-3.5 w-3.5" fill="none" stroke="currentColor" aria-hidden>
      <path d="M9 2h5v5M14 2 7.5 8.5M12 9.5V13a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3.5" />
    </svg>
  );
}

export function Card({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <section className="w-full rounded-2xl border border-zinc-800 bg-zinc-900/40 p-5">
      {title && <h2 className="mb-3 font-medium">{title}</h2>}
      {children}
    </section>
  );
}

/** Pill buttons: light "primary", outlined "secondary". `large` is the full-width welcome size. */
export function Button({
  variant = "primary",
  size = "normal",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary";
  size?: "normal" | "large";
}) {
  const style =
    variant === "primary"
      ? "bg-zinc-100 text-zinc-900 hover:bg-white"
      : "border border-zinc-700 text-zinc-100 hover:border-zinc-500";
  const sizing = size === "large" ? "h-10 w-full" : "px-4 py-1.5";
  return (
    <button
      className={`rounded-full text-sm font-semibold transition-colors disabled:opacity-40 ${sizing} ${style}`}
      {...props}
    />
  );
}
