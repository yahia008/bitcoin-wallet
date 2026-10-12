"use client";

import { useEffect, useRef, useState } from "react";

/** How long "Copied" stays up. */
const SHOWN_MS = 2000;

/** Copies text to the clipboard and remembers how it went for a moment, so the button can say
 * "Copied" (or that the browser refused, e.g. a page without clipboard access). Copying again
 * restarts the moment. */
export function useCopy() {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  async function copy(text: string) {
    clearTimeout(timer.current);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    timer.current = setTimeout(() => setState("idle"), SHOWN_MS);
  }

  return { copy, copied: state === "copied", failed: state === "failed" };
}

/** Says whether a copy worked, out loud too for screen readers (role="status"). Takes its
 * line even when empty, so the page doesn't jump. */
export function CopyStatus({
  copied,
  failed,
  what,
}: {
  copied: boolean;
  failed: boolean;
  what: string;
}) {
  return (
    <p
      role="status"
      className={`mt-2 h-4 text-xs ${failed ? "text-red-500" : "text-emerald-400"}`}
    >
      {copied
        ? `${what} copied to clipboard`
        : failed
          ? "Couldn't copy. Select it and copy it yourself."
          : ""}
    </p>
  );
}
