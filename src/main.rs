use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::{Address, Amount, Denomination, Network, Psbt, Txid};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::Segwitv0;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{AddressInfo, KeychainKind, Wallet};
use bitcoin_wallet::api::{BroadcastRequest, PsbtRequest, RegisterRequest};
use bitcoin_wallet::client::{self, ApiClient, ApiError};
use bitcoin_wallet::{
    chain, default_rpc_url, history, keys, load, parse_network, secret, send, wallet_id,
};
use clap::{Parser, Subcommand};

const MIN_PASSWORD_LEN: usize = 8;

#[derive(Parser)]
#[command(about = "A non-custodial Bitcoin wallet")]
struct Cli {
    /// Where wallet state is stored
    #[arg(long, default_value = "wallet.sqlite")]
    db: PathBuf,

    /// Which chain the wallet lives on: regtest, signet, testnet or testnet4
    #[arg(long, env = "NETWORK", default_value = "regtest", value_parser = parse_network)]
    network: Network,

    /// Bitcoin Core RPC URL [default: localhost on the network's default port]
    #[arg(long, env = "RPC_URL")]
    rpc_url: Option<String>,

    /// Bitcoin Core RPC username
    #[arg(long, env = "RPC_USER", default_value = "wallet")]
    rpc_user: String,

