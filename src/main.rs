use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::consensus::encode::deserialize_hex;
use bdk_wallet::bitcoin::{Address, Amount, Denomination, Network, Psbt, Transaction, Txid};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::Segwitv0;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{AddressInfo, KeychainKind, PersistedWallet, Wallet};
use bitcoin_wallet::api::{
    BroadcastRequest, BumpRequest, PsbtRequest, RegisterRequest, TokenRequest,
};
use bitcoin_wallet::client::{self, ApiClient, ApiError};
use bitcoin_wallet::chain::{self, ChainArgs};
use bitcoin_wallet::send::FeePriority;
use bitcoin_wallet::{history, keys, load, secret, send, wallet_id};
use clap::{Parser, Subcommand};
use wallet_core::crypto::MIN_PASSWORD_LEN;
use wallet_core::ownership;


#[derive(Parser)]
#[command(about = "A non-custodial Bitcoin wallet")]
struct Cli {
    /// Where wallet state is stored
    #[arg(long, default_value = "wallet.sqlite")]
    db: PathBuf,

    #[command(flatten)]
    chain: ChainArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new wallet from a freshly generated mnemonic
    Create,
    /// Restore a wallet from an existing mnemonic
    Restore {
        /// Block height the wallet was created at, to skip scanning older blocks. If unsure,
        /// go earlier: coins received before the birthday won't be found. Default: genesis.
        #[arg(long)]
        birthday: Option<u32>,
    },
    /// Print the wallet's public descriptors (safe to share with a watch-only server)
    Export,
    /// Register this wallet (public descriptors only) with a watch-only API server
    Register {
        /// API server URL
        #[arg(long, env = "WALLET_API")]
        server: String,
    },
    /// Reveal a new receive address
    Address,
    /// List revealed receive addresses and whether they have been used
    Addresses,
    /// Sync with the chain (Bitcoin Core or Esplora)
    Sync,
    /// Sync, then show confirmed and unconfirmed balance
    Balance,
    /// Sync, then list wallet transactions, newest first
    History,
    /// Send bitcoin to an address
    Send {
        /// Recipient address
        address: String,
        /// Amount in BTC, e.g. 0.5
        amount: String,
        #[command(flatten)]
        options: SendOptions,
        /// Let this API server build and broadcast the transaction; we only review and sign.
        /// Without it, everything happens locally against the chain backend.
        #[arg(long, env = "WALLET_API")]
        server: Option<String>,
    },
    /// Replace an unconfirmed transaction with one paying a higher fee (RBF). The payments
    /// stay the same; the extra fee comes out of your change.
    Bump {
        /// Transaction id of your unconfirmed transaction
        txid: String,
        #[command(flatten)]
        options: SendOptions,
        /// Let this API server build and broadcast the replacement; we only review and sign.
        #[arg(long, env = "WALLET_API")]
        server: Option<String>,
    },
    /// Show a transaction's confirmation status
    Status {
        /// Transaction id
        txid: String,
        /// Keep polling until the transaction reaches --until confirmations
        #[arg(long)]
        watch: bool,
        /// Confirmations to wait for with --watch
        #[arg(long, default_value_t = 1)]
        until: u32,
        /// Seconds between polls with --watch
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },
}

#[derive(clap::Args)]
struct SendOptions {
    /// Exact fee rate in sat/vB, instead of an estimate
    #[arg(long, conflicts_with = "fee")]
    fee_rate: Option<u64>,
    /// How soon to confirm; the backend estimates a fee rate for it
    /// [default: normal for send, fast for bump]
    #[arg(long, value_enum)]
    fee: Option<FeePriority>,
    /// Skip the confirmation prompt
    #[arg(long)]
    yes: bool,
    /// Sign even if the fee looks like a mistake (over 500 sat/vB, or over 10% of the amount
    /// when the fee is above 10,000 sats)
    #[arg(long)]
    allow_high_fee: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Create => create(&cli),
        Command::Restore { birthday } => restore(&cli, birthday),
        Command::Export => export(&cli),
        Command::Register { ref server } => register(&cli, server),
        Command::Address => address(&cli),
        Command::Addresses => addresses(&cli),
        Command::Sync => sync(&cli),
        Command::Balance => balance(&cli),
        Command::History => history(&cli),
        Command::Send { ref address, ref amount, ref options, ref server } => {
            let (to, amount) = parse_payment(address, amount, cli.chain.network)?;
            match server {
                Some(server) => send_via_server(&cli, server, &to, amount, options),
                None => send_local(&cli, &to, amount, options),
            }
        }
        Command::Bump { ref txid, ref options, ref server } => {
            let txid = Txid::from_str(txid).context("invalid txid")?;
            match server {
                Some(server) => bump_via_server(&cli, server, txid, options),
                None => bump_local(&cli, txid, options),
            }
        }
        Command::Status { ref txid, watch, until, interval } => {
            status(&cli, txid, watch, until, interval)
        }
    }
}

