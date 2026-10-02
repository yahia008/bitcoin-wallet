# Running the MVP locally

This demo runs every MVP feature in order on regtest. It uses a fresh `demo.sqlite`, so existing wallets aren't touched.

## 0. Setup (one time)

```bash
cd ~/bitcoin_wallet/bitcoin-wallet
cargo build                                   # compile once so the demo runs fast

docker compose up -d bitcoind                          # start Bitcoin Core (regtest)

# Shortcuts. Note: this bcli has NO "-it". With -it, $(bcli ...) gets a hidden
# carriage return at the end, which breaks addresses passed on to other commands.
alias bcli='docker exec bitcoind-regtest bitcoin-cli -regtest -rpcuser=wallet -rpcpassword=wallet'
alias bw='cargo run -q -- --db demo.sqlite'
mine() { bcli generatetoaddress ${1:-1} $(bcli -rpcwallet=miner getnewaddress) > /dev/null; echo "mined ${1:-1} block(s)"; }

bcli loadwallet miner 2>/dev/null || bcli createwallet miner   # the "faucet"
mine 101                                      # make sure the faucet has spendable coins
rm -f demo.sqlite                             # start clean
```

To keep the `bcli` alias, put the line above in `~/.bashrc`. If `~/.bashrc` already has an older `bcli` line that uses `-it`, replace it.

---

## 1. Create a wallet (spec: create wallet, BIP39, BIP84)

```bash
bw create           # choose a password (8+ characters); prints the birthday (current block)
bw export           # shows the path: [fingerprint/84'/1'/0'] …/0/* receive, …/1/* change
```

**What to point out:**
- It shows 12 words **once**, and they're stored **encrypted**.
- The first address starts with `bcrt1q` (native SegWit).

## 2. Receive addresses (spec: generate addresses, track used)

```bash
bw address          # index 1
bw address          # index 2, a new address each time
bw addresses        # all "unused" so far
```

## 3. Receive money, unconfirmed → confirmed (spec: sync, confirmed/unconfirmed balance)

```bash
bcli -rpcwallet=miner sendtoaddress <address from step 2> 1.5
bw balance          # Unconfirmed: 1.5 BTC (seen in the mempool)
mine
bw balance          # Confirmed: 1.5 BTC
bw addresses        # that address is now "used"
```

## 4. History (spec: list transaction history)

```bash
bw history          # +1.50000000, 1 conf (block N)
```

## 5. Send via PSBT (spec: coin selection, fee, sign PSBT, broadcast)

```bash
bw send $(bcli -rpcwallet=miner getnewaddress) 0.4
```

**What to point out:** the summary shows amount, fee, change and inputs. Answer `y`, then enter your password. Signing happens locally, then the transaction is broadcast and you get a **txid**.

## 6. Confirmation tracking (spec: poll status)

This step needs two terminals. Define the aliases from step 0 in both.

```bash
# Terminal A:
bw status <txid> --watch --until 3

# Terminal B, run 3 times:
mine
```

Terminal A prints `unconfirmed` → `1 conf` → `2 conf` → `3 conf` and then stops by itself.

## 7. Persistence and restore (spec: restore from mnemonic, persist state)

```bash
bw balance                                             # a new process, and state is still there
cargo run -q -- --db restored.sqlite restore           # type the 12 words from step 1
cargo run -q -- --db restored.sqlite balance           # same balance, found by syncing
cargo run -q -- --db restored.sqlite history           # same transactions
```

---

## Bonus: the non-custodial API

```bash
# Terminal A:
cargo run -q --bin server

# Terminal B:
bw register --server http://127.0.0.1:3000             # sends public keys only; prints id + token
TOKEN=<token printed by register>
curl -s -H "Authorization: Bearer $TOKEN" localhost:3000/wallets/<id>/balance
curl -s -H "Authorization: Bearer $TOKEN" localhost:3000/wallets/<id>/transactions
curl -s localhost:3000/wallets/<id>/balance            # no token: 401
bw send --server http://127.0.0.1:3000 $(bcli -rpcwallet=miner getnewaddress) 0.1
```

**What to point out:** *"Verified the server's PSBT…"*. The server built the transaction, but the keys never left the CLI.

## Tests

```bash
cargo test                  # 14 unit tests, including PSBT attack cases
cargo test -- --ignored     # the full end-to-end flow against regtest (~2 s)
```

## Cleanup

```bash
rm -f demo.sqlite restored.sqlite
```
