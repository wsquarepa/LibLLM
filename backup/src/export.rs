//! Plaintext database export for backup snapshots, handling optional SQLCipher decryption.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use libllm::crypto::DerivedKey;

/// Export a SQLite database (possibly encrypted with SQLCipher) to plaintext bytes.
///
/// Opens the database at `db_path`, optionally decrypts it using `key`, checkpoints the WAL,
/// then copies to a temp file via the SQLite backup API and returns the raw bytes.
///
/// Returns an error if the key is wrong, the database is corrupt, or any I/O fails.
pub fn export_plaintext_db(db_path: &Path, key: Option<&DerivedKey>) -> Result<Vec<u8>> {
    let src = rusqlite::Connection::open(db_path)?;

    if let Some(derived_key) = key {
        src.execute_batch(&derived_key.key_pragma())?;
        src.query_row("SELECT count(*) FROM sqlite_master;", [], |_| Ok(()))?;
    }

    src.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;

    let dest_file = tempfile::NamedTempFile::new()?;
    let dest_path = dest_file.path().to_path_buf();

    if key.is_some() {
        // SQLCipher does not support the backup API for encrypted-to-plaintext copies.
        // The canonical approach is to attach a plaintext destination and use sqlcipher_export.
        let dest_path_str = dest_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("temp file path is not valid UTF-8"))?;
        src.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS plaintext KEY '';\
             SELECT sqlcipher_export('plaintext');\
             DETACH DATABASE plaintext;",
            dest_path_str.replace('\'', "''"),
        ))?;
    } else {
        let mut dest = rusqlite::Connection::open(&dest_path)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dest)?;
        backup.run_to_completion(100, Duration::ZERO, None)?;
    }

    let bytes = std::fs::read(&dest_path)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_simple_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("test.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);")
            .unwrap();
        conn.execute_batch("INSERT INTO items (name) VALUES ('hello');")
            .unwrap();
        db_path
    }

    fn create_encrypted_db(dir: &TempDir, key: &DerivedKey) -> std::path::PathBuf {
        let db_path = dir.path().join("encrypted.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", &*key.hex()))
            .unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);")
            .unwrap();
        conn.execute_batch("INSERT INTO items (name) VALUES ('secret');")
            .unwrap();
        db_path
    }

    #[test]
    fn export_unencrypted_db_returns_valid_sqlite() {
        let dir = TempDir::new().unwrap();
        let db_path = create_simple_db(&dir);

        let bytes = export_plaintext_db(&db_path, None).unwrap();

        assert_eq!(&bytes[..16], b"SQLite format 3\0");
    }

    #[test]
    fn export_encrypted_db_returns_plaintext_sqlite() {
        let dir = TempDir::new().unwrap();
        let salt = libllm::crypto::load_or_create_salt(&dir.path().join(".salt")).unwrap();
        let key = libllm::crypto::derive_key("test-passkey", &salt).unwrap();
        let db_path = create_encrypted_db(&dir, &key);

        let bytes = export_plaintext_db(&db_path, Some(&key)).unwrap();

        assert_eq!(&bytes[..16], b"SQLite format 3\0");
    }

    #[test]
    fn export_encrypted_db_with_wrong_key_fails() {
        let dir = TempDir::new().unwrap();
        let salt = libllm::crypto::load_or_create_salt(&dir.path().join(".salt")).unwrap();
        let key = libllm::crypto::derive_key("correct-passkey", &salt).unwrap();
        let wrong_key = libllm::crypto::derive_key("wrong-passkey", &salt).unwrap();
        let db_path = create_encrypted_db(&dir, &key);

        let result = export_plaintext_db(&db_path, Some(&wrong_key));

        assert!(result.is_err());
    }

    #[test]
    fn export_produces_deterministic_output_across_calls() {
        let dir = TempDir::new().unwrap();
        let db_path = create_simple_db(&dir);

        let bytes1 = export_plaintext_db(&db_path, None).unwrap();
        let bytes2 = export_plaintext_db(&db_path, None).unwrap();

        assert_eq!(bytes1, bytes2);
    }
}
