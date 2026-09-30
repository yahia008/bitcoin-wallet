use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::{Address, Amount, Denomination, FeeRate, SignedAmount, Txid};
use bdk_wallet::chain::{ChainPosition, ConfirmationBlockTime};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::Segwitv0;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{AddressInfo, KeychainKind, Wallet};
use bitcoin_wallet::{NETWORK, chain, confirmations, keys, load, secret, send};
use clap::{Parser, Subcommand};

/// Used when Core can't estimate fees yet (always the case on a fresh regtest chain).
const FALLBACK_FEE_RATE_SAT_VB: u64 = 2;

const MIN_PASSWORD_LEN: usize = 8;

#[derive(Parser)]
#[command(about = "A non-custodial Bitcoin wallet")]
struct Cli {
    /// Where wallet state is stored
    #[arg(long, default_value = "wallet.sqlite")]
    db: PathBuf,

    /// Bitcoin Core RPC URL
    #[arg(long, env = "RPC_URL", default_value = "http://127.0.0.1:18443")]
    rpc_url: String,

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
    Restore,
    /// Print the wallet's public descriptors (safe to share with a watch-only server)
    Export,
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
        Command::Create => create(&cli.db),
        Command::Restore => restore(&cli.db),
        Command::Export => export(&cli.db),
        Command::Address => address(&cli.db),
        Command::Addresses => addresses(&cli),
        Command::Sync => sync(&cli),
        Command::Balance => balance(&cli),
        Command::History => history(&cli),
        Command::Send { ref address, ref amount, fee_rate, yes } => {
            send(&cli, address, amount, fee_rate, yes)
        }
        Command::Status { ref txid, watch, until, interval } => {
            status(&cli, txid, watch, until, interval)
        }
    }
}

fn create(db: &Path) -> anyhow::Result<()> {
    ensure_new(db)?;

    // 128 bits of entropy -> 12 words. Segwitv0 tells BDK which script context the key is for.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English))
            .map_err(|_| anyhow!("failed to generate mnemonic"))?;
    let mnemonic = mnemonic.into_key();

    let password = new_password()?;
    let first = init_wallet(db, &mnemonic, &password)?;

    println!("Wallet created: {}", db.display());
    println!();
    println!("Write down your recovery phrase. It is the ONLY way to recover your funds:");
    println!();
    println!("    {mnemonic}");
    println!();
    println!("First receive address (index {}): {}", first.index, first.address);
    Ok(())
}

fn restore(db: &Path) -> anyhow::Result<()> {
    ensure_new(db)?;

    // Read with echo off so the words never appear on screen or in shell history.
    let input = rpassword::prompt_password("Recovery phrase (hidden): ")?;
    let words = input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    // Also validates that every word is in the BIP39 list and the checksum matches.
    let mnemonic = Mnemonic::parse_in(Language::English, words)
        .map_err(|e| anyhow!("invalid recovery phrase: {e}"))?;

    let password = new_password()?;
    let first = init_wallet(db, &mnemonic, &password)?;

    println!("Wallet restored: {}", db.display());
    println!("First receive address (index {}): {}", first.index, first.address);
    Ok(())
}

fn export(db: &Path) -> anyhow::Result<()> {
    let (_, wallet) = load(db)?;
    // Public descriptors only: anyone holding these can watch the wallet, but not spend.
    println!("external: {}", wallet.public_descriptor(KeychainKind::External));
    println!("internal: {}", wallet.public_descriptor(KeychainKind::Internal));
    Ok(())
}

fn address(db: &Path) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(db)?;
    let next = wallet.reveal_next_address(KeychainKind::External);
    wallet.persist(&mut conn).context("saving wallet")?;

    println!("Receive address (index {}): {}", next.index, next.address);
    Ok(())
}

fn addresses(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db)?;
    // Sync first: "used" means a transaction paying to it has been seen on chain or in the mempool.
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;

    let Some(last) = wallet.derivation_index(KeychainKind::External) else {
        println!("No receive addresses revealed yet.");
        return Ok(());
    };
    for index in 0..=last {
        let info = wallet.peek_address(KeychainKind::External, index);
        let used = wallet.spk_index().is_used(KeychainKind::External, index);
        println!("{index:>4}  {}  {}", info.address, if used { "used" } else { "unused" });
    }
    Ok(())
}

fn sync(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db)?;
    let summary = chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;
    println!(
        "Synced: scanned {} new block(s), tip at height {}",
        summary.blocks_scanned, summary.tip_height
    );
    Ok(())
}

fn balance(cli: &Cli) -> anyhow::Result<()> {
    let (mut conn, mut wallet) = load(&cli.db)?;
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
    let (mut conn, mut wallet) = load(&cli.db)?;
    chain::sync(&mut wallet, &mut conn, &connect(cli)?)?;
    let tip = wallet.latest_checkpoint().height();

    let mut txs: Vec<_> = wallet.transactions().collect();
    if txs.is_empty() {
        println!("No transactions yet.");
        return Ok(());
    }
    // Unconfirmed first, then confirmed from newest block to oldest.
    txs.sort_by_key(|tx| match tx.chain_position {
        ChainPosition::Unconfirmed { .. } => (0, 0),
        ChainPosition::Confirmed { anchor, .. } => (1, u32::MAX - anchor.block_id.height),
    });

    println!("{:<64}  {:>17}  {:>12}  STATUS", "TXID", "AMOUNT", "FEE");
    for tx in txs {
        let (sent, received) = wallet.sent_and_received(&tx.tx_node.tx);
        let net = SignedAmount::from_sat(received.to_sat() as i64 - sent.to_sat() as i64);
        // Only show fees we paid. BDK can sometimes compute the fee of an incoming tx too (when
        // the sender spent change from an earlier payment to us), but that fee isn't ours.
        let fee = match wallet.calculate_fee(&tx.tx_node.tx) {
            Ok(fee) if sent.to_sat() > 0 => format!("{} sat", fee.to_sat()),
            _ => "-".to_owned(),
        };
        let status = describe(tip, &tx.chain_position);
        println!(
            "{}  {:>17}  {:>12}  {status}",
            tx.tx_node.txid,
            format!("{:+.8}", net.to_btc()),
            fee
        );
    }
    Ok(())
}