fn create(cli: &Cli) -> anyhow::Result<()> {
    let db = &cli.db;
    ensure_new(db)?;
    // A brand-new wallet can't have been paid before now, so its birthday is the chain tip.
    let birthday = cli.chain.connect()?.tip_height()?;

    // 128 bits of entropy -> 12 words. Segwitv0 tells BDK which script context the key is for.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English))
            .map_err(|_| anyhow!("failed to generate mnemonic"))?;
    let mnemonic = mnemonic.into_key();

    let password = new_password()?;
    let first = init_wallet(db, cli.chain.network, &mnemonic, &password, birthday)?;

    println!("Wallet created: {}", db.display());
    println!();
    println!("Write down your recovery phrase. It is the ONLY way to recover your funds:");
    println!();
    println!("    {mnemonic}");
    println!();
    println!("First receive address (index {}): {}", first.index, first.address);
    println!("Birthday: block {birthday} (note it down too: restoring is faster with it)");
    Ok(())
}

fn restore(cli: &Cli, birthday: Option<u32>) -> anyhow::Result<()> {
    let db = &cli.db;
    ensure_new(db)?;
    if let Some(height) = birthday {
        chain::check_birthday(&cli.chain.connect()?, height)?;
    }

    // Read with echo off so the words never appear on screen or in shell history.
    let input = rpassword::prompt_password("Recovery phrase (hidden): ")?;
    let words = input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    // Also validates that every word is in the BIP39 list and the checksum matches.
    let mnemonic = Mnemonic::parse_in(Language::English, words)
        .map_err(|e| anyhow!("invalid recovery phrase: {e}"))?;

    let password = new_password()?;
    init_wallet(db, cli.chain.network, &mnemonic, &password, birthday.unwrap_or(0))?;

    println!("Wallet restored: {}", db.display());
    if birthday.is_none() {
        println!("No --birthday given: the first sync scans from genesis.");
    }
    // Don't print index 0 here: a restored wallet has likely used it already. `address` syncs
    // first, so it knows which indexes are taken.
    println!("Run `balance` to find past transactions, and `address` for a fresh receive address.");
    Ok(())
}

fn export(cli: &Cli) -> anyhow::Result<()> {
    let (_, wallet) = load(&cli.db, cli.chain.network)?;
    // Public descriptors only: anyone holding these can watch the wallet, but not spend.
    println!("external: {}", wallet.public_descriptor(KeychainKind::External));
    println!("internal: {}", wallet.public_descriptor(KeychainKind::Internal));
    Ok(())
}

fn register(cli: &Cli, server: &str) -> anyhow::Result<()> {
    let (conn, wallet) = load(&cli.db, cli.chain.network)?;
    let external = wallet.public_descriptor(KeychainKind::External).to_string();
    let internal = wallet.public_descriptor(KeychainKind::Internal).to_string();
    let id = wallet_id(&external, &internal);
    // Only public descriptors leave this machine.
    let request = RegisterRequest { external, internal, birthday: chain::birthday(&conn)? };
    let api = ApiClient::new(server);
    let token = match api.register(&request) {
        Ok(response) => {
            println!("Registered with {server} as wallet {}", response.id);
            response.token
        }
        Err(e) if status_of(&e) == Some(409) => {
            if let Some(saved) = client::load_token(&conn, server)?
                && ApiClient::new(server).with_token(saved).token_works(&id)?
            {
                println!("Already registered with {server}.");
                return Ok(());
            }
            recover_token(&conn, &wallet, &api, &id, server)?
        }
        Err(e) => return Err(e),
    };
    client::save_token(&conn, server, &token)?;
    println!("API token (saved in {}; only needed for calling the API directly):", cli.db.display());
    println!("  {token}");
    Ok(())
}

