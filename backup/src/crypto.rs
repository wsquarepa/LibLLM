//! Backup-specific encryption using XChaCha20-Poly1305 with Argon2id key derivation.

use std::path::Path;

use anyhow::{Result, bail};
use argon2::Argon2;
use chacha20poly1305::{
    XChaCha20Poly1305,
    aead::{Aead, NewAead},
};
use rand::RngCore;

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const MIN_CIPHERTEXT_LEN: usize = NONCE_LEN + TAG_LEN;
const BACKUP_CONTEXT: &str = "libllm-backup-v1";

/// Derives a backup-specific encryption key from a passkey and salt.
///
/// Combines the salt with the context string "libllm-backup-v1" via blake3 to produce
/// a domain-separated 16-byte salt, then runs Argon2id with memory=65536, iterations=3,
/// parallelism=1, output=32. The resulting key is intentionally distinct from the DB key
/// produced by `libllm::crypto::derive_key` even when given the same passkey and salt.
pub fn derive_backup_key(passkey: &str, salt: &[u8; 16]) -> Result<[u8; 32]> {
    let derived_salt: [u8; 16] = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(salt);
        hasher.update(BACKUP_CONTEXT.as_bytes());
        let hash = hasher.finalize();
        let mut out = [0u8; 16];
        out.copy_from_slice(&hash.as_bytes()[..16]);
        out
    };

    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("invalid argon2 params: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut key_bytes = [0u8; 32];
    argon2
        .hash_password_into(passkey.as_bytes(), &derived_salt, &mut key_bytes)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;

    Ok(key_bytes)
}

/// Resolves a backup encryption key from data_dir and an optional passkey.
///
/// Loads (or creates) the salt from `data_dir/.salt`, then derives the backup key via
/// `derive_backup_key`. Returns `None` when `passkey` is `None`.
pub fn resolve_backup_key(data_dir: &Path, passkey: Option<&str>) -> Result<Option<[u8; 32]>> {
    match passkey {
        Some(pk) => {
            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            Ok(Some(derive_backup_key(pk, &salt)?))
        }
        None => Ok(None),
    }
}

/// Encrypts plaintext with XChaCha20-Poly1305.
///
/// Output format: [24-byte nonce][ciphertext + 16-byte Poly1305 tag].
pub fn encrypt_payload(plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = chacha20poly1305::XNonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

    let mut output = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    output.extend_from_slice(nonce_bytes.as_ref());
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypts a payload produced by `encrypt_payload`.
///
/// Expects at least 24 + 16 bytes (nonce + tag). Returns an error if the data is too
/// short or if authentication fails.
pub fn decrypt_payload(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    if data.len() < MIN_CIPHERTEXT_LEN {
        bail!(
            "ciphertext too short: expected at least {MIN_CIPHERTEXT_LEN} bytes, got {}",
            data.len()
        );
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let nonce = chacha20poly1305::XNonce::from_slice(nonce_bytes);
    let cipher = XChaCha20Poly1305::new(key.into());

    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PASSKEY: &str = "test-passkey-12345";
    const TEST_SALT: &[u8; 16] = b"0123456789abcdef";

    #[test]
    fn derive_backup_key_is_deterministic() {
        let key1 = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key1");
        let key2 = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key2");
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_backup_key_differs_from_db_key() {
        let backup_key = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("backup key");
        let db_key = libllm::crypto::derive_key(TEST_PASSKEY, TEST_SALT).expect("db key");
        assert_ne!(backup_key, *db_key.as_bytes());
    }

    #[test]
    fn encrypt_then_decrypt_round_trip() {
        let key = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key");
        let plaintext = b"hello, backup world!";

        let ciphertext = encrypt_payload(plaintext, &key).expect("encrypt");
        let decrypted = decrypt_payload(&ciphertext, &key).expect("decrypt");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key");
        let wrong_key = derive_backup_key("wrong-passkey", TEST_SALT).expect("wrong key");

        let ciphertext = encrypt_payload(b"secret data", &key).expect("encrypt");
        let result = decrypt_payload(&ciphertext, &wrong_key);

        assert!(result.is_err());
    }

    #[test]
    fn decrypt_truncated_ciphertext_fails() {
        let key = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key");
        let ciphertext = encrypt_payload(b"some data", &key).expect("encrypt");

        // Truncate to just the nonce — drops the tag entirely
        let truncated = &ciphertext[..NONCE_LEN];
        let result = decrypt_payload(truncated, &key);

        assert!(result.is_err());
    }

    #[test]
    fn ciphertext_format_starts_with_24_byte_nonce() {
        let key = derive_backup_key(TEST_PASSKEY, TEST_SALT).expect("key");
        let plaintext = b"verify format";

        let ciphertext = encrypt_payload(plaintext, &key).expect("encrypt");

        assert!(ciphertext.len() >= NONCE_LEN + TAG_LEN + plaintext.len());
    }
}
