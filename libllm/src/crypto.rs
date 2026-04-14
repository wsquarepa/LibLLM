use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use argon2::Argon2;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

const SALT_LEN: usize = 16;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    bytes: [u8; 32],
}

impl DerivedKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    pub fn hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

pub fn load_or_create_salt(path: &Path) -> Result<[u8; SALT_LEN]> {
    match std::fs::read(path) {
        Ok(data) => {
            if data.len() != SALT_LEN {
                bail!(
                    "invalid salt file length for {}: expected {SALT_LEN} bytes, got {}",
                    path.display(),
                    data.len()
                );
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&data);
            return Ok(salt);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).context(format!("failed to read salt file: {}", path.display()));
        }
    }

    let salt: [u8; SALT_LEN] = rand::random();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("failed to create directory for salt file")?;
    }
    std::fs::write(path, salt).context("failed to write salt file")?;
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

fn temp_write_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .context(format!("path has no file name: {}", path.display()))?
        .to_string_lossy();
    Ok(parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4())))
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let temp_path = temp_write_path(path)?;

    let write_result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(&temp_path).context(format!(
            "failed to create temp file: {}",
            temp_path.display()
        ))?;
        file.write_all(data).context(format!(
            "failed to write temp file: {}",
            temp_path.display()
        ))?;
        file.sync_all()
            .context(format!("failed to sync temp file: {}", temp_path.display()))?;
        drop(file);

        std::fs::rename(&temp_path, path).context(format!(
            "failed to replace file atomically: {}",
            path.display()
        ))
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }

    write_result
}

pub fn verify_or_set_key(check_path: &Path, key: &DerivedKey) -> Result<bool> {
    let fingerprint = key_fingerprint(key);

    match std::fs::read(check_path) {
        Ok(stored) => {
            if stored.len() != KEY_CHECK_LEN {
                bail!(
                    "invalid key check file length for {}: expected {KEY_CHECK_LEN} bytes, got {}",
                    check_path.display(),
                    stored.len()
                );
            }
            return Ok(stored == fingerprint);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).context(format!(
                "failed to read key check file: {}",
                check_path.display()
            ));
        }
    }

    if let Some(parent) = check_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create directory for key check file")?;
    }
    std::fs::write(check_path, fingerprint).context("failed to write key check file")?;
    Ok(true)
}

pub fn set_key_fingerprint(check_path: &Path, key: &DerivedKey) -> Result<()> {
    let fingerprint = key_fingerprint(key);
    if let Some(parent) = check_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create directory for key check file")?;
    }
    std::fs::write(check_path, fingerprint).context("failed to write key check file")
}