/// The server knows this wallet, but this copy has no working token for it (restored into a
/// new file, registered from the web wallet, or its token was revoked). The server issues a
/// new one to whoever signs its challenge with the wallet's key; the key stays here.
fn recover_token(
    conn: &Connection,
    wallet: &Wallet,
    api: &ApiClient,
    id: &str,
    server: &str,
) -> anyhow::Result<String> {
    println!("{server} already knows this wallet, but this copy has no working API token for it.");
    println!("Proving you own the wallet gets a new token. Other copies of this wallet using that");
    println!("server (e.g. the web wallet) are signed out and will have to do the same.");
    let account_key = unlock(conn, wallet)?;
    let challenge = api.challenge(id)?.challenge;
    let signature = ownership::sign(&account_key, id, &challenge)?;
    let token = api.recover_token(id, &TokenRequest { challenge, signature })?.token;
    println!("Got a new API token; the previous one no longer works.");
    Ok(token)
}

fn status_of(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<ApiError>().map(|e| e.status)
}

fn address(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    // Sync first so indexes already paid to on chain count as used; otherwise a restored or
    // stale wallet would hand out an address that was already used (address reuse).
    chain::sync(&mut wallet, &mut conn, &cli.chain.connect()?)?;
    let next = wallet.reveal_next_address(KeychainKind::External);
    wallet.persist(&mut conn).context("saving wallet")?;

    println!("Receive address (index {}): {}", next.index, next.address);
    Ok(())
}

fn addresses(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    // Sync first: "used" means a transaction paying to it has been seen on chain or in the mempool.
    chain::sync(&mut wallet, &mut conn, &cli.chain.connect()?)?;

    let addresses = history::receive_addresses(&wallet);
    if addresses.is_empty() {
        println!("No receive addresses revealed yet.");
    }
    for a in addresses {
        println!("{:>4}  {}  {}", a.index, a.address, if a.used { "used" } else { "unused" });
    }
    Ok(())
}

fn sync(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    let summary = chain::sync(&mut wallet, &mut conn, &cli.chain.connect()?)?;
    println!(
        "Synced: scanned {}, tip at height {}",
        summary.scanned, summary.tip_height
    );
    Ok(())
}

fn balance(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    chain::sync(&mut wallet, &mut conn, &cli.chain.connect()?)?;

    let b = wallet.balance();
    println!("Confirmed:   {}", b.confirmed);
    // trusted_pending = our own unconfirmed change; untrusted_pending = incoming from others.
    println!("Unconfirmed: {}", b.trusted_pending + b.untrusted_pending);
    if b.immature.to_sat() > 0 {
        println!("Immature:    {} (coinbase, spendable after 100 confirmations)", b.immature);
    }
    println!("Total:       {}", b.total());
    Ok(())
}

fn history(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    chain::sync(&mut wallet, &mut conn, &cli.chain.connect()?)?;

    let txs = history::transactions(&wallet);
    if txs.is_empty() {
        println!("No transactions yet.");
        return Ok(());
    }
    println!("{:<64}  {:>17}  {:>12}  STATUS", "TXID", "AMOUNT", "FEE");
    for tx in txs {
        let fee = tx.fee.map_or("-".to_owned(), |fee| format!("{} sat", fee.to_sat()));
        println!(
            "{}  {:>17}  {:>12}  {}",
            tx.txid,
            format!("{:+.8}", tx.net.to_btc()),
            fee,
            tx.status()
        );
    }
    Ok(())
}

