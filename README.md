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
- Coin selection, fee estimation, PSBT signing and broadcast
- Confirmation tracking (`status --watch`, or polling the API)
- Mnemonic encrypted at rest (Argon2id + XChaCha20-Poly1305); wallet state in SQLite

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
| `create` | sets one | New wallet from a freshly generated mnemonic; its birthday is the current chain tip, so Core must be running |
| `restore [--birthday H]` | sets one | Restore from an existing mnemonic (entered at a hidden prompt). Without `--birthday` the first sync scans from genesis; if unsure, pick an earlier height, since coins received before the birthday won't be found |
| `export` | no | Print the public descriptors |
| `register --server URL` | no | Register the public descriptors with an API server |
| `address` | no | Sync, then reveal a fresh receive address |
| `addresses` | no | List receive addresses and whether they have been used |
| `sync` | no | Fetch new blocks and mempool transactions |
| `balance` | no | Confirmed / unconfirmed / immature balance |
| `history` | no | Transactions, newest first |
| `send ADDRESS BTC [--server URL]` | yes | Build, review, sign and broadcast |
| `status TXID [--watch --until N]` | no | Confirmation status, or poll until N confirmations |

Global options: `--db` (default `wallet.sqlite`), plus `--rpc-url`, `--rpc-user` and `--rpc-pass`, which can also be set with the `RPC_URL`, `RPC_USER` and `RPC_PASS` environment variables.

## API server

```bash
cargo run --bin server                                   # listens on 127.0.0.1:3000
cargo run -- register --server http://127.0.0.1:3000   # saves (and prints) an API token
cargo run -- send --server http://127.0.0.1:3000 <address> 0.1
```

| Method | Endpoint | Body | Returns |
|---|---|---|---|
| GET | `/health` | | `{"status":"ok"}` |
| POST | `/wallets` | `{external, internal, birthday?}` public descriptors + start height | `201` `{id, token}` (`409` if already registered) |
| GET | `/wallets/{id}/balance` | | `{confirmed_sat, unconfirmed_sat, immature_sat, total_sat}` |
| POST | `/wallets/{id}/addresses` | | `201` `{index, address, used}` |
| GET | `/wallets/{id}/addresses` | | `[{index, address, used}]` |
| GET | `/wallets/{id}/transactions` | | `[{txid, net_sat, fee_sat, confirmed, confirmations, block_height}]` |
| GET | `/wallets/{id}/transactions/{txid}` | | one transaction (poll this for confirmations) |
| POST | `/wallets/{id}/psbt` | `{address, amount_sat, fee_rate_sat_vb?}` | unsigned PSBT (base64) + summary |
| POST | `/wallets/{id}/broadcast` | `{psbt}` signed PSBT (base64) | `{txid}` |

- **Auth:** every `/wallets/{id}/...` request needs `Authorization: Bearer <token>`. The token is returned once, at registration; the server stores only its SHA-256 hash.
- **Amounts** are integer satoshis.
- **Errors** are always `{"error": "..."}`:
  - `400` / `415` / `422`: bad input
  - `401`: missing or wrong API token
  - `404`: unknown wallet or endpoint
  - `409`: wallet already registered
  - `429`: rate limit hit; the `Retry-After` header says how many seconds to wait
  - `502`: Bitcoin Core unreachable or failing
  - `500`: anything else (details are only logged on the server)

Server options: `--data-dir` (default `server-data`), `--listen`, `--rate-limit-per-minute` (default 60 requests per IP, all endpoints) and `--register-limit-per-hour` (default 5 registrations per IP), plus the same RPC options as the CLI.

## Security model

```
CLI (your machine)                         API server (watch-only)          Bitcoin Core
 mnemonic, encrypted with your password     tpub descriptors only
 ── POST /psbt {address, amount} ────────►  picks coins, fee, change
 ◄──────────── unsigned PSBT ─────────────
 review() ← verify, don't trust the server
 sign()   ← the only step that uses keys
 ── POST /broadcast {signed PSBT} ───────►  finalize ───────────────────►  mempool
```

- **The server stores public descriptors only.** It rejects any descriptor containing a private key. If the server is compromised, an attacker can see balances but can't spend.
- **The CLI verifies every server-built PSBT before signing** (`send::review`). The recipient must get exactly the requested amount, and every other output must be the wallet's own change: the CLI derives the change script itself and compares. Input values are checked against the previous transactions' hashes, so a faked input value can't hide a large fee. Only `SIGHASH_ALL` is accepted.
- **On disk,** the mnemonic is encrypted in `wallet.sqlite`. It's decrypted only while signing, and the wallet database otherwise holds only public data.
- **Per-wallet API tokens.** Without its token nobody can read a wallet, reveal addresses or build PSBTs (which would reserve its coins). Registering is one-shot, since descriptors aren't secret: a second registration gets `409`, never the token. A lost token can't be recovered yet; the server admin has to delete the wallet from `--data-dir` so it can be registered again. Wallets registered before tokens existed must be re-registered the same way.
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
src/lib.rs          shared: NETWORK, load(), wallet_id(), confirmations()
src/keys.rs         mnemonic → BIP84 descriptors and keys
src/secret.rs       encrypted mnemonic storage
src/chain.rs        Bitcoin Core RPC and sync
src/history.rs      transaction and address views
src/send.rs         build → review → sign → finalize → broadcast
src/api.rs          JSON types shared by the server and the client
src/client.rs       HTTP client used by the CLI
src/main.rs         CLI
src/bin/server.rs   API server
tests/regtest.rs    end-to-end test
```

## Known limitations

- Regtest only; the coin type and network are fixed in `keys.rs` and `lib.rs`.
- The API has no TLS.
- A lost API token can't be recovered; the server admin has to delete the wallet so it can be registered again.
- Coin reservations and rate-limit counts live in memory, so a server restart clears them.
