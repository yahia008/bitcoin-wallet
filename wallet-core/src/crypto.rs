//! Password encryption for the wallet's signing secret (the account key, see keys.rs). The
//! CLI stores the result in SQLite, the browser in IndexedDB; both use this same code.
//!
//! XChaCha20-Poly1305 under a key derived from the password with Argon2id. Only the salt,
//! nonce and ciphertext are kept.

use anyhow::{Context, anyhow};
use argon2::Argon2;
use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

const SALT_LEN: usize = 16;

/// Shortest password accepted when creating a wallet (CLI and browser).
pub const MIN_PASSWORD_LEN: usize = 8;

/// A secret encrypted under a password.
pub struct Encrypted {
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Stretches the password into a 256-bit encryption key. Argon2id is deliberately slow and
/// memory-hard, so each password guess costs an attacker real time and RAM.
fn derive_key(password: &str, salt: &[u8]) -> anyhow::Result<Key> {
    let mut key = Key::default();
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

pub fn encrypt(secret: &str, password: &str) -> anyhow::Result<Encrypted> {
    // Fresh random salt and nonce every time: the same password never yields the same key,
    // and a key/nonce pair is never reused.
    let salt = <[u8; SALT_LEN]>::generate();
    let nonce = XNonce::generate();

    let cipher = XChaCha20Poly1305::new(&derive_key(password, &salt)?);
    let ciphertext = cipher
        .encrypt(&nonce, secret.as_bytes())
        .map_err(|_| anyhow!("encryption failed"))?;
    Ok(Encrypted { salt: salt.to_vec(), nonce: nonce.to_vec(), ciphertext })
}

/// A wrong password fails the Poly1305 authentication check.
pub fn decrypt(encrypted: &Encrypted, password: &str) -> anyhow::Result<String> {
    let nonce = XNonce::try_from(&encrypted.nonce[..])
        .map_err(|_| anyhow!("stored nonce is corrupt"))?;
    let cipher = XChaCha20Poly1305::new(&derive_key(password, &encrypted.salt)?);
    let plaintext = cipher
        .decrypt(&nonce, encrypted.ciphertext.as_ref())
        .map_err(|_| anyhow!("wrong password, or the encrypted key is corrupted"))?;
    String::from_utf8(plaintext).context("decrypted secret is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "abandon abandon abandon abandon abandon abandon \
                          abandon abandon abandon abandon abandon about";

    #[test]
    fn round_trip() {
        let encrypted = encrypt(SECRET, "correct horse").unwrap();
        assert_eq!(decrypt(&encrypted, "correct horse").unwrap(), SECRET);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let encrypted = encrypt(SECRET, "correct horse").unwrap();
        assert!(decrypt(&encrypted, "battery staple").is_err());
    }

    #[test]
    fn plaintext_is_not_stored() {
        let encrypted = encrypt(SECRET, "correct horse").unwrap();
        assert!(!encrypted.ciphertext.windows(7).any(|w| w == b"abandon"));
    }

    #[test]
    fn fresh_salt_and_nonce_each_time() {
        let (a, b) = (encrypt(SECRET, "pw").unwrap(), encrypt(SECRET, "pw").unwrap());
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.nonce, b.nonce);
    }
}
