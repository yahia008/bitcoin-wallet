# How this wallet works, explained with one example

This is the long, friendly version of [architecture.md](architecture.md). It follows **one
wallet** from its 12 words to a sent, sped-up transaction, and stops at every step to explain
what happens and why. The keys, descriptors and addresses below are **real**: they were
computed by this project's own code (`wallet-wasm`) from the phrase shown. Amounts and fees in
the send example are worked out by hand, but use the same rules the code uses.

> ⚠️ The phrase used here is a famous public test phrase. Everyone knows it. Never put real
> money on it.

---

## 1. The big picture in one paragraph

A Bitcoin wallet doesn't "hold coins". Coins live on the blockchain, locked to addresses.
A wallet holds **keys**: the secrets that prove you may spend what's locked to your addresses.
This project splits the work in two:

- **The server** (`src/bin/server.rs`) watches the blockchain for you. It knows your
  **public** keys, so it can find your coins, show your balance and *prepare* transactions.
  It can't spend: it never has a private key.
- **The client** (the CLI, or the **Osok** web wallet in your browser) holds the **private**
  key, encrypted with your password. It checks everything the server prepares, and only then
  signs.

The rule that ties it together: **verify, don't trust.** The server could be hacked, buggy or
malicious, and the client still won't sign anything that sends your money somewhere you
didn't ask.

```
   You (browser / CLI)                       Server (watch-only)            Bitcoin network
 ┌────────────────────────┐   public keys  ┌─────────────────────┐  sync  ┌──────────────┐
 │ 12 words → private key │ ─────────────▶ │ finds your coins,    │ ◀────▶ │ Bitcoin Core │
 │ (encrypted)            │                │ builds UNSIGNED txs  │        │  or Esplora  │
 │ checks + signs         │ ◀───────────── │ broadcasts signed    │ ─────▶ │              │
 └────────────────────────┘  unsigned tx   └─────────────────────┘        └──────────────┘
```

---

## 2. From 12 words to keys (BIP39 + BIP32 + BIP84)

### 2.1 The phrase (BIP39)

When you press **Create wallet**, the browser (inside WASM) generates 128 random bits and
turns them into 12 words from a fixed list of 2048:

```
abandon abandon abandon abandon abandon abandon
abandon abandon abandon abandon abandon about
```

Those words *are* the wallet. Anyone who has them has the money; lose them and nobody can help
you. That's why Osok makes you tap the words back in the right order before it continues
(`e24297a`): it proves you actually wrote them down.

The words are stretched (PBKDF2, 2048 rounds) into a 512-bit **seed**, and the seed becomes
the **master key**. The master key has a short ID called the **fingerprint**:

```
master fingerprint: 73c5da0a
```

### 2.2 The key tree (BIP32) and the path (BIP84)

From the master key you can derive a whole tree of child keys. A **path** says which branch
to take. We use the BIP84 path for native SegWit:

```
m / 84' / 1' / 0' / 0 / 5
│   │     │    │    │   └── index: the 6th address (counting from 0)
│   │     │    │    └────── 0 = receive addresses, 1 = change addresses
│   │     │    └─────────── account number (0 = first account, 1 = second, ...)
│   │     └──────────────── coin type: 1' = any test network (mainnet would be 0')
│   └────────────────────── purpose: 84' = BIP84 = native SegWit (addresses start bc1q / tb1q)
└────────────────────────── the master key
```

The `'` means **hardened**. A hardened child can only be made from the *private* parent. That
matters below: it's why we can give the server the account's public key without exposing the
other accounts.

**In code:** `wallet-core/src/keys.rs` turns the mnemonic into the account key at
`m/84'/1'/0'`.

### 2.3 Descriptors: the wallet's public "recipe"

From the account key we build two **output descriptors**. A descriptor is a one-line recipe
that says "this is how to make every address of this wallet". Here are the real ones for our
phrase:

