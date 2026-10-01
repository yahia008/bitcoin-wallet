"use client";

import { useState } from "react";

import { API_URL, ApiError, register } from "@/lib/api";
import { saveStoredWallet, type StoredWallet } from "@/lib/store";
import { createWallet, loadWallet } from "@/lib/wallet";

const MIN_PASSWORD_LEN = 8; // same as wallet-core's crypto::MIN_PASSWORD_LEN

type Mode = { kind: "choose" } | { kind: "create"; words: string } | { kind: "restore" };

/** Create a new wallet or restore one from its recovery phrase, then register it. */
export function WalletSetup({
  network,
  onReady,
}: {
  network: string;
  onReady: (wallet: StoredWallet) => void;
}) {
  const [mode, setMode] = useState<Mode>({ kind: "choose" });

  async function startCreate() {
    const wasm = await loadWallet();
    setMode({ kind: "create", words: wasm.generateMnemonic() });
  }

  if (mode.kind === "choose") {
    return (
      <Card title="Set up a wallet">
        <p className="mb-4 text-sm text-zinc-500">
          No wallet in this browser for <strong>{network}</strong> yet.
        </p>
        <div className="flex gap-2">
          <Button onClick={startCreate}>Create a new wallet</Button>
          <Button variant="secondary" onClick={() => setMode({ kind: "restore" })}>
            Restore from recovery phrase
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <SetupForm
      key={mode.kind}
      network={network}
      generatedWords={mode.kind === "create" ? mode.words : undefined}
      onBack={() => setMode({ kind: "choose" })}
      onReady={onReady}
    />
  );
}

function SetupForm({
  network,
  generatedWords,
  onBack,
  onReady,
}: {
  network: string;
  /** Set when creating: the new phrase to show. Unset when restoring: the user types it. */
  generatedWords?: string;
  onBack: () => void;
  onReady: (wallet: StoredWallet) => void;
}) {
  const creating = generatedWords !== undefined;
  const [typedWords, setTypedWords] = useState("");
  const [wroteDown, setWroteDown] = useState(false);
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const words = generatedWords ?? typedWords;
  const problem =
    (creating && !wroteDown && "Confirm you wrote the recovery phrase down.") ||
    (!creating && !typedWords.trim() && "Enter your recovery phrase.") ||
    (password.length < MIN_PASSWORD_LEN &&
      `Password must be at least ${MIN_PASSWORD_LEN} characters.`) ||
    (password !== repeat && "Passwords don't match.") ||
    undefined;

  async function submit() {
    setBusy(true);
    setError(undefined);
    try {
      // Derives the account key and encrypts it, all in this browser.
      const created = await createWallet(words, password, network);
      // Only the public descriptors go to the server.
      const registration = await register(created.external, created.internal);
      if (registration.id !== created.walletId) {
        throw new Error("the server computed a different wallet id; not saving");
      }
      const wallet: StoredWallet = {
        walletId: created.walletId,
        network: created.network,
        external: created.external,
        internal: created.internal,
        firstAddress: created.firstAddress,
        encryptedKey: created.encryptedKey,
        apiUrl: API_URL,
        apiToken: registration.token,
        createdAt: new Date().toISOString(),
      };
      await saveStoredWallet(wallet);
      onReady(wallet);
    } catch (e) {
      setError(explain(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title={creating ? "Create a new wallet" : "Restore a wallet"}>
      {creating ? (
        <>
          <p className="mb-2 text-sm">
            Write these 12 words down, in order, on paper. They are the <strong>only</strong> way
            to recover your coins. This browser will not keep them.
          </p>
          <ol className="mb-3 grid grid-cols-3 gap-2 font-mono text-sm">
            {generatedWords.split(" ").map((word, i) => (
              <li key={i} className="rounded border border-zinc-300 px-2 py-1 dark:border-zinc-700">
                <span className="text-zinc-400">{i + 1}.</span> {word}
              </li>
            ))}
          </ol>
          <label className="mb-4 flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={wroteDown}
              onChange={(e) => setWroteDown(e.target.checked)}
            />
            I&apos;ve written them down
          </label>
        </>
      ) : (
        <textarea
          className="mb-4 w-full rounded border border-zinc-300 bg-transparent p-2 font-mono text-sm dark:border-zinc-700"
          rows={3}
          placeholder="your 12 words, separated by spaces"
          value={typedWords}
          onChange={(e) => setTypedWords(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
      )}

      <p className="mb-2 text-sm text-zinc-500">
        Choose a password. It encrypts your key in this browser and is needed to send.
      </p>
      <div className="mb-4 flex flex-col gap-2">
        <input
          type="password"
          className="rounded border border-zinc-300 bg-transparent p-2 text-sm dark:border-zinc-700"
          placeholder="password"
          autoComplete="new-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <input
          type="password"
          className="rounded border border-zinc-300 bg-transparent p-2 text-sm dark:border-zinc-700"
          placeholder="repeat password"
          autoComplete="new-password"
          value={repeat}
          onChange={(e) => setRepeat(e.target.value)}
        />
      </div>

      {problem && <p className="mb-2 text-sm text-zinc-500">{problem}</p>}
      {error && <p className="mb-2 text-sm text-red-700 dark:text-red-400">{error}</p>}
      <div className="flex gap-2">
        <Button onClick={submit} disabled={busy || !!problem}>
          {busy ? "Setting up…" : creating ? "Create wallet" : "Restore wallet"}
        </Button>
        <Button variant="secondary" onClick={onBack} disabled={busy}>
          Back
        </Button>
      </div>
    </Card>
  );
}

function explain(e: unknown): string {
  if (e instanceof ApiError && e.status === 409) {
    // Tokens are shown once, at registration (see the API's register_wallet).
    return (
      "This wallet is already registered with this server, so it won't hand out its API " +
      "token again. Use a server with a fresh --data-dir for the web wallet."
    );
  }
  return e instanceof Error ? e.message : String(e);
}

export function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
      <h2 className="mb-3 font-medium">{title}</h2>
      {children}
    </section>
  );
}

export function Button({
  variant = "primary",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "secondary" }) {
  const style =
    variant === "primary"
      ? "bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900"
      : "border border-zinc-300 dark:border-zinc-700";
  return (
    <button className={`rounded px-3 py-1.5 text-sm disabled:opacity-40 ${style}`} {...props} />
  );
}
