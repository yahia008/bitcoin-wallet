"use client";

import { useEffect, useState } from "react";

import { API_URL, health, type Health } from "@/lib/api";

type Status =
  | { kind: "checking" }
  | { kind: "ok"; health: Health }
  | { kind: "error"; message: string };

export default function Home() {
  const [status, setStatus] = useState<Status>({ kind: "checking" });

  useEffect(() => {
    health()
      .then((h) => setStatus({ kind: "ok", health: h }))
      .catch((e: unknown) =>
        setStatus({ kind: "error", message: e instanceof Error ? e.message : String(e) }),
      );
  }, []);

  return (
    <main className="mx-auto flex w-full max-w-xl flex-1 flex-col gap-6 px-4 py-16">
      <h1 className="text-3xl font-semibold tracking-tight">Bitcoin Wallet</h1>
      <p className="text-zinc-600 dark:text-zinc-400">
        Non-custodial: your keys stay in this browser. The server only sees public keys.
      </p>

      <section className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
        <h2 className="mb-2 font-medium">API server</h2>
        <p className="font-mono text-sm text-zinc-500">{API_URL}</p>
        {status.kind === "checking" && <p className="mt-2">Checking…</p>}
        {status.kind === "ok" && (
          <p className="mt-2 text-green-700 dark:text-green-400">
            Connected · network <strong>{status.health.network}</strong>
          </p>
        )}
        {status.kind === "error" && (
          <p className="mt-2 text-red-700 dark:text-red-400">
            Can&apos;t reach it: {status.message}. Is the server running with CORS allowing
            this page?
          </p>
        )}
      </section>
    </main>
  );
}