```
receive:  wpkh([73c5da0a/84'/1'/0']tpubDC8msFGeGuwnKG9Upg7DM2b4DaRqg3CUZa5g8v2SRQ6K4NSkxUgd7HsL2XVWbVm39yBA4LAxysQAm397zwQSQoQgewGiYZqrA9DsP4zbQ1M/0/*)#2ag6nxcd
change:   wpkh([73c5da0a/84'/1'/0']tpubDC8msFGeGuwnKG9Upg7DM2b4DaRqg3CUZa5g8v2SRQ6K4NSkxUgd7HsL2XVWbVm39yBA4LAxysQAm397zwQSQoQgewGiYZqrA9DsP4zbQ1M/1/*)#mfdmwng4
```

Reading one piece by piece:

| Piece | Meaning |
|---|---|
| `wpkh(...)` | "witness pay-to-public-key-hash": native SegWit, single key |
| `[73c5da0a/84'/1'/0']` | where the key came from: master fingerprint + path. Signers use this to find the right private key |
| `tpubDC8m...` | the account's **extended public key** (`tpub` = testnet public). Public only: it can make addresses, never spend |
| `/0/*` or `/1/*` | `0` = receive branch, `1` = change branch; `*` = "any index" |
| `#2ag6nxcd` | a checksum, so a typo is caught |

These two strings are **all the server ever gets**. If someone sends a descriptor with a
`tprv` (private key) in it, the server refuses it (`public_descriptor` in `server.rs`; the
test `rejects_private_descriptor` checks this).

### 2.4 Addresses

Fill in `*` with 0, 1, 2… and you get addresses. Real values for our phrase:

| Path | Kind | regtest | testnet4 |
|---|---|---|---|
| `m/84'/1'/0'/0/0` | receive #0 | `bcrt1q6rz28mcfaxtmd6v789l9rrlrusdprr9pz3cppk` | `tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl` |
| `m/84'/1'/0'/0/1` | receive #1 | `bcrt1qd7spv5q28348xl4myc8zmh983w5jx32cs707jh` | `tb1qd7spv5q28348xl4myc8zmh983w5jx32cjhkn97` |
| `m/84'/1'/0'/1/0` | change #0 | `bcrt1q9u62588spffmq4dzjxsr5l297znf3z6jkgnhsw` | `tb1q9u62588spffmq4dzjxsr5l297znf3z6j5p2688` |
| `m/84'/1'/0'/1/4` | change #4 | `bcrt1qw3xfnyuspj8qnr2envc448mxwam7f7p249rqs0` | `tb1qw3xfnyuspj8qnr2envc448mxwam7f7p2hv6d8x` |

Two things to notice:

1. **Same key, different network prefix.** `bcrt1q6rz28…` and `tb1q6rz28…` are the *same*
   public key hash. Only the human-readable prefix (`bcrt` / `tb`) and the checksum at the
   end differ. That's why the wallet is pinned to one network: an address for the wrong
   network is refused (`checkAddress` in the Send screen).
2. **Receive vs change.** You give out receive addresses. Change addresses are used by your
   own wallet for the "leftover" of a payment (section 6). People never see them, but they
   matter a lot for safety (section 7.3).

### 2.5 More accounts

**Add wallet** in Osok makes account 1, 2, … from the **same** phrase:

```
account 1: wpkh([73c5da0a/84'/1'/1']tpubDC8msFGeGuwnP2xwTZBBZSie1BLg.../0/*)
first address: bcrt1qp7shgcwx3mpzgxjvff0d77vuhchcldzfxnktde
```

Same fingerprint `73c5da0a`, different `1'` in the path, completely different addresses.
Because the account level is **hardened**, the server holding account 0's `tpub` can't work
out account 1's. Adding an account asks for your phrase again: we only store an account's
key, never the phrase (section 3).

### 2.6 The wallet id

The server needs a name for each wallet's file. It's computed from the two descriptors:

```
account 0 → wallet id b12e8e68b97d63eb   → server file data-dir/b12e8e68b97d63eb.sqlite
account 1 → wallet id 7012e6dc9c228fea
```

Same descriptors → same id, on any server. (The id is the same on regtest and testnet4 too,
since the `tpub` is the same. That's fine because each network uses its own server and data
dir.) The server only accepts ids that are exactly 16 hex characters, because the id becomes
a file name: something like `../../etc` can't get through.

