import Link from "next/link";

import { Logo } from "../logo";

export const metadata = { title: "Terms of Use · Osok Wallet" };

/** Plain-language terms for a test-network, non-custodial wallet. Placeholder text the owner
 * should review before any real use. */
export default function Terms() {
  return (
    <main className="mx-auto flex w-full max-w-xl flex-1 flex-col gap-6 px-4 py-12">
      <Link href="/" className="flex items-center gap-3">
        <Logo className="h-8 w-8" />
        <span className="text-lg font-semibold tracking-tight">Osok</span>
      </Link>
      <h1 className="text-3xl font-bold tracking-tight">Terms of Use</h1>
      <div className="flex flex-col gap-4 text-sm leading-6 text-zinc-300">
        <p>
          Osok is experimental software for Bitcoin <strong>test networks</strong> (regtest,
          testnet, signet). Coins on those networks have no value. Don&apos;t use it with real
          bitcoin.
        </p>
        <p>
          <strong>You alone hold your keys.</strong> Your recovery phrase is shown once and never
          stored or sent anywhere; your key is kept in this browser, encrypted with your password.
          The server only ever sees public keys. Nobody, including the people running Osok, can
          recover your phrase or password for you. If you lose your recovery phrase, your coins are
          gone.
        </p>
        <p>
          Before you sign anything, Osok checks it in your browser and shows you what it does. Read
          that summary: a signed transaction can&apos;t be undone.
        </p>
        <p>
          The software is provided as is, without warranty of any kind. Use it at your own risk.
        </p>
      </div>
    </main>
  );
}
