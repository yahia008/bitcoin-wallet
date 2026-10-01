"use client";

import { useState } from "react";

import { loadWallet } from "@/lib/wallet";

/** A first test of the WASM module: 12 words in, first receive address out, all in-browser. */
export function FirstAddress({ network }: { network: string }) {
  const [words, setWords] = useState("");
  const [result, setResult] = useState<{ address?: string; error?: string }>({});

  async function derive() {
    try {
      const wallet = await loadWallet();
      setResult({ address: wallet.firstAddress(words, network) });
    } catch (e) {
      setResult({ error: e instanceof Error ? e.message : String(e) });
    }
  }

  return (
    <section className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
      <h2 className="mb-2 font-medium">First address from a recovery phrase</h2>
      <p className="mb-3 text-sm text-zinc-500">
        Runs the wallet&apos;s Rust code in this browser; nothing is sent anywhere. Try the
        BIP39 test phrase &ldquo;abandon&rdquo; ×11 + &ldquo;about&rdquo;, not a real one.
      </p>
      <textarea
        className="w-full rounded border border-zinc-300 bg-transparent p-2 font-mono text-sm dark:border-zinc-700"
        rows={3}
        placeholder="twelve words separated by spaces"
        value={words}
        onChange={(e) => setWords(e.target.value)}
        autoComplete="off"
        spellCheck={false}
      />
      <button
        className="mt-2 rounded bg-zinc-900 px-3 py-1.5 text-sm text-white disabled:opacity-40 dark:bg-zinc-100 dark:text-zinc-900"
        onClick={derive}
        disabled={!words.trim()}
      >
        Show first address
      </button>
      {result.address && <p className="mt-3 break-all font-mono text-sm">{result.address}</p>}
      {result.error && <p className="mt-3 text-sm text-red-700 dark:text-red-400">{result.error}</p>}
    </section>
  );
}