---

## 3. Keeping the key safe: what is stored where

### 3.1 Only the account key is stored, encrypted

We **don't store the 12 words**. We store only the account private key (`m/84'/1'/0'`,
written as a `tprv`), and only encrypted:

```
password "correct horse battery"
        │  Argon2id (slow on purpose, ~0.2 s, uses a random salt)
        ▼
  256-bit encryption key
        │  XChaCha20-Poly1305 (encrypts + detects tampering, random nonce)
        ▼
{ "salt":   "ccf7766cf34336e09fa08df47e75ba2e",
  "nonce":  "2e13951c54a642934481c5f2dfac16a00c50dfcaf24bfa6d",
  "ciphertext": "719d15527625ed3aeae14f7ed40d2e57185af2d3c2478a8985fb26a693…" }
```

That object (from a real run; the salt and nonce are random, so yours will differ) is what
Osok keeps in **IndexedDB** in your browser, and what the CLI keeps in `wallet.sqlite`.

- **Why Argon2id?** It's slow and memory-hungry on purpose, so guessing passwords costs an
  attacker a lot of time per guess.
- **Why only the account key?** If someone steals the file *and* your password, they get
  that one account, not the phrase and so not every account. It also means "Add account"
  needs the phrase typed in again. (We decided to keep it that way.)
- **In the browser** the key is decrypted *inside* WebAssembly to sign, and never handed to
  JavaScript (`signPsbt`).

**In code:** `wallet-core/src/crypto.rs`.

### 3.2 Who holds what

| Place | Holds | Can it spend? |
|---|---|---|
| Your browser (IndexedDB) | encrypted account key, descriptors, API token | only with your password |
| CLI `wallet.sqlite` | encrypted account key, wallet state | only with your password |
| Server `data-dir/<id>.sqlite` | descriptors, sync state, **hash** of the API token, coin reservations | **never** |

---

## 4. Registering with the server

Right after creating the wallet, Osok registers it:

```
POST /wallets
{ "external": "wpkh([73c5da0a/84'/1'/0']tpubDC8m…/0/*)#2ag6nxcd",
  "internal": "wpkh([73c5da0a/84'/1'/0']tpubDC8m…/1/*)#mfdmwng4",
  "birthday": 812 }

→ 201 { "id": "b12e8e68b97d63eb", "token": "9f3c…(64 hex chars)…" }
```

- **birthday** = the block height when the wallet was created. When syncing with Bitcoin
  Core, the server starts scanning from there instead of from block 0. Much faster, and safe
  because a brand-new wallet can't have older coins. (A restored wallet can pass an earlier
  birthday.)
- **token**: from now on every request needs `Authorization: Bearer <token>`. The server
  stores only its SHA-256 hash, so a leaked server database doesn't leak working tokens.
- **Registering again gets `409`.** Descriptors aren't secret, so "I know the descriptor" is
  no proof you own the wallet, and the server mustn't hand out a second token for it.

### 4.1 Lost your token? Prove you own the key

Say you clear your browser and restore from the 12 words. Registration now returns 409. Osok
handles it by itself:

```
POST /wallets/b12e8e68b97d63eb/challenge   → { "challenge": "5d1e…" }     (one use, 5 minutes)
  browser signs the challenge with the private key of receive #0 (m/84'/1'/0'/0/0)
POST /wallets/b12e8e68b97d63eb/token  { challenge, signature }
  server checks the signature against the public key from the registered descriptor