fn status(cli: &Cli, txid: &str, watch: bool, until: u32, interval: u64) -> anyhow::Result<()> {
    let txid = Txid::from_str(txid).context("invalid txid")?;
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    let backend = cli.chain.connect()?;

    let mut last_printed = None;
    loop {
        // Each sync only fetches blocks we haven't seen, so polling like this is cheap.
        chain::sync(&mut wallet, &mut conn, &backend)?;
        let Some(tx) = history::transaction(&wallet, txid) else {
            bail!("{txid} not found: not a wallet transaction, or dropped from the mempool");
        };

        let line = tx.status();
        if last_printed.as_ref() != Some(&line) {
            println!("{line}");
            last_printed = Some(line);
        }
        if !watch || tx.confirmations >= until {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

/// Validates the recipient and amount before anything touches the wallet.
fn parse_payment(
    address: &str,
    amount: &str,
    network: Network,
) -> anyhow::Result<(Address, Amount)> {
    // require_network rejects addresses for other networks, e.g. mainnet on testnet4.
    let to = Address::from_str(address)
        .context("invalid address")?
        .require_network(network)
        .context("address is for a different network")?;
    let amount = Amount::from_str_in(amount, Denomination::Bitcoin).context("invalid amount")?;
    Ok((to, amount))
}

/// Builds, signs and broadcasts everything locally, talking straight to the chain backend.
fn send_local(
    cli: &Cli,
    to: &Address,
    amount: Amount,
    options: &SendOptions,
) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    let backend = cli.chain.connect()?;
    chain::sync(&mut wallet, &mut conn, &backend)?;

    let priority = options.fee.unwrap_or(FeePriority::Normal);
    let fee_rate = send::choose_fee_rate(&backend, options.fee_rate, priority)?;
    let draft = send::build(&mut wallet, to, amount, fee_rate, &[])?;
    print_summary(to, amount, draft.fee, draft.change, &draft.psbt);
    // Returning here or below without persisting also discards the change address the
    // builder revealed.
    check_fee(&draft.psbt, amount, draft.fee, options)?;

    if !options.yes && !confirm("Sign and broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let account_key = unlock(&conn, &wallet)?;
    let mut psbt = draft.psbt;
    send::sign(&wallet, &account_key, &mut psbt)?;
    let tx = send::finalize(&wallet, psbt)?;
    let txid = send::broadcast(&mut wallet, &mut conn, &backend, tx)?;

    println!("Broadcast: {txid}");
    println!("Status: unconfirmed. {}", confirm_hint(cli.chain.network));
    Ok(())
}

/// The non-custodial API flow: the server builds an unsigned PSBT, we verify it ourselves,
/// sign it locally, and hand it back for broadcasting. Our keys never leave this machine.
fn send_via_server(
    cli: &Cli,
    server: &str,
    to: &Address,
    amount: Amount,
    options: &SendOptions,
) -> anyhow::Result<()> {
    let (conn, wallet, api, id) = api_session(cli, server)?;

    let request = PsbtRequest {
        address: to.to_string(),
        amount_sat: amount.to_sat(),
        fee_rate_sat_vb: options.fee_rate,
        fee_priority: options.fee.unwrap_or(FeePriority::Normal),
    };
    let response = api.build_psbt(&id, &request).map_err(explain_api_error)?;
    let mut psbt = Psbt::from_str(&response.psbt).context("server returned an invalid PSBT")?;

    // Don't trust the server's summary: work out what the PSBT really does and refuse
    // anything that isn't exactly the payment we asked for plus our own change.
    let review = send::review(&wallet, &psbt, to, amount).context("refusing to sign")?;
    println!("Verified the server's PSBT: pays exactly the recipient, all other outputs are yours.");
    print_summary(to, amount, review.fee, review.change, &psbt);
    check_fee(&psbt, amount, review.fee, options)?;

    if !options.yes && !confirm("Sign and send to the server for broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let account_key = unlock(&conn, &wallet)?;
    send::sign(&wallet, &account_key, &mut psbt)?;
    let response = api.broadcast(&id, &BroadcastRequest { psbt: psbt.to_string() })?;

    println!("Broadcast: {}", response.txid);
    println!("Status: unconfirmed. {}", confirm_hint(cli.chain.network));
    Ok(())
}

/// Loads the wallet and an API client authenticated for it on `server`.
fn api_session(
    cli: &Cli,
    server: &str,
) -> anyhow::Result<(Connection, PersistedWallet<Connection>, ApiClient, String)> {
    let (conn, wallet) = load(&cli.db, cli.chain.network)?;
    let token = client::load_token(&conn, server)?.with_context(|| {
        format!("this wallet isn't registered with {server}; run `register --server {server}` first")
    })?;
    let api = ApiClient::new(server).with_token(token);
    let id = wallet_id(
        &wallet.public_descriptor(KeychainKind::External).to_string(),
        &wallet.public_descriptor(KeychainKind::Internal).to_string(),
    );
    Ok((conn, wallet, api, id))
}

fn explain_api_error(e: anyhow::Error) -> anyhow::Error {
    match status_of(&e) {
        Some(404) => e.context("the server doesn't know this wallet; run `register` again"),
        Some(401) => e.context(
            "the server rejected this wallet's API token (signed out by another copy of the \
             wallet?); run `register --server URL` again to get a new one",
        ),
        _ => e,
    }
}

/// Replaces our unconfirmed `txid` with a higher-fee copy (RBF), built locally.
fn bump_local(cli: &Cli, txid: Txid, options: &SendOptions) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.chain.network)?;
    let backend = cli.chain.connect()?;
    chain::sync(&mut wallet, &mut conn, &backend)?;

    let priority = options.fee.unwrap_or(FeePriority::Fast);
    let fee_rate =
        send::choose_bump_fee_rate(&backend, &wallet, txid, options.fee_rate, priority)?;
    let original = wallet
        .get_tx(txid)
        .with_context(|| format!("transaction {txid} is not in this wallet"))?
        .tx_node
        .tx;
    let draft = send::bump(&mut wallet, txid, fee_rate, &[])?;
    // We built it ourselves, but the same checks give us the numbers to show.
    let (review, old_fee) = send::review_bump(&wallet, &draft.psbt, &original)?;
    print_bump_summary(&original, old_fee, review.fee, review.change, &draft.psbt);
    check_fee(&draft.psbt, paid(&draft.psbt, review.change), review.fee, options)?;

    if !options.yes && !confirm("Sign and broadcast the replacement?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let account_key = unlock(&conn, &wallet)?;
    let mut psbt = draft.psbt;
    send::sign(&wallet, &account_key, &mut psbt)?;
    let tx = send::finalize(&wallet, psbt)?;
    let new_txid = send::broadcast(&mut wallet, &mut conn, &backend, tx)?;
    println!("Broadcast: {new_txid} (replaces {txid})");
    println!("Status: unconfirmed. {}", confirm_hint(cli.chain.network));
    Ok(())
}

/// The API flow for bump: the server builds the replacement, we check it against the
/// original (which the server sends, and whose txid we verify), then sign locally.
fn bump_via_server(
    cli: &Cli,
    server: &str,
    txid: Txid,
    options: &SendOptions,
) -> anyhow::Result<()> {
    let (conn, wallet, api, id) = api_session(cli, server)?;
    let request = BumpRequest {
        txid: txid.to_string(),
        fee_rate_sat_vb: options.fee_rate,
        fee_priority: options.fee,
    };
    let response = api.bump(&id, &request).map_err(explain_api_error)?;

    // The txid is a hash of the transaction, so a matching txid means the server sent the
    // real original, not a doctored one that would make a malicious "bump" look harmless.
    let original: Transaction = deserialize_hex(&response.original_tx)
        .context("server returned an invalid original transaction")?;
    if original.compute_txid() != txid {
        bail!("server returned a different transaction than {txid}");
    }
    let mut psbt = Psbt::from_str(&response.psbt).context("server returned an invalid PSBT")?;
    let (review, old_fee) =
        send::review_bump(&wallet, &psbt, &original).context("refusing to sign")?;
    println!("Verified the server's replacement: same payments, only the fee and change differ.");
    print_bump_summary(&original, old_fee, review.fee, review.change, &psbt);
    check_fee(&psbt, paid(&psbt, review.change), review.fee, options)?;

    if !options.yes && !confirm("Sign and send the replacement to the server for broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let account_key = unlock(&conn, &wallet)?;
    send::sign(&wallet, &account_key, &mut psbt)?;
    let response = api.broadcast(&id, &BroadcastRequest { psbt: psbt.to_string() })?;
    println!("Broadcast: {} (replaces {txid})", response.txid);
    println!("Status: unconfirmed. {}", confirm_hint(cli.chain.network));
    Ok(())
}

/// What a PSBT pays to others: everything but our change and the fee.
fn paid(psbt: &Psbt, change: Amount) -> Amount {
    psbt.unsigned_tx.output.iter().map(|o| o.value).sum::<Amount>() - change
}

fn print_bump_summary(
    original: &Transaction,
    old_fee: Amount,
    fee: Amount,
    change: Amount,
    psbt: &Psbt,
) {
    // The original is signed, so its size is exact.
    let old_rate = old_fee.to_sat() as f64 / original.vsize() as f64;
    let rate = send::signed_fee_rate(&psbt.unsigned_tx, fee);
    println!();
    println!("  Replacing: {}", original.compute_txid());
    println!("  Payments:  {} (unchanged)", paid(psbt, change));
    println!("  Old fee:   {old_fee} (~{old_rate:.1} sat/vB)");
    println!("  New fee:   {fee} (~{rate:.1} sat/vB)");
    println!("  Change:    {change}");
    println!("  Inputs:    {}", psbt.inputs.len());
    println!();
}

/// Everything here comes from the PSBT itself (or our own review of it), not from the server.
fn print_summary(to: &Address, amount: Amount, fee: Amount, change: Amount, psbt: &Psbt) {
    let rate = send::signed_fee_rate(&psbt.unsigned_tx, fee);
    println!();
    println!("  To:        {to}");
    println!("  Amount:    {amount}");
    println!("  Fee:       {fee} (~{rate:.1} sat/vB)");
    println!("  Change:    {change}");
    println!("  Inputs:    {}", psbt.inputs.len());
    println!("  Total out: {}", amount + fee);
    println!();
}

/// The high-fee guard, unless the user opted out with --allow-high-fee.
fn check_fee(
    psbt: &Psbt,
    amount: Amount,
    fee: Amount,
    options: &SendOptions,
) -> anyhow::Result<()> {
    if options.allow_high_fee {
        return Ok(());
    }
    send::check_fee(&psbt.unsigned_tx, amount, fee)
        .context("refusing to sign: the fee looks like a mistake (pass --allow-high-fee if not)")
}

/// Asks for the wallet password and decrypts the account key. Only needed for signing.
///
/// Wallets created before we stored only the account key hold the whole mnemonic. The first
/// unlock swaps it for the account key, once that key is checked against the wallet.
fn unlock(conn: &Connection, wallet: &Wallet) -> anyhow::Result<String> {
    let password = rpassword::prompt_password("Wallet password: ")?;
    let secret = secret::load(conn, &password)?;
    if keys::is_account_key(&secret) {
        return Ok(secret);
    }

    let mnemonic = Mnemonic::parse_in(Language::English, secret.as_str())
        .map_err(|e| anyhow!("stored mnemonic is invalid: {e}"))?;
    let account_key = keys::account_key(&mnemonic)?;
    send::check_account_key(wallet, &account_key)?;
    secret::replace(conn, &account_key, &password)?;
    println!("Upgraded the wallet file: it now stores this account's key, not your recovery phrase.");
    Ok(account_key)
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// What to do to get a new transaction confirmed.
fn confirm_hint(network: Network) -> &'static str {
    match network {
        Network::Regtest => "Mine a block to confirm it.",
        _ => "It confirms once a miner includes it in a block (about 10 minutes).",
    }
}

fn ensure_new(db: &Path) -> anyhow::Result<()> {
    if db.exists() {
        bail!("{} already exists; refusing to overwrite a wallet", db.display());
    }
    Ok(())
}

/// Prompts for a new password twice and checks they match.
fn new_password() -> anyhow::Result<String> {
    let password = rpassword::prompt_password("New wallet password: ")?;
    if password.chars().count() < MIN_PASSWORD_LEN {
        bail!("password must be at least {MIN_PASSWORD_LEN} characters");
    }
    if rpassword::prompt_password("Repeat password: ")? != password {
        bail!("passwords do not match");
    }
    Ok(password)
}

/// Creates the wallet database: BDK's wallet state plus the encrypted account key, written in one
/// SQLite transaction so we never end up with a wallet whose keys weren't saved.
/// Returns the first receive address.
fn init_wallet(
    db: &Path,
    network: Network,
    mnemonic: &Mnemonic,
    password: &str,
    birthday: u32,
) -> anyhow::Result<AddressInfo> {
    let result = (|| {
        let account_key = keys::account_key(mnemonic)?;
        let (external, internal) = keys::descriptors(&account_key)?;
        let mut conn = Connection::open(db).context("opening wallet database")?;
        let mut tx = conn.transaction()?;

        let mut wallet = Wallet::create(external, internal)
            .network(network)
            .create_wallet(&mut tx)
            .context("creating wallet")?;
        let first = wallet.reveal_next_address(KeychainKind::External);
        // Persist so the revealed index survives restarts (otherwise we'd hand out the same address).
        wallet.persist(&mut tx).context("saving wallet")?;
        // Only the account key: the mnemonic itself is never written to disk.
        secret::save(&tx, &account_key, password)?;
        chain::save_birthday(&tx, birthday)?;

        tx.commit().context("committing wallet")?;
        Ok(first)
    })();

    if result.is_err() {
        // Don't leave a half-written file behind; ensure_new() would then block a retry.
        let _ = std::fs::remove_file(db);
    }
    result
}
