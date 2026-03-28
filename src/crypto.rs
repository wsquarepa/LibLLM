use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, AeadCore, Nonce};
use anyhow::{Context, Result, bail};
use argon2::Argon2;
use sha2::{Sha256, Digest};
use zeroize::{Zeroize, ZeroizeOnDrop};

const EXT_PLAINTEXT: &str = "json";

const MAGIC: &[u8; 4] = b"LLMS";
const VERSION: u8 = 0x01;
const HEADER_LEN: usize = 4 + 1 + 12;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    bytes: [u8; 32],
}

impl DerivedKey {
    fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

pub fn load_or_create_salt(path: &Path) -> Result<[u8; SALT_LEN]> {
    if let Ok(data) = std::fs::read(path) {
        if data.len() == SALT_LEN {
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&data);
            return Ok(salt);
        }
    }

    let salt: [u8; SALT_LEN] = rand::random();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .context("failed to create directory for salt file")?;
    }
    std::fs::write(path, salt)
        .context("failed to write salt file")?;
    Ok(salt)
}

pub fn derive_key(passkey: &str, salt: &[u8; SALT_LEN]) -> Result<DerivedKey> {
    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("invalid argon2 params: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut key_bytes = [0u8; 32];
    argon2
        .hash_password_into(passkey.as_bytes(), salt, &mut key_bytes)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;

    Ok(DerivedKey { bytes: key_bytes })
}

pub fn encrypt(plaintext: &[u8], key: &DerivedKey) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| anyhow::anyhow!("cipher init failed: {e}"))?;

    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

    let mut blob = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.push(VERSION);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

pub fn decrypt(blob: &[u8], key: &DerivedKey) -> Result<Vec<u8>> {
    if blob.len() < HEADER_LEN {
        bail!("encrypted file too short");
    }
    if &blob[0..4] != MAGIC {
        bail!("not an encrypted session file (invalid magic)");
    }
    if blob[4] != VERSION {
        bail!("unsupported encryption format version: {}", blob[4]);
    }

    let nonce_bytes: &[u8; NONCE_LEN] = blob[5..5 + NONCE_LEN]
        .try_into()
        .context("invalid nonce length")?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let ciphertext = &blob[HEADER_LEN..];

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| anyhow::anyhow!("cipher init failed: {e}"))?;

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed -- wrong passkey?"))
}

pub fn is_encrypted(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN && data[0..4] == *MAGIC
}

const KEY_CHECK_LEN: usize = 32;

fn key_fingerprint(key: &DerivedKey) -> [u8; KEY_CHECK_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(b"libllm-key-check");
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; KEY_CHECK_LEN];
    out.copy_from_slice(&result);
    out
}

pub fn resolve_encrypted_path(dir: &Path, slug: &str, encrypted_ext: &str) -> PathBuf {
    let encrypted = dir.join(format!("{slug}.{encrypted_ext}"));
    if encrypted.exists() {
        return encrypted;
    }
    dir.join(format!("{slug}.{EXT_PLAINTEXT}"))
}

pub fn encrypted_extension<'a>(key: Option<&DerivedKey>, encrypted_ext: &'a str) -> &'a str {
    if key.is_some() { encrypted_ext } else { EXT_PLAINTEXT }
}

pub fn read_and_decrypt(path: &Path, key: Option<&DerivedKey>) -> Result<String> {
    let raw = std::fs::read(path)
        .context(format!("failed to read file: {}", path.display()))?;
    if is_encrypted(&raw) {
        let key = key.ok_or_else(|| anyhow::anyhow!("file is encrypted but no passkey available: {}", path.display()))?;
        let decrypted = decrypt(&raw, key)?;
        String::from_utf8(decrypted).context("decrypted content is not valid UTF-8")
    } else {
        String::from_utf8(raw).context("file content is not valid UTF-8")
    }
}

pub fn encrypt_and_write(path: &Path, plaintext: &[u8], key: Option<&DerivedKey>) -> Result<()> {
    let data = match key {
        Some(k) => encrypt(plaintext, k)?,
        None => plaintext.to_vec(),
    };
    std::fs::write(path, data).context(format!("failed to write file: {}", path.display()))
}

pub fn verify_or_set_key(check_path: &Path, key: &DerivedKey) -> Result<bool> {
    let fingerprint = key_fingerprint(key);

    if let Ok(stored) = std::fs::read(check_path) {
        if stored.len() == KEY_CHECK_LEN {
            return Ok(stored == fingerprint);
        }
    }

    if let Some(parent) = check_path.parent() {
        std::fs::create_dir_all(parent)
            .context("failed to create directory for key check file")?;
    }
    std::fs::write(check_path, fingerprint)
        .context("failed to write key check file")?;
    Ok(true)
}

pub fn set_key_fingerprint(check_path: &Path, key: &DerivedKey) -> Result<()> {
    let fingerprint = key_fingerprint(key);
    if let Some(parent) = check_path.parent() {
        std::fs::create_dir_all(parent)
            .context("failed to create directory for key check file")?;
    }
    std::fs::write(check_path, fingerprint)
        .context("failed to write key check file")
}

pub fn re_encrypt_file(path: &Path, old_key: &DerivedKey, new_key: &DerivedKey) -> Result<()> {
    let raw = std::fs::read(path)
        .context(format!("failed to read file: {}", path.display()))?;
    if !is_encrypted(&raw) {
        return Ok(());
    }
    let plaintext = decrypt(&raw, old_key)?;
    let new_blob = encrypt(&plaintext, new_key)?;
    std::fs::write(path, new_blob)
        .context(format!("failed to write file: {}", path.display()))
}

pub fn re_encrypt_directory(dir: &Path, extensions: &[&str], old_key: &DerivedKey, new_key: &DerivedKey) -> Vec<String> {
    let mut warnings = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("{}: {e}", dir.display()));
            return warnings;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matches = path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| extensions.contains(&ext));
        if !matches {
            continue;
        }
        if let Err(e) = re_encrypt_file(&path, old_key, new_key) {
            warnings.push(format!("{}: {e}", path.display()));
        }
    }
    warnings
}
