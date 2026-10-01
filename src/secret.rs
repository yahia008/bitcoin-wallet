//! Where the CLI keeps the wallet's encrypted signing secret: the account key, or the whole
//! mnemonic in wallets created before that change. The encryption itself is
//! `wallet_core::crypto` (shared with the browser); this stores its salt, nonce and
//! ciphertext in a `secret` table next to BDK's own tables in the wallet database.

use anyhow::{Context, bail};
use bdk_wallet::rusqlite::{Connection, params};
use wallet_core::crypto::{self, Encrypted};

/// Encrypts `secret` under `password` and stores it. Fails if a secret is already stored.
pub fn save(conn: &Connection, secret: &str, password: &str) -> anyhow::Result<()> {
    let Encrypted { salt, nonce, ciphertext } = crypto::encrypt(secret, password)?;

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
    .context("storing encrypted secret")?;
    Ok(())
}

/// Overwrites the stored secret with `secret`, encrypted under `password`.
pub fn replace(conn: &Connection, secret: &str, password: &str) -> anyhow::Result<()> {
    let Encrypted { salt, nonce, ciphertext } = crypto::encrypt(secret, password)?;
    let updated = conn
        .execute(
            "UPDATE secret SET salt = ?1, nonce = ?2, ciphertext = ?3 WHERE id = 0",
            params![&salt[..], &nonce[..], ciphertext],
        )
        .context("replacing encrypted secret")?;
    if updated != 1 {
        bail!("wallet has no encrypted secret to replace");
    }
    Ok(())
}

/// Decrypts the stored secret. A wrong password fails the Poly1305 authentication check.
pub fn load(conn: &Connection, password: &str) -> anyhow::Result<String> {
    let encrypted = conn
        .query_row(
            "SELECT salt, nonce, ciphertext FROM secret WHERE id = 0",
            [],
            |row| Ok(Encrypted { salt: row.get(0)?, nonce: row.get(1)?, ciphertext: row.get(2)? }),
        )
        .context("wallet has no encrypted secret")?;
    crypto::decrypt(&encrypted, password)
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
    fn replace_overwrites() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "correct horse").unwrap();
        replace(&conn, "new secret", "correct horse").unwrap();
        assert_eq!(load(&conn, "correct horse").unwrap(), "new secret");
    }

    #[test]
    fn second_save_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        save(&conn, WORDS, "one").unwrap();
        assert!(save(&conn, WORDS, "two").is_err());
    }
}
