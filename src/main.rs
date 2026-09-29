use std::path::PathBuf;
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
use bdk_wallet::{KeychainKind, Wallet};
use clap::{Parser, Subcommand};

/// Regtest only for now. Changing this also requires changing the coin type (1') below.
const NETWORK: Network = Network::Regtest;

/// BIP84 account path: purpose 84' (native SegWit) / coin type 1' (test networks) / account 0'.
const ACCOUNT_PATH: &str = "m/84h/1h/0h";

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Create => create(&cli.db),
    }
}

fn create(db: &PathBuf) -> anyhow::Result<()> {
    if db.exists() {
        bail!("{} already exists; refusing to overwrite a wallet", db.display());
    }

    // 128 bits of entropy -> 12 words. Segwitv0 tells BDK which script context the key is for.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English))
            .map_err(|_| anyhow!("failed to generate mnemonic"))?;
    let mnemonic = mnemonic.into_key();

    let (external, internal) = descriptors(&mnemonic)?;

    let mut conn = Connection::open(db).context("opening wallet database")?;
    let mut wallet = Wallet::create(external, internal)
        .network(NETWORK)
        .create_wallet(&mut conn)
        .context("creating wallet")?;

    let first = wallet.reveal_next_address(KeychainKind::External);
    // Persist so the revealed index survives restarts (otherwise we'd hand out the same address).
    wallet.persist(&mut conn).context("saving wallet")?;

    println!("Wallet created: {}", db.display());
    println!();
    println!("Write down your recovery phrase. It is the ONLY way to recover your funds:");
    println!();
    println!("    {mnemonic}");
    println!();
    println!("First receive address (index {}): {}", first.index, first.address);
    Ok(())
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