fn status(cli: &Cli, txid: &str, watch: bool, until: u32, interval: u64) -> anyhow::Result<()> {
    let txid = Txid::from_str(txid).context("invalid txid")?;
    let (mut conn, mut wallet) = load(&cli.db)?;
    let rpc = connect(cli)?;

    let mut last_printed = None;
    loop {
        // Each sync only fetches blocks we haven't seen, so polling like this is cheap.
        chain::sync(&mut wallet, &mut conn, &rpc)?;
        let tip = wallet.latest_checkpoint().height();
        // get_tx only returns txs in the wallet's current view of the chain, so a tx that was
        // replaced or evicted from the mempool shows up as missing.
        let Some(tx) = wallet.get_tx(txid) else {
            bail!("{txid} not found: not a wallet transaction, or dropped from the mempool");
        };

        let line = describe(tip, &tx.chain_position);
        if last_printed.as_ref() != Some(&line) {
            println!("{line}");
            last_printed = Some(line);
        }
        if !watch || confirmations(tip, &tx.chain_position) >= until {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn describe(tip: u32, position: &ChainPosition<ConfirmationBlockTime>) -> String {
    match position {
        ChainPosition::Unconfirmed { .. } => "unconfirmed".to_owned(),
        ChainPosition::Confirmed { anchor, .. } => format!(
            "{} conf (block {})",
            confirmations(tip, position),
            anchor.block_id.height
        ),
    }
}

fn send(
    cli: &Cli,
    address: &str,
    amount: &str,
    fee_rate: Option<u64>,
    yes: bool,
) -> anyhow::Result<()> {
    // Validate input before touching the wallet. require_network rejects e.g. mainnet addresses.
    let to = Address::from_str(address)
        .context("invalid address")?
        .require_network(NETWORK)
        .context("address is for a different network")?;
    let amount = Amount::from_str_in(amount, Denomination::Bitcoin).context("invalid amount")?;

    let (mut conn, mut wallet) = load(&cli.db)?;
    let rpc = connect(cli)?;
    chain::sync(&mut wallet, &mut conn, &rpc)?;

    let fee_rate = match fee_rate {
        Some(rate) => FeeRate::from_sat_per_vb(rate).context("fee rate too large")?,
        None => send::estimate_fee_rate(&rpc).unwrap_or_else(|| {
            println!("Node has no fee estimate yet; using {FALLBACK_FEE_RATE_SAT_VB} sat/vB.");
            FeeRate::from_sat_per_kwu(FALLBACK_FEE_RATE_SAT_VB * 250)
        }),
    };

    let draft = send::build(&mut wallet, &to, amount, fee_rate)?;
    println!();
    println!("  To:        {to}");
    println!("  Amount:    {amount}");
    println!("  Fee:       {} ({} sat/vB)", draft.fee, fee_rate.to_sat_per_vb_ceil());
    println!("  Change:    {}", draft.change);
    println!("  Inputs:    {}", draft.inputs);
    println!("  Total out: {}", amount + draft.fee);
    println!();

    // Returning here without persisting also discards the change address the builder revealed.
    if !yes && !confirm("Sign and broadcast?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let password = rpassword::prompt_password("Wallet password: ")?;
    let words = secret::load(&conn, &password)?;
    let mnemonic = Mnemonic::parse_in(Language::English, words.as_str())
        .map_err(|e| anyhow!("stored mnemonic is invalid: {e}"))?;

    let tx = send::sign(&wallet, &mnemonic, draft.psbt)?;
    let txid = send::broadcast(&mut wallet, &mut conn, &rpc, tx)?;

    println!("Broadcast: {txid}");
    println!("Status: unconfirmed. Mine a block on regtest to confirm it.");
    Ok(())
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn connect(cli: &Cli) -> anyhow::Result<chain::Client> {
    chain::connect(&cli.rpc_url, &cli.rpc_user, &cli.rpc_pass)
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
fn init_wallet(db: &Path, mnemonic: &Mnemonic, password: &str) -> anyhow::Result<AddressInfo> {
    let result = (|| {
        let (external, internal) = keys::descriptors(mnemonic)?;
        let mut conn = Connection::open(db).context("opening wallet database")?;
        let mut tx = conn.transaction()?;

        let mut wallet = Wallet::create(external, internal)
            .network(NETWORK)
            .create_wallet(&mut tx)
            .context("creating wallet")?;
        let first = wallet.reveal_next_address(KeychainKind::External);
        // Persist so the revealed index survives restarts (otherwise we'd hand out the same address).
        wallet.persist(&mut tx).context("saving wallet")?;
        secret::save(&tx, &mnemonic.to_string(), password)?;

        tx.commit().context("committing wallet")?;
        Ok(first)
    })();

    if result.is_err() {
        // Don't leave a half-written file behind; ensure_new() would then block a retry.
        let _ = std::fs::remove_file(db);
    }
    result
}
