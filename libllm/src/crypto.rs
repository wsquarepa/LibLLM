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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_create_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let salt_path = dir.path().join(".salt");

        let salt1 = load_or_create_salt(&salt_path).expect("first call");
        let salt2 = load_or_create_salt(&salt_path).expect("second call");
        assert_eq!(salt1, salt2);
    }

    #[test]
    fn key_determinism() {
        let dir = tempfile::tempdir().unwrap();
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");

        let key1 = derive_key("same-passkey", &salt).expect("key1");
        let key2 = derive_key("same-passkey", &salt).expect("key2");

        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn different_passkeys_differ() {
        let dir = tempfile::tempdir().unwrap();
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");

        let key_a = derive_key("passkey-a", &salt).expect("key_a");
        let key_b = derive_key("passkey-b", &salt).expect("key_b");

        assert_ne!(key_a.as_bytes(), key_b.as_bytes());
    }

    #[test]
    fn hex_returns_lowercase_hex_string() {
        let dir = tempfile::tempdir().unwrap();
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");
        let key = derive_key("test-passkey", &salt).expect("key");

        let hex = key.hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn verify_or_set_key_creates_new_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let check_path = dir.path().join(".key_check");
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");
        let key = derive_key("passkey", &salt).expect("key");

        let result = verify_or_set_key(&check_path, &key).expect("verify_or_set");
        assert!(result);
        assert!(check_path.exists());
    }

    #[test]
    fn verify_or_set_key_accepts_matching_key() {
        let dir = tempfile::tempdir().unwrap();
        let check_path = dir.path().join(".key_check");
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");
        let key = derive_key("passkey", &salt).expect("key");

        verify_or_set_key(&check_path, &key).expect("first call");
        let result = verify_or_set_key(&check_path, &key).expect("second call");
        assert!(result);
    }

    #[test]
    fn verify_or_set_key_rejects_mismatched_key() {
        let dir = tempfile::tempdir().unwrap();
        let check_path = dir.path().join(".key_check");
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");

        let key_a = derive_key("passkey-a", &salt).expect("key_a");
        let key_b = derive_key("passkey-b", &salt).expect("key_b");

        verify_or_set_key(&check_path, &key_a).expect("set with key_a");
        let result = verify_or_set_key(&check_path, &key_b).expect("check with key_b");
        assert!(!result);
    }

    #[test]
    fn set_key_fingerprint_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let check_path = dir.path().join(".key_check");
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).expect("salt");

        let key_a = derive_key("passkey-a", &salt).expect("key_a");
        let key_b = derive_key("passkey-b", &salt).expect("key_b");

        set_key_fingerprint(&check_path, &key_a).expect("write key_a fingerprint");
        set_key_fingerprint(&check_path, &key_b).expect("overwrite with key_b fingerprint");

        let result = verify_or_set_key(&check_path, &key_b).expect("verify key_b");
        assert!(result);
    }
}
