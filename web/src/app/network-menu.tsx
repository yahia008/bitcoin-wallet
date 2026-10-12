"use client";

import { useEffect, useRef, useState } from "react";

import { API_SERVERS } from "@/lib/api";

import { CheckIcon, ChevronIcon } from "./icons";

/** What each network is, in the menu. */
const DESCRIPTIONS: Record<string, string> = {
  regtest: "Local test chain, blocks in seconds",
  testnet4: "Public test network",
  testnet: "Public test network (old)",
  signet: "Public test network, steady blocks",
};

/** The networks this site has a server for (NEXT_PUBLIC_API_URLS), in the configured order. */
const NETWORKS = API_SERVERS.flatMap((s) => (s.network ? [s.network] : []));

/** The network pill. When the site offers more than one network, it opens a menu to switch:
 * each network has its own API server and its own wallet in this browser. */
export function NetworkBadge({
  network,
  onSelect,
}: {
  network: string;
  onSelect: (network: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);

  // Clicking anywhere else, or Escape, closes it.
  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!menu.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const pill = "rounded-full border border-zinc-800 px-3 py-1 text-xs text-zinc-400";
  if (NETWORKS.length < 2) return <span className={pill}>{network}</span>;

  return (
    <div ref={menu} className="relative">
      <button
        type="button"
        aria-label={`Network: ${network}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={`${pill} inline-flex items-center gap-1 hover:border-zinc-700 hover:text-zinc-200`}
      >
        {network}
        <ChevronIcon />
      </button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 z-10 mt-2 w-64 rounded-xl border border-zinc-800 bg-zinc-900 p-1 text-left shadow-xl"
        >
          {NETWORKS.map((n) => (
            <button
              key={n}
              type="button"
              role="menuitemradio"
              aria-checked={n === network}
              onClick={() => {
                setOpen(false);
                if (n !== network) onSelect(n);
              }}
              className="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 hover:bg-zinc-800"
            >
              <span>
                <span className="block text-sm capitalize text-zinc-100">{n}</span>
                {DESCRIPTIONS[n] && (
                  <span className="block text-xs text-zinc-500">{DESCRIPTIONS[n]}</span>
                )}
              </span>
              {n === network && <CheckIcon className="h-4 w-4 shrink-0 text-violet-300" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