→ { "token": "new…" }   and the old token stops working
```

Only someone with the private key can make that signature. **In code:**
`wallet-core/src/ownership.rs`.

---

## 5. Receiving coins

1. You press **Receive**. The server returns the next unused receive address, say index 0:
   `bcrt1q6rz28mcfaxtmd6v789l9rrlrusdprr9pz3cppk`.
2. **The browser re-derives index 0 from your own descriptor** and compares. If the server
   had sent a different address (a hacked server's own), Osok refuses to show it. Without
   this check, people could pay the attacker while believing they paid you.
3. Someone pays 1 BTC to it. On regtest: `./fund bcrt1q6rz28…`.
4. The server syncs (Bitcoin Core or Esplora), sees an output paying that address's script,
   and your balance shows 1 BTC. That output is now a **UTXO** ("unspent transaction
   output"): a coin you own.

Once an address has been used, Receive moves on to the next index (privacy: don't reuse
addresses).

Our coin, which the rest of the example uses:

```
coin A = txid aaaa…aaaa, output #0, value 1.00000000 BTC (100,000,000 sats)
         paying receive #0 (m/84'/1'/0'/0/0)
```

---

## 6. Sending: the core flow

You want to send **0.3 BTC** to your friend at `bcrt1qfriend…`, at fee priority **Normal**.

### 6.1 The server builds an *unsigned* transaction (a PSBT)

```
POST /wallets/b12e8e68b97d63eb/psbt
{ "address": "bcrt1qfriend…", "amount_sat": 30000000, "fee_priority": "normal" }
```

On the server (`send::build` with BDK):

1. **Fee rate**: ask the backend's estimate for ~6 blocks ("normal"). Say it says
   **2 sat/vB** (on our test networks it's often the 2 sat/vB fallback).
2. **Coin selection**: pick coins that cover 0.3 BTC + fee. Only coin A exists, so coin A.
3. **Size and fee**: 1 SegWit input + 2 outputs ≈ **141 vbytes** → fee = 141 × 2 = **282 sats**.
4. **Change**: 100,000,000 − 30,000,000 − 282 = **69,999,718 sats** must come back to us.
   The server takes the next unused **change** address, say change #4 (`m/84'/1'/0'/1/4` =
   `bcrt1qw3xfnyuspj8qnr2envc448mxwam7f7p249rqs0`).
5. **Reserve coin A for 10 minutes** (section 8).

The result is a **PSBT** (Partially Signed Bitcoin Transaction, BIP174): the transaction
plus extra information a signer needs, but **no signatures**:

```
PSBT
├── unsigned tx
│   ├── input 0:  coin A (aaaa…:0)
│   ├── output 0: 30,000,000 sats → bcrt1qfriend…                         (payment)
│   └── output 1: 69,999,718 sats → bcrt1qw3xfnyuspj8qnr2envc448mxwam7f7p249rqs0  (change)
├── input 0 extra:  the full previous transaction that created coin A
│                   + "this coin belongs to key [73c5da0a/84'/1'/0'/0/0]"
└── output 1 extra: "this output belongs to key [73c5da0a/84'/1'/0'/1/4]"
```

### 6.2 The browser reviews it: don't trust the server

Before you even see the review screen, `reviewSend` runs the **same Rust code as the CLI**
(`wallet-core/src/review.rs`) on that PSBT, inside WASM:

| Check | What it catches |
|---|---|
| Some output pays **exactly 30,000,000 sats to `bcrt1qfriend…`** | the server swapping the recipient or changing the amount |
| **Every other output** is our change (7.3) | the server adding an output that pays itself |
| Each input's value is read from **its full previous transaction**, and that transaction's txid must match | the server lying about how much a coin is worth, to hide a fee |
| fee = inputs − outputs = 100,000,000 − 99,999,718 = **282** | the server's "fee" claim being false |
| **fee guard** (7.4) | absurd fees |

If any check fails, Osok shows the error and there's nothing to sign.

### 6.3 What you see

The review sheet:

```
You are sending           0.3 BTC  →  bcrt1q…friend
Network fee               0.00000282 BTC   ~2.0 sat/vB
Change back to you        0.69999718 BTC
▸ Transaction details
    ✓ Verified in this browser: pays exactly this, the rest comes back to you.
    To                     bcrt1qfriend…
    Amount                 0.30000000 BTC
    Network fee            0.00000282 BTC
    Change                 0.69999718 BTC
    Change address #4      bcrt1qw3xfnyuspj8qnr2envc448mxwam7f7p249rqs0
    Total out              0.30000282 BTC
    Inputs                 1
```

