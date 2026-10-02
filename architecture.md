# Architecture

This document explains what the project is, why it's built the way it is, and how the pieces
fit together. For commands and API details see [README.md](README.md); for tests see
[test.md](test.md).

## The idea

A **non-custodial Bitcoin wallet**: you, not a server, hold the keys. The project is split so
that the part that talks to the blockchain (and could be hosted by someone else) never sees a
private key, and the part that holds keys never has to trust that server.

Three clients share one Rust core:

- **CLI** (`cargo run -- ...`): holds keys, can work alone against Bitcoin Core or Esplora.
- **Web wallet "Osok"** (`web/`): a static Next.js site; keys live in the browser, and all key
  handling runs as Rust compiled to WebAssembly.
- **Watch-only API server** (`src/bin/server.rs`): knows each wallet's *public* descriptors, so
  it can sync, show balances and history, hand out addresses and build unsigned transactions.
  It cannot spend.

The core principle is **"verify, don't trust"**: the server builds a transaction (a PSBT), but
the client re-checks every output and the fee with its own keys before it signs.

## System architecture

```mermaid
flowchart LR
    subgraph User["User's device (holds keys)"]
        direction TB
        subgraph Browser["Browser: Osok web wallet"]
            UI["Next.js UI<br/>(static export)"]
            WASM["wallet-wasm<br/>(wallet-core in WebAssembly)"]
            IDB[("IndexedDB<br/>encrypted account key,<br/>descriptors, API token")]
            UI <--> WASM
            UI <--> IDB
        end
        subgraph CLI["CLI: bitcoin-wallet"]
            CMD["main.rs commands"]
            CORE1["wallet-core"]
            SQL1[("wallet.sqlite<br/>encrypted account key<br/>+ wallet state")]
            CMD <--> CORE1
            CMD <--> SQL1
        end
    end

    subgraph Server["API server (watch-only, no private keys)"]
        AXUM["Axum HTTP API<br/>auth, rate limits, CORS"]
        BDK["BDK wallets<br/>coin selection, PSBT building,<br/>coin reservations"]
        SQL2[("data-dir/&lt;id&gt;.sqlite<br/>public descriptors,<br/>token hash")]
        AXUM <--> BDK <--> SQL2
    end

    subgraph Chain["Bitcoin network"]
        CORE["Bitcoin Core<br/>(RPC: blocks + mempool)"]
        ESP["Esplora<br/>(mempool.space / blockstream)"]
    end

    UI -- "HTTPS/JSON: descriptors, PSBTs,<br/>Bearer token" --> AXUM
    CMD -- "same API (--server)" --> AXUM
    CMD -. "or directly, standalone" .-> CORE
    CMD -. "or directly, standalone" .-> ESP
    BDK -- "sync + broadcast" --> CORE
    BDK -- "sync + broadcast" --> ESP
```

What crosses each boundary:

| From → to | What is sent | Never sent |
|---|---|---|
| Client → API | public descriptors (`tpub`), recipient + amount, signed PSBTs, API token | mnemonic, private keys, password |
| API → client | addresses, balances, history, **unsigned** PSBTs, original tx for bumps | — |
| API → chain | block/mempool queries, finalized transactions | — |

## Code layout

