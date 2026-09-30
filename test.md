# Testing the wallet

There are two ways to test the project:

1. **Automated tests.** One command, done in a few seconds.
2. **Manual tests.** Go through every MVP feature by hand on regtest.

---

## 1. Automated tests

```bash
cd ~/bitcoin_wallet/bitcoin-wallet

cargo test                       # 19 unit tests, no node needed
docker compose up -d             # start Bitcoin Core (regtest)
cargo test -- --include-ignored  # 21 tests: the 19 above plus the 2 end-to-end regtest tests
```

| Test | What it checks |
|---|---|
| `secret::tests::*` | The mnemonic is encrypted, a wrong password is rejected, and the plaintext is never stored |
| `send::tests::*` | `review` accepts an honest PSBT and rejects tampered ones: a changed amount, redirected change, an extra output, a weak sighash, a faked input value, a missing previous transaction |
| `server` tests | The server accepts public descriptors and rejects private ones and garbage; API tokens are random and stored hashed; coin reservations expire and release; the rate limiter blocks per IP until the window ends |
| `client::tests::*` | The CLI saves and finds each server's API token |
| `registration_is_rate_limited` (`tests/regtest.rs`) | With a limit of 2 registrations per hour, the 3rd gets `429` with a `Retry-After` header |
| `non_custodial_send_flow` (`tests/regtest.rs`) | The full API flow on a live node: register, token auth (missing, wrong, other wallet's, re-register), fund, build the PSBT, review, sign, broadcast, confirm, coin reservations |

Every test should report `ok`.

`create`, `restore` and local `send` prompt for passwords at the terminal, so no automated test covers them. Section 2 does.

---

## 2. Manual tests (every MVP feature)

### Setup (one time per terminal)

```bash
cd ~/bitcoin_wallet/bitcoin-wallet
cargo build
docker compose up -d

# No "-it" here: with -it, $(bcli ...) returns a hidden carriage return that breaks addresses.
alias bcli='docker exec bitcoind-regtest bitcoin-cli -regtest -rpcuser=wallet -rpcpassword=wallet'
alias bw='cargo run -q -- --db demo.sqlite'
mine() { bcli generatetoaddress ${1:-1} $(bcli -rpcwallet=miner getnewaddress) > /dev/null; echo "mined ${1:-1} block(s)"; }

bcli loadwallet miner 2>/dev/null || bcli createwallet miner   # the "faucet"
mine 101                                                       # coinbase needs 100 confirmations
rm -f demo.sqlite restored.sqlite                              # start clean
```

### Checklist

Each step lists the MVP requirement it covers and the result to expect.

#### ✅ Create a new wallet
```bash
bw create
```
Expect: 12 words shown **once**, and a first address starting with `bcrt1q`. **Write down the words** for the restore test.

#### ✅ BIP32 / BIP84 derivation
```bash
bw export
```
Expect: `wpkh([xxxxxxxx/84'/1'/0']tpub…/0/*)` for receive and `…/1/*` for change. There's no `tprv`: nothing private is printed.

#### ✅ New receive addresses, and tracking which are used
```bash
bw address        # index 1
bw address        # index 2: a different address every time
bw addresses      # all "unused"
```

#### ✅ Sync, and confirmed vs unconfirmed balance
```bash
bcli -rpcwallet=miner sendtoaddress <address from above> 1.5
bw balance        # Unconfirmed: 1.5 BTC (seen in the mempool)
mine
bw balance        # Confirmed: 1.5 BTC
bw addresses      # that address is now "used"
bw sync           # "Synced: scanned N new block(s), tip at height H"
```

#### ✅ Transaction history
```bash
bw history
```
Expect: `+1.50000000` with `1 conf (block N)`.

#### ✅ Sign (PSBT) and broadcast
```bash
bw send $(bcli -rpcwallet=miner getnewaddress) 0.4
```
Expect: a summary with To, Amount, Fee, Change and Inputs. Answer `y`, enter your password, and get a `Broadcast: <txid>` line. Copy the txid.

Also try:
- A wrong password. It should refuse to sign.
- Answering `n`. It should print `Cancelled.` and nothing is sent.
- More than you have, e.g. `bw send <addr> 100`. It should fail with an insufficient-funds error.
- A mainnet address (`bc1q…`). It should fail with "address is for a different network".

#### ✅ Poll transaction status
Use two terminals, with the setup aliases defined in both.
```bash
# Terminal A
bw status <txid> --watch --until 3

# Terminal B: run 3 times
mine
```
Expect: terminal A prints `unconfirmed` → `1 conf` → `2 conf` → `3 conf`, then exits.

#### ✅ Persist state between runs
```bash
bw balance        # a new process, and the balance and history are still there
bw history        # now shows the -0.4… send with its fee
```

#### ✅ Restore from an existing mnemonic
```bash
cargo run -q -- --db restored.sqlite restore    # type the 12 words from the create step
cargo run -q -- --db restored.sqlite balance    # same balance as demo.sqlite
cargo run -q -- --db restored.sqlite history    # same transactions
cargo run -q -- --db restored.sqlite address    # skips the addresses already used
```
Also try: a wrong word, or the words in the wrong order. It should fail with "invalid recovery phrase".

---

## 3. The watch-only API (bonus)

```bash
# Terminal A
cargo run -q --bin server

# Terminal B
bw register --server http://127.0.0.1:3000      # sends public descriptors only; prints id + token
TOKEN=<token printed by register>
curl -s -H "Authorization: Bearer $TOKEN" localhost:3000/wallets/<id>/balance
curl -s -H "Authorization: Bearer $TOKEN" localhost:3000/wallets/<id>/addresses
curl -s -H "Authorization: Bearer $TOKEN" localhost:3000/wallets/<id>/transactions
bw send --server http://127.0.0.1:3000 $(bcli -rpcwallet=miner getnewaddress) 0.1
mine
```
Expect: `Verified the server's PSBT: pays exactly the recipient…`. The server built the transaction, but signing happened in the CLI.

Also try:
- `send --server` with a wallet that isn't registered. It should tell you to run `register` first.
- `curl` without the header, or with a made-up token: `401`.
- `register` a second time: `Already registered`. The server answers `409` and never hands out the token again.

---

## Cleanup

```bash
rm -f demo.sqlite restored.sqlite
docker compose down        # optional: stop the node
```