The **"Change address #4"** line is new (`587ce88`), and it's worth understanding why it's
safe to show (7.3).

### 6.4 Sign and broadcast

1. You type your password. WASM decrypts the account key, signs input 0 with the key at
   `m/84'/1'/0'/0/0` (found through the PSBT's "this coin belongs to key …" note), and
   returns the signed PSBT. The key itself never leaves WASM.
2. `POST /wallets/b12e8e68b97d63eb/broadcast { psbt: "<signed>" }`
3. The server finalizes it (puts the signature in place), sends it to Bitcoin Core / Esplora,
   records it in the wallet, and **releases coin A's reservation** (it's spent now, so it no
   longer needs one).
4. → `{ "txid": "bbbb…" }`. Activity now shows **Sent 0.3 BTC**, unconfirmed.

---

## 7. Why the server can't cheat: the attacks the review stops

These are the checks from 6.2, each with the attack it stops. Most of them have a unit test
in `src/send.rs` or `wallet-core`.

### 7.1 Swapped recipient

The server puts its own address in output 0. The review looks for "exactly 30,000,000 sats to
`bcrt1qfriend…`", doesn't find it, and fails:

```
PSBT is missing the payment of 0.3 BTC to …
```

### 7.2 An extra output to the attacker

The server keeps your payment intact but splits your change, sending part of it to
`bcrt1qattacker…`. That output isn't the payment, so it must be our change. It has no
derivation note we can match, so:

```
output 2 pays an address that is neither yours nor the recipient's
```

### 7.3 Fake change (and why "Change address #4" is trustworthy)

A cleverer server **lies in the note**: it pays `bcrt1qattacker…` and writes "this output
belongs to key `[73c5da0a/84'/1'/0'/1/4]`" next to it.

The review doesn't believe the note. It only uses it as a *hint*: it takes the index (4),
derives change #4 **from our own change descriptor**, and compares scripts:

```
claimed index           4
derived from OUR /1/*   bcrt1qw3xfnyuspj8qnr2envc448mxwam7f7p249rqs0
output actually pays    bcrt1qattacker…
→ not equal → "output 1 claims to be change but doesn't pay your change address"
```

When it *does* match, the review keeps the index (`Review.change_indexes = [4]`). The
browser derives the address again from `wallet.internal` at index 4 and shows it. So the
"Change address #4 bcrt1qw3xf…" line in your review is computed from **your** keys, not
copied from the server. That's the point of `587ce88`: you see where *every* sat goes,
including the part coming back.

```
review.rs check_outputs ──[4]──▶ wasm SendReview.changeIndexes ──▶ wallet.ts addressAt(internal, 4)
                                                                         │
                                         review sheet: "Change address #4  bcrt1qw3xf…"  ◀┘
```

### 7.4 Absurd fees (the fee guard)

`check_fee` in `review.rs` blocks a fee that looks like a mistake or an attack, unless you
tick "send anyway":

| Example | Result |
|---|---|
| 0.3 BTC at 2 sat/vB, fee 282 sats | fine |
| any send at **600 sat/vB** | blocked: above 500 sat/vB |
| send 50,000 sats, fee 20,000 sats | blocked: fee is over 10% of the amount **and** over 10,000 sats |
| send 5,000 sats, fee 1,000 sats | fine: 20%, but tiny (under 10,000 sats), normal for small payments |

### 7.5 A lie about a coin's value

With SegWit, a signature commits to the coin's value, but a lying server could still trick a
wallet into misjudging the *fee*. So the review demands the **full previous transaction** of
every input, hashes it, checks the hash equals the input's txid, and reads the value from
there. A lie would change the hash.

---

## 8. Coin reservations (new: they now survive a restart)

### 8.1 The problem

BDK learns a coin is spent only when the spending transaction is broadcast. So:

```
10:00:00  POST /psbt (0.3 BTC)  → server picks coin A → PSBT #1 handed out, not signed yet
10:00:20  POST /psbt (0.1 BTC)  → server picks coin A AGAIN → PSBT #2
10:00:40  broadcast #1 ✔
10:00:50  broadcast #2 ✘ "double-spend": coin A is already spent
```