```
bitcoin-wallet/                 Cargo workspace
├── wallet-core/                Pure wallet logic, no I/O. Shared by CLI, server and browser.
│   ├── keys.rs                 BIP39 mnemonic → account key m/84'/1'/n' → descriptors
│   ├── crypto.rs               Argon2id + XChaCha20-Poly1305 encryption of the account key
│   ├── review.rs               review / review_bump / check_fee: verify a server's PSBT
│   ├── sign.rs                 sign a PSBT with the account key
│   └── ownership.rs            sign/verify a server challenge (API token recovery)
├── wallet-wasm/                Thin wasm-bindgen wrapper exposing wallet-core to JavaScript
├── src/                        The `bitcoin-wallet` crate (native only)
│   ├── main.rs                 CLI commands (create, restore, send, bump, ...)
│   ├── bin/server.rs           Watch-only Axum API server
│   ├── chain.rs                Chain backends: Bitcoin Core RPC or Esplora; sync
│   ├── send.rs                 build → review → sign → finalize → broadcast
│   ├── history.rs              transaction and address views
│   ├── secret.rs               encrypted account key storage in SQLite
│   ├── api.rs / client.rs      JSON types and the CLI's HTTP client for the server
│   └── lib.rs                  shared helpers (network parsing, wallet id, loading)
├── tests/regtest.rs            End-to-end: real server + regtest node, full send
├── web/                        Osok: Next.js + TypeScript + Tailwind, static export
│   ├── src/app/                screens: setup, dashboard, send, speed-up, password card
│   ├── src/lib/                api.ts (server), wallet.ts (WASM), store.ts (IndexedDB),
│   │                           accounts.ts (multi-account), format.ts (exact sat ↔ BTC)
│   └── e2e/                    Playwright browser tests
├── docker-compose.yml          bitcoind + api + web, one command
├── Dockerfile.server           builds the API server image
├── Dockerfile.web              Rust → WASM → Next.js static site → nginx
└── fund                        regtest faucet script
```

Why `wallet-core` is separate: the security checks (PSBT review, fee guard, signing) are the
part that must be correct. Writing them once in Rust and compiling the same code to
WebAssembly means the browser doesn't carry a second, TypeScript implementation that could
drift from the CLI's.

## Keys and storage

```
12-word mnemonic (BIP39)
   └── master key  (fingerprint)
         └── m/84'/1'/n'   account key  ← the only secret that is stored, encrypted
               ├── /0/*    receive addresses  (external descriptor, public)
               └── /1/*    change addresses   (internal descriptor, public)
```

- **BIP84 native SegWit** (`wpkh`). Coin type `1'` (test networks); mainnet is refused for now.
- **Only the account key is stored**, never the mnemonic. It's encrypted with the user's
  password (Argon2id key derivation + XChaCha20-Poly1305) and decrypted only while signing.
  A leaked file plus password exposes one account, not the whole seed.
- **Multiple accounts** in the web wallet are `m/84'/1'/0'`, `/1'`, `/2'`... of the same seed;
  each is registered with the server as its own wallet.
- **Wallet id** is derived from the public descriptors, so the same wallet always gets the same
  id on any server.
- **Birthday**: a new wallet records the current block height so the first Core sync doesn't
  scan from genesis.

| Where | Holds |
|---|---|
| CLI `wallet.sqlite` | encrypted account key + BDK wallet state |
| Browser IndexedDB (`wallet:<network>`) | encrypted account key, descriptors, API token, account list |
| Server `data-dir/<id>.sqlite` | public descriptors, BDK state, SHA-256 of the API token |

## Main flows

### Create a wallet and register it

```mermaid
sequenceDiagram
    participant U as User
    participant B as Browser (UI + WASM)
    participant S as API server
    U->>B: choose password
    B->>B: generate mnemonic (WASM)
    B->>U: show 12 words, ask to confirm them
    B->>B: derive account key, encrypt with password,<br/>build public descriptors
    B->>S: POST /wallets {external, internal, birthday}
    S->>S: reject if descriptor has a private key,<br/>create <id>.sqlite, store token hash
    S-->>B: 201 {id, token}
    B->>B: store encrypted key + descriptors + token in IndexedDB
```

### Send (the core security flow)

```mermaid
sequenceDiagram
    participant B as Client (browser or CLI)
    participant S as API server
    participant C as Bitcoin Core / Esplora
    B->>S: POST /psbt {address, amount, fee_priority}
    S->>C: sync, fee estimate
    S->>S: coin selection, change output,<br/>reserve coins for 10 min
    S-->>B: unsigned PSBT + summary
    B->>B: review(): recipient gets exactly the amount,<br/>every other output is our own change,<br/>input values checked against prev txs,<br/>fee guard (≤500 sat/vB, ≤10% when fee >10k sats)
    B->>B: user confirms, enters password
    B->>B: decrypt account key and sign (inside WASM)
    B->>S: POST /broadcast {signed PSBT}
    S->>C: finalize + broadcast
    S-->>B: {txid}
```

