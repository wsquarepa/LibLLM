//! Database encryption via SQLCipher with Argon2id key derivation and atomic file writes.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use argon2::Argon2;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SALT_LEN: usize = 16;

#[cfg(not(any(test, feature = "test-support")))]
const ARGON2_MEM_KIB: u32 = 65536;
#[cfg(not(any(test, feature = "test-support")))]
const ARGON2_ITERATIONS: u32 = 3;

#[cfg(any(test, feature = "test-support"))]
const ARGON2_MEM_KIB: u32 = 8;
#[cfg(any(test, feature = "test-support"))]
const ARGON2_ITERATIONS: u32 = 1;

const ARGON2_PARALLELISM: u32 = 1;
const ARGON2_OUTPUT_LEN: usize = 32;

/// Tightens the permissions on `path` to 0600 (owner read/write only).
///
/// On non-Unix platforms this is a no-op that always returns `Ok(())`.
pub fn chmod_0600(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Returns the Argon2id parameters shared by every LibLLM key derivation.
///
/// Production: `m_cost=65536 KiB, t_cost=3, p_cost=1, output=32`. Under `cfg(test)`
/// or the `test-support` feature, reduced to `m_cost=8, t_cost=1, p_cost=1` so the
/// test suite does not pay a multi-second KDF per encrypted-database open.
pub fn argon2_params() -> argon2::Params {
    argon2::Params::new(
        ARGON2_MEM_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(ARGON2_OUTPUT_LEN),
    )
    .expect("argon2 parameters are valid by construction")
}

fn write_restricted(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to write: {}", path.display()))?;
    file.write_all(data)
        .with_context(|| format!("failed to write: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync: {}", path.display()))?;
    Ok(())
}

/// A 32-byte encryption key derived from a passkey via Argon2id, automatically zeroed on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    bytes: [u8; 32],
}

impl DerivedKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    pub fn hex(&self) -> Zeroizing<String> {
        Zeroizing::new(self.bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// Reads an existing 16-byte salt from `path`, or generates a cryptographically random one and writes it.
///
/// Returns an error if the file exists but has the wrong length, or if I/O fails.
pub fn load_or_create_salt(path: &Path) -> Result<[u8; SALT_LEN]> {
    match std::fs::read(path) {
        Ok(data) => {
            if data.len() != SALT_LEN {
                tracing::error!(
                    phase = "load",
                    result = "error",
                    reason = "invalid_length",
                    path = %path.display(),
                    bytes = data.len(),
                    expected = SALT_LEN,
                    "crypto.salt",
                );
                bail!(
                    "invalid salt file length for {}: expected {SALT_LEN} bytes, got {}",
                    path.display(),
                    data.len()
                );
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&data);
            tracing::info!(
                phase = "load",
                result = "ok",
                path = %path.display(),
                bytes = SALT_LEN,
                "crypto.salt",
            );
            return Ok(salt);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::error!(
                phase = "load",
                result = "error",
                path = %path.display(),
                error = %err,
                "crypto.salt",
            );
            return Err(err).context(format!("failed to read salt file: {}", path.display()));
        }
    }

    let salt: [u8; SALT_LEN] = rand::random();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("failed to create directory for salt file")?;
    }
    match write_restricted(path, &salt) {
        Ok(()) => {
            tracing::info!(
                phase = "create",
                result = "ok",
                path = %path.display(),
                "crypto.salt",
            );
            Ok(salt)
        }
        Err(err) => {
            tracing::error!(
                phase = "create",
                result = "error",
                path = %path.display(),
                error = %err,
                "crypto.salt",
            );
            Err(err)
        }
    }
}

/// Derives a 32-byte database encryption key from a passkey and 16-byte salt using Argon2id.
///
/// Parameters come from [`argon2_params`]; see its docs for production vs. test values.
pub fn derive_key(passkey: &str, salt: &[u8; SALT_LEN]) -> Result<DerivedKey> {
    crate::timed_result!(
        tracing::Level::INFO,
        "crypto.derive",
        mem_kib = ARGON2_MEM_KIB,
        iterations = ARGON2_ITERATIONS,
        parallelism = ARGON2_PARALLELISM,
        output_bytes = ARGON2_OUTPUT_LEN,
        salt_bytes = SALT_LEN
        ; {
            let argon2 = Argon2::new(
                argon2::Algorithm::Argon2id,
                argon2::Version::V0x13,
                argon2_params(),
            );

            let mut key_bytes = [0u8; 32];
            argon2
                .hash_password_into(passkey.as_bytes(), salt, &mut key_bytes)
                .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;

            Ok(DerivedKey { bytes: key_bytes })
        }
    )
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

/// Writes data to a temporary file then atomically renames it to `path`.
///
/// The temporary file is created with mode 0600 on Unix. If the rename fails, the
/// temporary file is cleaned up and the error is returned.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let temp_path = temp_write_path(path)?;

    let path_str = path.display().to_string();
    let write_result = crate::timed_result!(
        tracing::Level::INFO,
        "crypto.write_atomic",
        path = path_str.as_str(),
        bytes = data.len()
        ; {
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
        }
    );

    if write_result.is_err() {
        let cleanup = std::fs::remove_file(&temp_path);
        let cleanup_result = if cleanup.is_ok() { "ok" } else { "error" };
        tracing::info!(
            phase = "cleanup",
            result = cleanup_result,
            path = %temp_path.display(),
            "crypto.write_atomic",
        );
    }

    write_result
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
        assert!((*hex).chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn argon2_params_use_reduced_values_under_cfg_test() {
        let params = argon2_params();
        assert_eq!(params.m_cost(), 8, "m_cost should be reduced under cfg(test)");
        assert_eq!(params.t_cost(), 1, "t_cost should be reduced under cfg(test)");
        assert_eq!(params.p_cost(), 1, "p_cost should be reduced under cfg(test)");
    }
}
