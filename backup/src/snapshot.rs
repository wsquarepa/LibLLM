use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::index::{
    BackupEntry, BackupIndex, BackupType, backup_filename, generate_backup_id, load_index,
    save_index,
};
use crate::BackupConfig;

pub fn create_snapshot(data_dir: &Path, passkey: Option<&str>, config: &BackupConfig) -> Result<()> {
    let db_path = data_dir.join("data.db");
    let backups_dir = data_dir.join("backups");
    std::fs::create_dir_all(&backups_dir)
        .context("failed to create backups directory")?;

    let index_path = backups_dir.join("index.json");
    let mut index = load_index(&index_path)?;

    let (db_key, backup_key) = match passkey {
        Some(pk) => {
            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            let dk = libllm::crypto::derive_key(pk, &salt)?;
            let bk = crate::crypto::derive_backup_key(pk, &salt)?;
            (Some(dk), Some(bk))
        }
        None => (None, None),
    };

    let plaintext = crate::export::export_plaintext_db(&db_path, db_key.as_ref())?;
    let plaintext_hash = crate::hash::hash_bytes(&plaintext);

    let (backup_type, compressed) =
        build_payload(&plaintext, &index, &backups_dir, &backup_key, config)?;

    let stored = match &backup_key {
        Some(key) => crate::crypto::encrypt_payload(&compressed, key)?,
        None => compressed,
    };

    let id = generate_backup_id();
    let filename = backup_filename(&id, backup_type);
    let file_path = backups_dir.join(&filename);

    libllm::crypto::write_atomic(&file_path, &stored)
        .with_context(|| format!("failed to write backup file: {}", file_path.display()))?;

    let file_hash = crate::hash::hash_bytes(&stored);

    let base_id = match backup_type {
        BackupType::Base => None,
        BackupType::Diff => index.latest_base().map(|e| e.id.clone()),
    };

    let entry = BackupEntry {
        id,
        entry_type: backup_type,
        filename,
        base_id,
        plaintext_hash,
        file_hash,
        plaintext_size: plaintext.len() as u64,
        stored_size: stored.len() as u64,
        encrypted: backup_key.is_some(),
        created_at: Utc::now(),
    };

    index.entries.push(entry);
    crate::retention::run_retention(&mut index, config, &backups_dir);
    save_index(&index_path, &index)?;

    Ok(())
}

/// Returns (BackupType, compressed_payload). The payload is already zstd-compressed.
fn build_payload(
    plaintext: &[u8],
    index: &BackupIndex,
    backups_dir: &Path,
    backup_key: &Option<[u8; 32]>,
    config: &BackupConfig,
) -> Result<(BackupType, Vec<u8>)> {
    let compress_as_base =
        || crate::diff::compress(plaintext).context("failed to compress base payload");

    let Some(latest_base) = index.latest_base() else {
        return Ok((BackupType::Base, compress_as_base()?));
    };

    if index.diffs_since_last_base() >= config.rebase_hard_ceiling as usize {
        return Ok((BackupType::Base, compress_as_base()?));
    }

    let base_file_path = backups_dir.join(&latest_base.filename);
    let base_file_bytes = std::fs::read(&base_file_path)
        .with_context(|| format!("failed to read base file: {}", base_file_path.display()))?;

    let decrypted = match backup_key {
        Some(key) => crate::crypto::decrypt_payload(&base_file_bytes, key)?,
        None => base_file_bytes,
    };
    let base_plaintext = crate::diff::decompress(&decrypted)?;

    let patch = crate::diff::compute_diff(&base_plaintext, plaintext)?;
    let compressed_patch = crate::diff::compress(&patch)?;

    let threshold = (latest_base.plaintext_size * config.rebase_threshold_percent as u64) / 100;
    if compressed_patch.len() as u64 > threshold {
        return Ok((BackupType::Base, compress_as_base()?));
    }

    Ok((BackupType::Diff, compressed_patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{self, BackupType};

    fn setup_test_db(dir: &std::path::Path) -> std::path::PathBuf {
        let db_path = dir.join("data.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);")
            .unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", ["hello"])
            .unwrap();
        drop(conn);
        db_path
    }

    fn modify_test_db(db_path: &std::path::Path) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", ["world"])
            .unwrap();
    }

    fn load_test_index(dir: &std::path::Path) -> BackupIndex {
        let index_path = dir.join("backups").join("index.json");
        index::load_index(&index_path).unwrap()
    }

    #[test]
    fn first_backup_creates_base() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].entry_type, BackupType::Base);
        assert!(idx.entries[0].base_id.is_none());
    }

    #[test]
    fn second_backup_creates_diff() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries[1].entry_type, BackupType::Diff);
        assert!(idx.entries[1].base_id.is_some());
    }

    #[test]
    fn diff_is_smaller_than_base_for_small_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        let base = &idx.entries[0];
        let diff = &idx.entries[1];

        assert_eq!(diff.entry_type, BackupType::Diff);
        assert!(diff.stored_size < base.stored_size);
    }

    #[test]
    fn hard_ceiling_forces_rebase() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        // rebase_hard_ceiling: 2 means after 2 diffs exist, the next snapshot is a base
        let config = BackupConfig {
            rebase_hard_ceiling: 2,
            ..BackupConfig::default()
        };

        create_snapshot(dir.path(), None, &config).unwrap(); // base
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap(); // diff (1 diff)
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap(); // diff (2 diffs)
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap(); // forced base (2 >= 2)

        let idx = load_test_index(dir.path());
        assert_eq!(idx.entries.len(), 4);
        assert_eq!(idx.entries[3].entry_type, BackupType::Base);
    }

    #[test]
    fn backup_files_exist_on_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        let backups_dir = dir.path().join("backups");
        for entry in &idx.entries {
            let file_path = backups_dir.join(&entry.filename);
            assert!(file_path.exists(), "backup file missing: {}", entry.filename);
        }
    }

    #[test]
    fn unmodified_db_still_creates_backup() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        assert_eq!(idx.entries.len(), 2);
    }

    #[test]
    fn retention_runs_after_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig {
            keep_all_days: 0,
            keep_daily_days: 0,
            keep_weekly_days: 0,
            rebase_hard_ceiling: 100,
            ..BackupConfig::default()
        };

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        std::thread::sleep(std::time::Duration::from_secs(1));
        create_snapshot(dir.path(), None, &config).unwrap();

        let idx = load_test_index(dir.path());
        assert!(idx.entries.len() <= 2);
    }
}