### 8.2 The fix: reserve coins for 10 minutes

When the server hands out a PSBT, it marks its coins as **reserved** for 10 minutes. The next
`/psbt` skips reserved coins. If none are left, the error says why:

```
insufficient funds … (1 coin(s) are reserved by PSBTs not broadcast yet; they free up once
broadcast or after 10 minutes)
```

Broadcasting releases them early. Never broadcasting lets them expire.

### 8.3 What changed in `007d3db`

Before, reservations lived **only in memory**, so a server restart forgot them:

```
10:00:00  POST /psbt → coin A reserved (in memory)
10:01:00  server restarts (deploy, crash, docker compose up -d …) → memory wiped
10:02:00  POST /psbt → coin A looks free → picked again → double-spend later
```

Now every reservation is **also written to the wallet's SQLite file**:

```sql
CREATE TABLE IF NOT EXISTS coin_reservation (
    txid BLOB NOT NULL,          -- coin A's txid  aaaa…aaaa
    vout INTEGER NOT NULL,       -- 0
    expires_at INTEGER NOT NULL, -- unix seconds, e.g. 1759485600 (= now + 600)
    PRIMARY KEY (txid, vout))
```

Example row after the 10:00:00 `/psbt` (say 10:00:00 is unix time 1759485000):

| txid | vout | expires_at |
|---|---|---|
| `aaaa…aaaa` | 0 | 1759485600 |

On restart, opening the wallet runs `Reservations::load`:

1. Create the table if this wallet is older than the feature (so old wallets upgrade by themselves).
2. Delete rows with `expires_at <= now`. They're expired.
3. Load the rest into memory. Coin A stays reserved until 10:10:00, as before the restart.

**Why unix seconds and not `Instant`?** Rust's `Instant` is a stopwatch that only means
something inside the running program. You can't save "stopwatch reading 1234" and use it after
a restart. Unix seconds (wall-clock time) can be saved.

**But wall clocks can jump.** If the clock is wrongly set back an hour, a row expiring at
10:10 would look 70 minutes away. So a loaded expiry is **capped at now + 10 minutes**: a
reservation can never last longer than the TTL. (Test: `clock_jumping_back_caps_loaded_expiry`.)

**Write order:** SQLite first, then memory. If the database write fails, `/psbt` fails and
no PSBT goes out with coins that weren't reserved. After a broadcast, a failed release only
logs a warning: the transaction is already out, and a leftover row just expires.

**Proof it works:** the regtest test `coin_reservations_survive_a_server_restart` funds a
wallet with one coin, builds a PSBT, restarts the server and asks again. That second request
gets "reserved". Run against the old code, the same test fails.

---

## 9. Speeding up a stuck transaction (RBF)

Our 0.3 BTC payment paid 2 sat/vB and it's sitting unconfirmed. Open it in **Activity** to
reach the **transaction detail screen**, which has a **Speed up** button while it's
unconfirmed.

Every transaction we make signals **replace-by-fee** (BIP125), meaning "I may replace this
with a higher-fee version". Speed up does exactly that:

1. `POST /wallets/b12e8e68b97d63eb/bump { "txid": "bbbb…" }`
2. The server picks the new rate: **max(the "fast" estimate, old rate + 1)**, say
   **3 sat/vB**. New fee ≈ 141 × 3 = **423 sats**. The payment stays the same; the extra
   141 sats come **out of the change**: 69,999,718 → **69,999,577**.
