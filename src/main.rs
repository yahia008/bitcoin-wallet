mod secret;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::bip32::DerivationPath;
use bdk_wallet::bitcoin::secp256k1::Secp256k1;
use bdk_wallet::bitcoin::{Network, NetworkKind};
use bdk_wallet::descriptor;
use bdk_wallet::descriptor::IntoWalletDescriptor;
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::{self, Segwitv0};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{AddressInfo, KeychainKind, Wallet};
use clap::{Parser, Subcommand};

/// Regtest only for now. Changing this also requires changing the coin type (1') below.
const NETWORK: Network = Network::Regtest;

/// BIP84 account path: purpose 84' (native SegWit) / coin type 1' (test networks) / account 0'.
const ACCOUNT_PATH: &str = "m/84h/1h/0h";

const MIN_PASSWORD_LEN: usize = 8;

#[derive(Parser)]
#[command(about = "A non-custodial Bitcoin wallet")]
struct Cli {
    /// Where wallet state is stored
    #[arg(long, default_value = "wallet.sqlite")]
    db: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new wallet from a freshly generated mnemonic
    Create,
    /// Restore a wallet from an existing mnemonic
    Restore,
    /// Reveal the next unused receive address
    Address,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Create => create(&cli.db),
        Command::Restore => restore(&cli.db),
        Command::Address => address(&cli.db),
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

fn address(db: &Path) -> anyhow::Result<()> {
    // Loading needs no password: the database only holds public descriptors.
    let mut conn = open_existing(db)?;
    let mut wallet = Wallet::load()
        .check_network(NETWORK)
        .load_wallet(&mut conn)
        .context("loading wallet")?
        .ok_or_else(|| anyhow!("{} contains no wallet", db.display()))?;

    let next = wallet.reveal_next_address(KeychainKind::External);
    wallet.persist(&mut conn).context("saving wallet")?;

    println!("Receive address (index {}): {}", next.index, next.address);
    Ok(())
}

fn ensure_new(db: &Path) -> anyhow::Result<()> {
    if db.exists() {
        bail!("{} already exists; refusing to overwrite a wallet", db.display());
    }
    Ok(())
}

fn open_existing(db: &Path) -> anyhow::Result<Connection> {
    if !db.exists() {
        bail!("{} not found; run `create` or `restore` first", db.display());
    }
    Connection::open(db).context("opening wallet database")
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
        let (external, internal) = descriptors(mnemonic)?;
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

/// Builds the BIP84 receive (`.../0/*`) and change (`.../1/*`) descriptors from a mnemonic.
/// The returned strings contain private keys (tprv) and must never be logged or stored.
fn descriptors(mnemonic: &Mnemonic) -> anyhow::Result<(String, String)> {
    let external_path = DerivationPath::from_str(&format!("{ACCOUNT_PATH}/0"))?;
    let internal_path = DerivationPath::from_str(&format!("{ACCOUNT_PATH}/1"))?;

    // (mnemonic, None) = no BIP39 passphrase.
    let key = (mnemonic.clone(), None::<String>);
    let secp = Secp256k1::new();

    // NetworkKind::Test makes the keys tprv/tpub, used by testnet, signet and regtest.
    let (ext, ext_keys) = descriptor!(wpkh((key.clone(), external_path)))?
        .into_wallet_descriptor(&secp, NetworkKind::Test)?;
    let (int, int_keys) = descriptor!(wpkh((key, internal_path)))?
        .into_wallet_descriptor(&secp, NetworkKind::Test)?;

    Ok((
        ext.to_string_with_secret(&ext_keys),
        int.to_string_with_secret(&int_keys),
    ))
}
