"use client";

import { useState } from "react";

import { EyeIcon, EyeOffIcon } from "./icons";
import { Logo } from "./logo";

/** The compact "Enter your password" card: icon, title, a password field with show/hide, the
 * action right below it, and a Cancel link. Used before signing a transaction and before
 * adding an account. */
export function PasswordCard({
  subtitle,
  actionLabel,
  busyLabel,
  busy,
  error,
  onSubmit,
  onCancel,
}: {
  subtitle: string;
  actionLabel: string;
  busyLabel: string;
  busy: boolean;
  error?: string;
  onSubmit: (password: string) => void;
  onCancel: () => void;
}) {
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const submit = () => password && !busy && onSubmit(password);

  return (
    <div className="w-full rounded-2xl border border-zinc-800 bg-zinc-900 p-6 shadow-2xl">
      <div className="flex flex-col items-center text-center">
        <span className="flex h-14 w-14 items-center justify-center rounded-full bg-violet-500/15 text-violet-300">
          <Logo className="h-7 w-7" />
        </span>
        <h2 className="mt-4 text-xl font-semibold">Enter your password</h2>
        <p className="mt-1 text-sm text-zinc-400">{subtitle}</p>
      </div>
      <label className="mt-5 flex h-12 items-center gap-2 rounded-xl border border-zinc-800 bg-zinc-950/60 px-4 focus-within:border-zinc-600">
        <input
          autoFocus
          type={show ? "text" : "password"}
          autoComplete="current-password"
          placeholder="Enter password"
          aria-label="Wallet password"
          className="flex-1 bg-transparent text-sm placeholder:text-zinc-500 focus:outline-none"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          disabled={busy}
        />
        <button
          type="button"
          aria-label={show ? "Hide password" : "Show password"}
          onClick={() => setShow((v) => !v)}
          className="text-zinc-400 hover:text-zinc-100"
        >
          {show ? <EyeIcon /> : <EyeOffIcon />}
        </button>
      </label>
      {error && <p className="mt-2 text-sm text-red-500">{error}</p>}
      <button
        onClick={submit}
        disabled={!password || busy}
        className="mt-3 h-12 w-full rounded-full bg-zinc-100 text-sm font-semibold text-zinc-900 transition-colors hover:bg-white disabled:bg-zinc-800 disabled:text-zinc-500"
      >
        {busy ? busyLabel : actionLabel}
      </button>
      <button
        onClick={onCancel}
        disabled={busy}
        className="mt-3 w-full text-center text-sm text-zinc-400 hover:text-zinc-100 disabled:opacity-40"
      >
        Cancel
      </button>
    </div>
  );
}
