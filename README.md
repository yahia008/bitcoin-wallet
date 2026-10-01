# bitcoin-wallet

A non-custodial Bitcoin wallet in Rust, built on [BDK](https://bitcoindevkit.org) and Bitcoin Core. It comes as:

- **a CLI** that holds your keys and can do everything on its own, and
- **a watch-only HTTP API** that can see wallets and build transactions but **never holds private keys**. The CLI signs, and the server broadcasts.

It currently runs on **regtest** only.

## Features

- BIP39 mnemonic (12 words); create or restore
- Wallet birthday: the first sync starts at the block the wallet was created at, not at genesis
- BIP84 native SegWit derivation: `m/84'/1'/0'/0/*` for receive, `m/84'/1'/0'/1/*` for change
- Fresh receive addresses, with tracking of which ones have been used
- Sync with Bitcoin Core (blocks and mempool); confirmed, unconfirmed and immature balance
- Transaction history with fees and confirmation counts
- Coin selection, fee estimation (`send --fee fast|normal|slow` targets 2, 6 or 144 blocks, or `--fee-rate N` for an exact sat/vB; never below 1 sat/vB), PSBT signing and broadcast
- High-fee guard: `send` refuses to sign a fee over 500 sat/vB, or over 10% of the amount once the fee is above 10,000 sats, unless you pass `--allow-high-fee`. It also applies to PSBTs from the API server, so a compromised server can't drain the wallet through fees
- Confirmation tracking (`status --watch`, or polling the API)
- Only the account key (m/84'/1'/0') is kept, encrypted at rest (Argon2id + XChaCha20-Poly1305); the mnemonic is never stored. Wallet state in SQLite

## Quick start

Requirements: Rust (stable) and Docker.

```bash
# 1. Start Bitcoin Core on regtest
docker compose up -d
alias bcli='docker exec -it bitcoind-regtest bitcoin-cli -regtest -rpcuser=wallet -rpcpassword=wallet'
bcli createwallet miner
bcli -generate 101                 # coinbase needs 100 confirmations to be spendable

# 2. Create a wallet and receive some coins
cargo run -- create                # write down the 12 words; choose a password
cargo run -- address
bcli -rpcwallet=miner sendtoaddress <address> 1
bcli -generatetoaddress 1 $(bcli -rpcwallet=miner getnewaddress)
cargo run -- balance

# 3. Send
cargo run -- send $(bcli -rpcwallet=miner getnewaddress) 0.1
cargo run -- status <txid> --watch
```

## CLI

| Command | Needs password | What it does |
|---|---|---|
| `create` | sets one | New wallet from a freshly generated mnemonic; its birthday is the current chain tip, so the chain backend must be reachable |
| `restore [--birthday H]` | sets one | Restore from an existing mnemonic (entered at a hidden prompt). Without `--birthday` the first sync scans from genesis; if unsure, pick an earlier height, since coins received before the birthday won't be found |
| `export` | no | Print the public descriptors |
| `register --server URL` | only to recover | Register the public descriptors with an API server. If the server already knows the wallet but this file has no working token (restored into a new file, registered from the web wallet, or signed out), it asks for the password, proves ownership and gets a new token; other copies of the wallet on that server are signed out |
| `address` | no | Sync, then reveal a fresh receive address |
| `addresses` | no | List receive addresses and whether they have been used |
| `sync` | no | Fetch new blocks and mempool transactions |
| `balance` | no | Confirmed / unconfirmed / immature balance |
| `history` | no | Transactions, newest first |
| `send ADDRESS BTC [--server URL]` | yes | Build, review, sign and broadcast |
| `bump TXID [--fee-rate N \| --fee P] [--server URL]` | yes | Replace an unconfirmed transaction with a higher-fee copy (RBF). Payments stay the same; the extra fee comes from change. Defaults to the `fast` estimate, and never less than the old rate + 1 sat/vB |
| `status TXID [--watch --until N]` | no | Confirmation status, or poll until N confirmations |

Global options (each can also be set with the environment variable in brackets):

- `--db` (default `wallet.sqlite`)
- `--network` [`NETWORK`]: `regtest` (default), `testnet4`, `testnet` or `signet`. Mainnet is refused for now. A wallet only opens on the network it was created for.
- `--backend` [`BACKEND`]: `core` or `esplora`. Defaults to `core` on regtest and `esplora` elsewhere.
- `--esplora-url` [`ESPLORA_URL`]: defaults to mempool.space (testnet4, signet) or blockstream.info (testnet).
- `--rpc-url`, `--rpc-user`, `--rpc-pass` [`RPC_URL`, `RPC_USER`, `RPC_PASS`]: for `--backend core`. The URL defaults to localhost on the network's port.

On testnet4 with no node of your own:

```bash
export NETWORK=testnet4
cargo run -- create      # then get coins from a testnet4 faucet
cargo run -- balance     # syncs through mempool.space
```

Esplora looks up the wallet's addresses directly (a full scan up to 20 unused addresses per keychain), so there's no node to run and the birthday isn't used. The Esplora server does learn which addresses are yours.

## API server

```bash
cargo run --bin server                                   # listens on 127.0.0.1:3000
cargo run -- register --server http://127.0.0.1:3000   # saves (and prints) an API token
cargo run -- send --server http://127.0.0.1:3000 <address> 0.1
```

| Method | Endpoint | Body | Returns |
|---|---|---|---|
| GET | `/health` | | `{"status":"ok","network":"testnet4"}` |
| POST | `/wallets` | `{external, internal, birthday?}` public descriptors + start height | `201` `{id, token}` (`409` if already registered) |
| POST | `/wallets/{id}/challenge` | | `{challenge, expires_in_secs}`: one-time, 5 minutes, no token needed |
| POST | `/wallets/{id}/token` | `{challenge, signature}` (`wallet_core::ownership::sign`) | `{token}`: a new API token; the old one stops working |
| GET | `/wallets/{id}/balance` | | `{confirmed_sat, unconfirmed_sat, immature_sat, total_sat}` |
| POST | `/wallets/{id}/addresses` | | `201` `{index, address, used}` |
| GET | `/wallets/{id}/addresses` | | `[{index, address, used}]` |
| GET | `/wallets/{id}/transactions` | | `[{txid, net_sat, fee_sat, confirmed, confirmations, block_height}]` |
| GET | `/wallets/{id}/transactions/{txid}` | | one transaction (poll this for confirmations) |
| POST | `/wallets/{id}/psbt` | `{address, amount_sat, fee_rate_sat_vb?, fee_priority?}` | unsigned PSBT (base64) + summary. `fee_priority` is `fast`, `normal` (default) or `slow`; an exact `fee_rate_sat_vb` overrides it |
| POST | `/wallets/{id}/bump` | `{txid, fee_rate_sat_vb?, fee_priority?}` | unsigned replacement PSBT (RBF) + `original_tx` (hex). `fee_priority` defaults to `fast` |
| POST | `/wallets/{id}/broadcast` | `{psbt}` signed PSBT (base64) | `{txid}` |

- **Auth:** every `/wallets/{id}/...` request needs `Authorization: Bearer <token>`. The token is returned once, at registration; the server stores only its SHA-256 hash.
- **Amounts** are integer satoshis.
- **Errors** are always `{"error": "..."}`:
  - `400` / `415` / `422`: bad input
  - `401`: missing or wrong API token
  - `404`: unknown wallet or endpoint
  - `409`: wallet already registered
  - `429`: rate limit hit; the `Retry-After` header says how many seconds to wait
  - `502`: chain backend (Bitcoin Core or Esplora) unreachable or failing
  - `500`: anything else (details are only logged on the server)

Server options: `--data-dir` (default `server-data`), `--listen`, `--rate-limit-per-minute` (default 60 requests per IP, all endpoints) and `--register-limit-per-hour` (default 5 registrations per IP), plus the same network and backend options as the CLI. Use a separate `--data-dir` per network. `--sync-interval-secs` (default 30): a wallet synced that recently isn't synced again, so a web page load costs one chain scan instead of three, and public Esplora rate limits (blockstream.info: 700 requests/hour) aren't hit; 0 syncs on every request. `--cors-origins` (env `CORS_ORIGINS`, comma-separated, default `http://localhost:3001,http://127.0.0.1:3001`) lists the web pages allowed to call the API from a browser.

## Web wallet (Osok)

`web/` is Osok, a Next.js + TypeScript + Tailwind app that will do what the CLI does, in the browser: keys stay in the browser, which talks to the same API server. It builds to static files (`output: "export"`), so there's no Next.js server that could ever see a secret.

The browser runs the same Rust code as the CLI: `wallet-core` (keys, PSBT review, signing) compiled to WebAssembly through the thin `wallet-wasm` bindings crate. Building it needs `wasm-pack` (`cargo install wasm-pack`) and `clang` (`sudo apt install clang`, for secp256k1's C code); `npm run dev` and `npm run build` build it first.

```bash
cargo run --bin server -- --network testnet4 --data-dir server-testnet4   # API on :3000
cd web && npm install && npm run dev                                       # wallet on :3001
```

Open http://localhost:3001. It shows whether it can reach the API and on which network, then lets you create a wallet (12 new words, shown once) or restore one from its words. The browser derives the account key, encrypts it with your password (the same Argon2id + XChaCha20-Poly1305 code as the CLI, in `wallet_core::crypto`) and keeps only that, plus the public descriptors and API token, in IndexedDB. Only the public descriptors are sent to the server. Once set up, it shows the balance, a receive address with a QR code, and the transaction history. Every receive address the server hands out is re-derived in the browser from the wallet's own public descriptor and refused if it differs, so a compromised server can't show its own address as yours. Sending works like the CLI's `send --server`: the server builds an unsigned PSBT, the browser checks it with the same `review` and `check_fee` code (in WASM), shows the amount, fee, rate and change, and on your password decrypts the key and signs inside WASM, so the key never reaches JavaScript. A fee that looks like a mistake has to be confirmed explicitly. Unconfirmed transactions you sent get a "Speed up" button (RBF, like the CLI's `bump --server`): the server builds the replacement and sends the original along, and the browser runs `review_bump` (the original's txid must match, every payment must be unchanged, the fee must be higher) before you sign.

For developing the web wallet, regtest is the smoothest backend (no rate limits, blocks on demand): `docker compose up -d`, then `cargo run --bin server` (regtest is the default). Set `NEXT_PUBLIC_API_URL` to point it at another server (default `http://127.0.0.1:3000`).

## Security model

```
CLI (your machine)                         API server (watch-only)          Core or Esplora
 account tprv, encrypted with your password tpub descriptors only
 ── POST /psbt {address, amount} ────────►  picks coins, fee, change
 ◄──────────── unsigned PSBT ─────────────
 review() ← verify, don't trust the server
 sign()   ← the only step that uses keys
 ── POST /broadcast {signed PSBT} ───────►  finalize ───────────────────►  mempool
```

- **The server stores public descriptors only.** It rejects any descriptor containing a private key. If the server is compromised, an attacker can see balances but can't spend.
- **The CLI verifies every server-built PSBT before signing** (`send::review`). The recipient must get exactly the requested amount, and every other output must be the wallet's own change: the CLI derives the change script itself and compares. Input values are checked against the previous transactions' hashes, so a faked input value can't hide a large fee. Only `SIGHASH_ALL` is accepted.
- **Fee bumps are verified too** (`send::review_bump`). The server sends the original transaction, and the CLI checks its txid matches the one it asked to bump. The replacement must spend every coin the original spent, pay every payment exactly as before, send everything else to your own change, and pay a higher fee. A server can't use a "fee bump" to redirect a payment.
- **On disk,** `wallet.sqlite` holds only the account key (`[fingerprint/84'/1'/0']tprv…`), encrypted with your password. It's decrypted only while signing, and the database otherwise holds only public data. The mnemonic itself is never stored, so a leaked file plus password exposes this one account, not everything the seed controls; your written-down words are the only copy. Wallets created before this stored the mnemonic; the first `send` upgrades them to the account key.
- **Per-wallet API tokens.** Without its token nobody can read a wallet, reveal addresses or build PSBTs (which would reserve its coins). Registering is one-shot, since descriptors aren't secret: a second registration gets `409`, never the token. A lost token is recovered by proving you hold the wallet's key: the server issues a one-time challenge (`/challenge`), the wallet signs it with the key of its first receive address (`wallet_core::ownership`), and the server checks that against the public key from the registered descriptor before issuing a new token (`/token`) and revoking the old one. A challenge accepts one answer and expires after 5 minutes, and `/token` shares the registration rate limit. The CLI's `register` and the web wallet's restore do this automatically when the server already knows the wallet. Wallets registered before tokens existed have to be deleted from `--data-dir` and registered again.
- **Rate limits per IP** stop registration spam (each registration creates a database file) and request floods (each request syncs with Core). They're in memory and use the connecting IP, so behind a reverse proxy they'd need to read `X-Forwarded-For` instead.
- **No TLS yet,** so tokens travel in plain text. Keep the server on `127.0.0.1` (the default).

## Tests

```bash
cargo test                        # unit tests: encryption, descriptor checks, PSBT attack cases
docker compose up -d
cargo test -- --ignored           # end-to-end: real server + regtest, full non-custodial send
```

## Project layout

```
src/lib.rs          shared: parse_network(), load(), wallet_id(), confirmations()
src/secret.rs       encrypted account key storage
src/chain.rs        chain backends (Core RPC, Esplora) and sync
src/history.rs      transaction and address views
src/send.rs         build → review → sign → finalize → broadcast
src/api.rs          JSON types shared by the server and the client
src/client.rs       HTTP client used by the CLI
src/main.rs         CLI
src/bin/server.rs   API server
tests/regtest.rs    end-to-end test
wallet-core/        keys, PSBT review and signing with no I/O; shared by the CLI and the browser
wallet-wasm/        JavaScript bindings for wallet-core
web/                web wallet (Next.js, static export)
```

## Known limitations

- Regtest only; the coin type and network are fixed in `keys.rs` and `lib.rs`.
- The API has no TLS.
- A lost API token can't be recovered; the server admin has to delete the wallet so it can be registered again.
- Coin reservations and rate-limit counts live in memory, so a server restart clears them.
