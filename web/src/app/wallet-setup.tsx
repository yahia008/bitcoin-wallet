"use client";

import { useState } from "react";

import { API_URL, ApiError, register } from "@/lib/api";
import { saveStoredWallet, type StoredWallet } from "@/lib/store";
import { createWallet, loadWallet, recoverApiToken } from "@/lib/wallet";

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
  const [typedWords, setTypedWords] = useState("");
  const [wroteDown, setWroteDown] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const words = generatedWords ?? typedWords;
  const ready = creating ? wroteDown : typedWords.trim() !== "";

  async function submit() {
    setBusy(true);
    setError(undefined);
    try {
      // Derives the account key and encrypts it, all in this browser.
      const created = await createWallet(words, password, network);
      // Only the public descriptors go to the server.
      let apiToken: string;
      try {
        const registration = await register(created.external, created.internal);
        if (registration.id !== created.walletId) {
          throw new Error("the server computed a different wallet id; not saving");
        }
        apiToken = registration.token;
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 409)) throw e;
        // Already registered (e.g. restored after "Forget"): the server won't hand out the
        // old token, so prove we hold the key and get a new one.
        apiToken = await recoverApiToken(created, password);
      }
      const wallet: StoredWallet = {
        walletId: created.walletId,
        network: created.network,
        external: created.external,
        internal: created.internal,
        firstAddress: created.firstAddress,
        encryptedKey: created.encryptedKey,
        apiUrl: API_URL,
        apiToken,
        createdAt: new Date().toISOString(),
      };
      await saveStoredWallet(wallet);
      onReady(wallet);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
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
      <label className="mt-6 flex items-center gap-3 text-sm text-zinc-400">
        <input
          type="checkbox"
          className="h-4 w-4 accent-violet-500"
          checked={wroteDown}
          onChange={(e) => setWroteDown(e.target.checked)}
        />
        I&apos;ve written them down
      </label>
      {error && <p className="mt-4 text-sm text-red-500">{error}</p>}
      <StepButtons
        onBack={onBack}
        onNext={submit}
        nextLabel={busy ? "Creating…" : "Create wallet"}
        nextDisabled={!ready || busy}
        backDisabled={busy}
      />
    </SetupCard>
  ) : (
    <SetupCard title="Import Your Wallet" subtitle="Enter your 12-word recovery phrase">
      <textarea
        className="w-full rounded-lg border border-zinc-800 bg-zinc-950/60 p-3 font-mono text-sm placeholder:text-zinc-500 focus:border-zinc-600 focus:outline-none"
        rows={4}
        placeholder="word1 word2 word3 …"
        value={typedWords}
        onChange={(e) => setTypedWords(e.target.value)}
        autoComplete="off"
        spellCheck={false}
      />
      {error && <p className="mt-4 text-sm text-red-500">{error}</p>}
      <StepButtons
        onBack={onBack}
        onNext={submit}
        nextLabel={busy ? "Importing…" : "Import wallet"}
        nextDisabled={!ready || busy}
        backDisabled={busy}
      />
    </SetupCard>
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

export function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="w-full rounded-2xl border border-zinc-800 bg-zinc-900/40 p-5">
      <h2 className="mb-3 font-medium">{title}</h2>
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
