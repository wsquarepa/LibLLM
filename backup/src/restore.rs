//! Chain replay and database restoration from backup points.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::index::{BackupEntry, load_index};

/// Replays a backup chain (base + diffs) and returns the resulting plaintext bytes.
///
/// `chain` must be ordered base-first (as returned by `BackupIndex::chain_to`).
/// Each file is read from `backups_dir`, optionally decrypted, decompressed,
/// and diffs are applied sequentially over the base.
pub(crate) fn replay_chain(
    backups_dir: &Path,
    chain: &[&BackupEntry],
    backup_key: &Option<[u8; 32]>,
) -> Result<Vec<u8>> {
    if backup_key.is_none()
        && let Some(encrypted) = chain.iter().find(|e| e.encrypted)
    {
        bail!(
            "backup entry {} is encrypted but no passkey was provided",
            encrypted.id
        );
    }

    let base_entry = chain[0];
    let base_bytes = std::fs::read(backups_dir.join(&base_entry.filename))
        .with_context(|| format!("failed to read base backup: {}", base_entry.filename))?;

    let base_decrypted = match backup_key {
        Some(key) => crate::crypto::decrypt_payload(&base_bytes, key)?,
        None => base_bytes,
    };
    let mut plaintext = crate::diff::decompress(&base_decrypted)?;

    for diff_entry in &chain[1..] {
        let diff_bytes = std::fs::read(backups_dir.join(&diff_entry.filename))
            .with_context(|| format!("failed to read diff backup: {}", diff_entry.filename))?;

        let diff_decrypted = match backup_key {
            Some(key) => crate::crypto::decrypt_payload(&diff_bytes, key)?,
            None => diff_bytes,
        };
        let patch = crate::diff::decompress(&diff_decrypted)?;
        plaintext = crate::diff::apply_patch(&plaintext, &patch)?;
    }

    Ok(plaintext)
}

/// Restores the database to the state captured at `target_id`.
///
/// Loads the backup chain ending at `target_id`, replays diffs over the base, verifies the
/// result against the stored plaintext hash, creates a pre-restore safety backup, then writes
/// the restored database to `data_dir/data.db`.
///
/// When `passkey` is provided, backup files are decrypted before use and the restored database
/// is written as an encrypted SQLCipher database using the DB key derived from that passkey.
/// When `passkey` is None, backup files are read as plaintext and the restored database is
/// written as a plaintext SQLite file.
pub fn restore_to_point(data_dir: &Path, target_id: &str, passkey: Option<&str>) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    let index_path = backups_dir.join("index.json");
    let index = load_index(&index_path)?;

    let chain = index
        .chain_to(target_id)
        .with_context(|| format!("backup id not found or chain is broken: {target_id}"))?;

    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;

    let plaintext = replay_chain(&backups_dir, &chain, &backup_key)
        .context("failed to replay backup chain")?;

    let target_entry = chain.last().expect("chain is non-empty");
    let actual_hash = crate::hash::hash_bytes(&plaintext);
    if actual_hash != target_entry.plaintext_hash {
        bail!(
            "hash mismatch after chain replay: expected {}, got {actual_hash}",
            target_entry.plaintext_hash
        );
    }

    let db_path = data_dir.join("data.db");
    if db_path.exists() {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let safety_path = backups_dir.join(format!("pre-restore-{timestamp}.db"));
        std::fs::copy(&db_path, &safety_path).with_context(|| {
            format!(
                "failed to create pre-restore safety backup: {}",
                safety_path.display()
            )
        })?;
    }

    match passkey {
        None => {
            libllm::crypto::write_atomic(&db_path, &plaintext)
                .context("failed to write restored database")?;
        }
        Some(pk) => {
            let temp_plain = tempfile::NamedTempFile::new()
                .context("failed to create temp file for restore")?;
            let temp_plain_path = temp_plain.path().to_path_buf();
            std::fs::write(&temp_plain_path, &plaintext)
                .context("failed to write plaintext to temp file")?;

            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            let db_key = libllm::crypto::derive_key(pk, &salt)?;

            // Remove the existing DB file so the destination connection creates a fresh
            // unencrypted database. We then use sqlcipher_export to write an encrypted copy.
            if db_path.exists() {
                std::fs::remove_file(&db_path)
                    .context("failed to remove existing database before encrypted restore")?;
            }

            // Open the plaintext source and export it directly into an encrypted destination.
            // SQLCipher's backup API does not support plaintext->encrypted transfers, so we use
            // ATTACH + sqlcipher_export which is the canonical SQLCipher migration path.
            let src = rusqlite::Connection::open(&temp_plain_path)
                .context("failed to open plaintext temp db")?;
            let key_hex = db_key.hex();
            src.execute_batch(&format!(
                "ATTACH DATABASE '{}' AS encrypted KEY \"x'{}'\";\
                 SELECT sqlcipher_export('encrypted');\
                 DETACH DATABASE encrypted;",
                db_path.display(),
                key_hex,
            ))
            .context("failed to export plaintext database as encrypted")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db(dir: &Path) -> std::path::PathBuf {
        let db_path = dir.join("data.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);")
            .unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", ["initial"])
            .unwrap();
        drop(conn);
        db_path
    }

    fn add_row(db_path: &Path, value: &str) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", [value])
            .unwrap();
    }

    fn count_rows(db_path: &Path) -> i64 {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.query_row("SELECT count(*) FROM test", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn restore_to_base_point() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = crate::BackupConfig::default();

        let base_row_count = count_rows(&db_path);

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let index_path = dir.path().join("backups").join("index.json");
        let index = load_index(&index_path).unwrap();
        let base_id = index.entries[0].id.clone();

        add_row(&db_path, "extra row");
        assert_eq!(count_rows(&db_path), base_row_count + 1);

        restore_to_point(dir.path(), &base_id, None).unwrap();

        assert_eq!(count_rows(&db_path), base_row_count);
    }

    #[test]
    fn restore_to_diff_point() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = crate::BackupConfig::default();

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        add_row(&db_path, "second row");
        let row_count_after_diff = count_rows(&db_path);

        // Wait for the second to tick over so generate_backup_id produces a distinct id.
        std::thread::sleep(std::time::Duration::from_secs(1));
        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let index_path = dir.path().join("backups").join("index.json");
        let index = load_index(&index_path).unwrap();
        let diff_id = index
            .entries
            .iter()
            .find(|e| e.entry_type == crate::index::BackupType::Diff)
            .unwrap()
            .id
            .clone();

        add_row(&db_path, "third row beyond diff");
        assert!(count_rows(&db_path) > row_count_after_diff);

        restore_to_point(dir.path(), &diff_id, None).unwrap();

        assert_eq!(count_rows(&db_path), row_count_after_diff);
    }

    #[test]
    fn restore_creates_pre_restore_backup() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = crate::BackupConfig::default();

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let index_path = dir.path().join("backups").join("index.json");
        let index = load_index(&index_path).unwrap();
        let target_id = index.entries[0].id.clone();

        add_row(&db_path, "extra row");

        restore_to_point(dir.path(), &target_id, None).unwrap();

        let backups_dir = dir.path().join("backups");
        let pre_restore_exists = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("pre-restore-")
            });

        assert!(pre_restore_exists, "expected a pre-restore-* file in backups dir");
    }
}