    /// Bitcoin Core RPC password
    #[arg(long, env = "RPC_PASS", default_value = "wallet", hide_default_value = true)]
    rpc_pass: String,

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
    /// Sync with Bitcoin Core
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
        /// Fee rate in sat/vB (default: ask Bitcoin Core for an estimate)
        #[arg(long)]
        fee_rate: Option<u64>,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Let this API server build and broadcast the transaction; we only review and sign.
        /// Without it, everything happens locally against Bitcoin Core.
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
        Command::Send { ref address, ref amount, fee_rate, yes, ref server } => {
            let (to, amount) = parse_payment(address, amount, cli.network)?;
            match server {
                Some(server) => send_via_server(&cli, server, &to, amount, fee_rate, yes),
                None => send_local(&cli, &to, amount, fee_rate, yes),
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
    let birthday = chain::tip_height(&connect(cli)?)?;

    // 128 bits of entropy -> 12 words. Segwitv0 tells BDK which script context the key is for.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English))
            .map_err(|_| anyhow!("failed to generate mnemonic"))?;
    let mnemonic = mnemonic.into_key();

    let password = new_password()?;
    let first = init_wallet(db, cli.network, &mnemonic, &password, birthday)?;

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
        chain::check_birthday(&connect(cli)?, height)?;
    }

    // Read with echo off so the words never appear on screen or in shell history.
    let input = rpassword::prompt_password("Recovery phrase (hidden): ")?;
    let words = input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    // Also validates that every word is in the BIP39 list and the checksum matches.
    let mnemonic = Mnemonic::parse_in(Language::English, words)
        .map_err(|e| anyhow!("invalid recovery phrase: {e}"))?;

    let password = new_password()?;
    init_wallet(db, cli.network, &mnemonic, &password, birthday.unwrap_or(0))?;

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
    let (_, wallet) = load(&cli.db, cli.network)?;
    // Public descriptors only: anyone holding these can watch the wallet, but not spend.
    println!("external: {}", wallet.public_descriptor(KeychainKind::External));
    println!("internal: {}", wallet.public_descriptor(KeychainKind::Internal));
    Ok(())
}

fn register(cli: &Cli, server: &str) -> anyhow::Result<()> {
    let (conn, wallet) = load(&cli.db, cli.network)?;
    // Only public descriptors leave this machine.
    let request = RegisterRequest {
        external: wallet.public_descriptor(KeychainKind::External).to_string(),
        internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
        birthday: chain::birthday(&conn)?,
    };
    let response = match ApiClient::new(server).register(&request) {
        Ok(response) => response,
        Err(e) if status_of(&e) == Some(409) => {
            if client::load_token(&conn, server)?.is_some() {
                println!("Already registered with {server}.");
                return Ok(());
            }
            // The server only hands out a token once, so we can't get it again.
            return Err(e.context(
                "already registered with this server, but this wallet has no token for it \
                 (registered from another copy of the wallet?)",
            ));
        }
        Err(e) => return Err(e),
    };
    client::save_token(&conn, server, &response.token)?;
    println!("Registered with {server} as wallet {}", response.id);
    println!("API token (saved in {}; only needed for calling the API directly):", cli.db.display());
    println!("  {}", response.token);
    Ok(())
}

fn status_of(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<ApiError>().map(|e| e.status)
}

fn address(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    // Sync first so indexes already paid to on chain count as used; otherwise a restored or
    // stale wallet would hand out an address that was already used (address reuse).
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;
    let next = wallet.reveal_next_address(KeychainKind::External);
    wallet.persist(&mut conn).context("saving wallet")?;

    println!("Receive address (index {}): {}", next.index, next.address);
    Ok(())
}

fn addresses(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    // Sync first: "used" means a transaction paying to it has been seen on chain or in the mempool.
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;

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
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    let summary = chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;
    println!(
        "Synced: scanned {} new block(s), tip at height {}",
        summary.blocks_scanned, summary.tip_height
    );
    Ok(())
}

fn balance(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;

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
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;

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
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    let rpc = connect(cli)?;

    let mut last_printed = None;
    loop {
        // Each sync only fetches blocks we haven't seen, so polling like this is cheap.
        chain::sync(&mut wallet, &mut conn, &rpc)?;
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

/// Builds, signs and broadcasts everything locally, talking straight to Bitcoin Core.
fn send_local(
    cli: &Cli,
    to: &Address,
    amount: Amount,
    fee_rate: Option<u64>,
    yes: bool,
) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db, cli.network)?;
    let rpc = connect(cli)?;
    chain::sync(&mut wallet, &mut conn, &rpc)?;

    let fee_rate = send::choose_fee_rate(&rpc, fee_rate)?;
    let draft = send::build(&mut wallet, to, amount, fee_rate, &[])?;
    print_summary(to, amount, draft.fee, draft.change, draft.inputs);

    // Returning here without persisting also discards the change address the builder revealed.
    if !yes && !confirm("Sign and broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let mnemonic = unlock(&conn)?;
    let mut psbt = draft.psbt;
    send::sign(&wallet, &mnemonic, &mut psbt)?;
    let tx = send::finalize(&wallet, psbt)?;
    let txid = send::broadcast(&mut wallet, &mut conn, &rpc, tx)?;

    println!("Broadcast: {txid}");
    println!("Status: unconfirmed. {}", confirm_hint(cli.network));
    Ok(())
}

/// The non-custodial API flow: the server builds an unsigned PSBT, we verify it ourselves,
/// sign it locally, and hand it back for broadcasting. Our keys never leave this machine.
fn send_via_server(
    cli: &Cli,
    server: &str,
    to: &Address,
    amount: Amount,
    fee_rate: Option<u64>,
    yes: bool,
) -> anyhow::Result<()> {
    let (conn, wallet) = load(&cli.db, cli.network)?;
    let token = client::load_token(&conn, server)?.with_context(|| {
        format!("this wallet isn't registered with {server}; run `register --server {server}` first")
    })?;
    let api = ApiClient::new(server).with_token(token);
    let id = wallet_id(
        &wallet.public_descriptor(KeychainKind::External).to_string(),
        &wallet.public_descriptor(KeychainKind::Internal).to_string(),
    );

    let request =
        PsbtRequest { address: to.to_string(), amount_sat: amount.to_sat(), fee_rate_sat_vb: fee_rate };
    let response = api.build_psbt(&id, &request).map_err(|e| match status_of(&e) {
        Some(404) => e.context("the server doesn't know this wallet; run `register` again"),
        Some(401) => e.context("the server rejected this wallet's API token"),
        _ => e,
    })?;
    let mut psbt = Psbt::from_str(&response.psbt).context("server returned an invalid PSBT")?;

    // Don't trust the server's summary: work out what the PSBT really does and refuse
    // anything that isn't exactly the payment we asked for plus our own change.
    let review = send::review(&wallet, &psbt, to, amount).context("refusing to sign")?;
    println!("Verified the server's PSBT: pays exactly the recipient, all other outputs are yours.");
    print_summary(to, amount, review.fee, review.change, review.inputs);

    if !yes && !confirm("Sign and send to the server for broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let mnemonic = unlock(&conn)?;
    send::sign(&wallet, &mnemonic, &mut psbt)?;
    let response = api.broadcast(&id, &BroadcastRequest { psbt: psbt.to_string() })?;

    println!("Broadcast: {}", response.txid);
    println!("Status: unconfirmed. {}", confirm_hint(cli.network));
    Ok(())
}

fn print_summary(to: &Address, amount: Amount, fee: Amount, change: Amount, inputs: usize) {
    println!();
    println!("  To:        {to}");
    println!("  Amount:    {amount}");
    println!("  Fee:       {fee}");
    println!("  Change:    {change}");
    println!("  Inputs:    {inputs}");
    println!("  Total out: {}", amount + fee);
    println!();
}

/// Asks for the wallet password and decrypts the mnemonic. Only needed for signing.
fn unlock(conn: &Connection) -> anyhow::Result<Mnemonic> {
    let password = rpassword::prompt_password("Wallet password: ")?;
    let words = secret::load(conn, &password)?;
    Mnemonic::parse_in(Language::English, words.as_str())
        .map_err(|e| anyhow!("stored mnemonic is invalid: {e}"))
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn connect(cli: &Cli) -> anyhow::Result<chain::Client> {
    let url = cli.rpc_url.clone().unwrap_or_else(|| default_rpc_url(cli.network));
    chain::connect(&url, &cli.rpc_user, &cli.rpc_pass, cli.network)
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

/// Creates the wallet database: BDK's wallet state plus the encrypted mnemonic, written in one
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
        let (external, internal) = keys::descriptors(mnemonic)?;
        let mut conn = Connection::open(db).context("opening wallet database")?;
        let mut tx = conn.transaction()?;

        let mut wallet = Wallet::create(external, internal)
            .network(network)
            .create_wallet(&mut tx)
            .context("creating wallet")?;
        let first = wallet.reveal_next_address(KeychainKind::External);
        // Persist so the revealed index survives restarts (otherwise we'd hand out the same address).
        wallet.persist(&mut tx).context("saving wallet")?;
        secret::save(&tx, &mnemonic.to_string(), password)?;
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