The server can lie about anything, and the client still won't sign a transaction that pays
someone else, hides a large fee, or uses a non-`SIGHASH_ALL` signature.

### Speed up (RBF fee bump)

The server builds a replacement transaction and also returns the **original** transaction. The
client checks the original's txid matches the one it asked to bump, then `review_bump`
verifies the replacement spends every original input, pays every original payment unchanged,
sends the rest to its own change, and pays a higher fee.

### API token recovery

Registration is one-shot (a second one gets `409`, since descriptors aren't secret). If a
client loses its token, it asks for a one-time challenge (`/challenge`), signs it with the key
of its first receive address (`wallet_core::ownership`), and the server verifies that against
the registered descriptor before issuing a new token (`/token`) and revoking the old one.

## Server internals

- **Axum** HTTP server; each wallet is a BDK `PersistedWallet` in its own SQLite file, opened
  lazily and kept in memory behind a mutex.
- **Auth**: per-wallet bearer token; only its SHA-256 hash is stored.
- **Wallet ids** must be exactly 16 hex chars (they become file names, so this blocks path
  traversal).
- **Coin reservations**: coins used in a handed-out PSBT are reserved for 10 minutes so two
  sends in a row don't pick the same coins. In memory only.
- **Sync throttling**: `--sync-interval-secs` (default 30) avoids re-syncing on every request
  and keeps public Esplora rate limits happy.
- **Rate limits** per IP: 60 requests/minute overall, 5 registrations/hour.
- **CORS** allows only the web wallet's origins.
- **Backends** (`chain.rs`): Bitcoin Core RPC (blocks + mempool, uses the birthday) or Esplora
  (address lookups, stop gap 20), with retries on flaky connections (broadcast isn't retried).

## Networks

| Network | Typical backend | Use |
|---|---|---|
| regtest (default) | local Bitcoin Core in Docker | development, tests, the Docker demo |
| testnet4 | Esplora (mempool.space) | real test network |
| testnet / signet | Esplora | alternatives |
| mainnet | — | refused for now |

A wallet only opens on the network it was created for, and the server checks the chain it's
connected to matches `--network`.

## Deployment: the Docker demo

```mermaid
flowchart LR
    Browser["Your browser"] -- "http://localhost:3001" --> Web
    Browser -- "http://localhost:3000<br/>(API calls from the page)" --> API
    subgraph Compose["docker compose up"]
        Web["web<br/>nginx serving the static site + .wasm"]
        API["api<br/>server --network regtest --backend core"]
        BTC["bitcoind<br/>regtest node"]
        API -- "RPC :18443" --> BTC
    end
    Fund["./fund &lt;address&gt;"] -- "docker exec bitcoin-cli" --> BTC
```

- `bitcoind` must be healthy (RPC answers) before `api` starts.
- `web` is built in three stages: Rust → WASM (`wasm-pack`), Next.js static export, nginx.
  The API URL is baked in at build time because the page calls the API from the browser.
- Ports bind to `127.0.0.1` only; chain data and server wallets live in Docker volumes.

## Security model in one list

1. The server stores **public descriptors only** and rejects private ones; a compromised server
   can see balances but can't spend.
2. Clients **review every PSBT** (and every fee bump) before signing.
3. A **high-fee guard** blocks fees that look like mistakes or attacks unless explicitly allowed.
4. Every receive address from the server is **re-derived in the browser** and refused if it
   differs.
5. Keys are **encrypted at rest**, decrypted only to sign; in the browser this happens inside
   WASM, so the key never reaches JavaScript.
6. The web wallet is a **static site**: there's no web server that could ever see a secret.

Known gaps: no TLS yet (keep the server on localhost), rate limits read the direct IP (not
`X-Forwarded-For`), and coin reservations are lost on restart.

## Testing

| Layer | Command | Covers |
|---|---|---|
| Unit | `cargo test --workspace` | encryption, descriptor checks, PSBT attack cases, rate limiter |
| End-to-end | `cargo test --test regtest -- --ignored` | real server + regtest node, full non-custodial send |
| Browser | `cd web && npm run e2e` | Playwright drives the real web wallet against a real server |
