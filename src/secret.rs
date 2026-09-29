//! Password-encrypted storage for the wallet mnemonic.
//!
//! The mnemonic is encrypted with XChaCha20-Poly1305 under a key derived from the user's
//! password with Argon2id. Only the salt, nonce and ciphertext are stored, in a `secret`
//! table next to BDK's own tables in the wallet database.

use anyhow::{Context, anyhow};
use argon2::Argon2;
use bdk_wallet::rusqlite::{Connection, params};
use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

const SALT_LEN: usize = 16;

/// Stretches the password into a 256-bit encryption key. Argon2id is deliberately slow and
/// memory-hard, so each password guess costs an attacker real time and RAM.
fn derive_key(password: &str, salt: &[u8]) -> anyhow::Result<Key> {
    let mut key = Key::default();
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

/// Encrypts `mnemonic` under `password` and stores it. Fails if a secret is already stored.
pub fn save(conn: &Connection, mnemonic: &str, password: &str) -> anyhow::Result<()> {
    // Fresh random salt and nonce every time: the same password never yields the same key,
    // and a key/nonce pair is never reused.
    let salt = <[u8; SALT_LEN]>::generate();
    let nonce = XNonce::generate();

    let cipher = XChaCha20Poly1305::new(&derive_key(password, &salt)?);
    let ciphertext = cipher
        .encrypt(&nonce, mnemonic.as_bytes())
        .map_err(|_| anyhow!("encryption failed"))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS secret (
            id         INTEGER PRIMARY KEY CHECK (id = 0),
            salt       BLOB NOT NULL,
            nonce      BLOB NOT NULL,
            ciphertext BLOB NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "INSERT INTO secret (id, salt, nonce, ciphertext) VALUES (0, ?1, ?2, ?3)",
        params![&salt[..], &nonce[..], ciphertext],
    )
    .context("storing encrypted mnemonic")?;
    Ok(())
}

/// Decrypts the stored mnemonic. A wrong password fails the Poly1305 authentication check.
pub fn load(conn: &Connection, password: &str) -> anyhow::Result<String> {
    let (salt, nonce, ciphertext): (Vec<u8>, Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT salt, nonce, ciphertext FROM secret WHERE id = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("wallet has no encrypted mnemonic")?;

    let nonce = XNonce::try_from(&nonce[..]).map_err(|_| anyhow!("stored nonce is corrupt"))?;
    let cipher = XChaCha20Poly1305::new(&derive_key(password, &salt)?);
    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| anyhow!("wrong password, or the wallet file is corrupted"))?;
    String::from_utf8(plaintext).context("decrypted mnemonic is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: &str = "abandon abandon abandon abandon abandon abandon \
                         abandon abandon abandon abandon abandon about";

    #[test]
    fn round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "correct horse").unwrap();
        assert_eq!(load(&conn, "correct horse").unwrap(), WORDS);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "correct horse").unwrap();
        assert!(load(&conn, "battery staple").is_err());
    }

    #[test]
    fn plaintext_is_not_stored() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "correct horse").unwrap();
        let ciphertext: Vec<u8> = conn
            .query_row("SELECT ciphertext FROM secret", [], |r| r.get(0))
            .unwrap();
        assert!(!ciphertext.windows(7).any(|w| w == b"abandon"));
    }

    #[test]
    fn second_save_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "one").unwrap();
        assert!(save(&conn, WORDS, "two").is_err());
    }
}