3. It returns the replacement PSBT **and the original transaction**.
4. The browser runs `review_bump`:
   - the original's hash must be `bbbb…` (the server can't fake "the original");
   - the replacement spends **every** input of the original (otherwise it isn't a replacement);
   - it pays **every** original payment unchanged (still exactly 0.3 BTC to your friend);
   - everything else goes to **our** change (the same derive-and-compare as 7.3);
   - the new fee (423) is **higher** than the old one (282).
5. It shows old fee, new fee, change, **the change address (derived in the browser)** and the
   number of inputs. You enter your password, sign, broadcast. The node drops `bbbb…` and
   keeps the replacement `cccc…`.

---

## 10. Looking at a transaction (the detail screen)

`GET /wallets/b12e8e68b97d63eb/transactions/cccc…` returns, for example:

```json
{
  "txid": "cccc…",
  "net_sat": -30000423,
  "fee_sat": 423,
  "fee_rate_sat_vb": 3.0,
  "vsize": 141,
  "rbf": true,
  "confirmed": true, "confirmations": 1, "block_height": 815,
  "time": 1759486800,
  "inputs":  [ { "address": "bcrt1q6rz28…pz3cppk", "value_sat": 100000000, "owner": "receive" } ],
  "outputs": [ { "address": "bcrt1qfriend…",        "value_sat": 30000000,  "owner": "external" },
               { "address": "bcrt1qw3xfnyusp…rqs0", "value_sat": 69999577,  "owner": "change" } ]
}
```

- **net_sat** = what your balance changed by: −(0.3 BTC + fee).
- **time** = the block's time once confirmed; before that, when the node first saw it in the
  mempool. It becomes the date in the Activity list (`3d5cce7`).
- **owner** is decided by the server (BDK's `derivation_of_spk`): a script from our `/0/*` is
  `receive`, from `/1/*` is `change`, anything else is `external`.

This screen is **information only**: it comes from the server and isn't re-verified. That's
acceptable because nothing on it gets signed. The moment you act (Speed up), the WASM review
checks everything again.

---

## 11. Networks and the Docker demo

| Network | What it is | Backend we use |
|---|---|---|
| **regtest** | your own private chain; you mine blocks on demand, coins are free | Bitcoin Core in Docker |
| **testnet4** | the public test network; worthless coins from faucets | Esplora (mempool.space) |
| mainnet | real money | **refused** for now (would also need coin type `0'`) |

The demo (`docker compose up -d`) runs three containers:

```
browser ──▶ web  :3001  nginx serving the static Osok site + the .wasm file
browser ──▶ api  :3000  the watch-only server (regtest, Bitcoin Core backend)
            api  ──RPC──▶ bitcoind (regtest node)
./fund <address>  → sends test coins and mines a block
```

The web container is just files. There's no server-side code that could ever see a secret.
The browser talks to the API directly.

---

## 12. One-page recap

```
12 words ──BIP39──▶ seed ──BIP32──▶ master (73c5da0a) ──m/84'/1'/0'──▶ account key
                                                                         │
                         encrypted with your password (Argon2id + XChaCha20) and stored
                                                                         │
                              public part ▼ (tpub)
            receive  wpkh([73c5da0a/84'/1'/0']tpub…/0/*)   change  …/1/*
                              │ sent to the server (watch-only), wallet id b12e8e68b97d63eb
                              ▼
  server: syncs, shows balance, gives out addresses (browser re-derives them),
          builds UNSIGNED PSBTs, reserves their coins for 10 min (saved in SQLite)
                              │
  browser: review() in WASM ─ exact payment ✓ every other output is OUR change ✓ (shows its address)
                              input values from previous txs ✓  fee guard ✓
                              │
           password → decrypt key inside WASM → sign → server broadcasts → txid
                              │
           stuck? Speed up → review_bump → same payments, higher fee, change still ours
```

| Concept | Where in the code |
|---|---|
| phrase → keys → descriptors | `wallet-core/src/keys.rs` |
| key encryption | `wallet-core/src/crypto.rs` |
| PSBT review, fee guard, change indexes | `wallet-core/src/review.rs` |
| signing | `wallet-core/src/sign.rs` |
| token recovery signature | `wallet-core/src/ownership.rs` |
| browser bridge to all of the above | `wallet-wasm/src/lib.rs`, `web/src/lib/wallet.ts` |
| build / bump / broadcast | `src/send.rs` |
| server, auth, rate limits, reservations | `src/bin/server.rs` |
| history and transaction details | `src/history.rs` |
| sync with Core / Esplora | `src/chain.rs` |
