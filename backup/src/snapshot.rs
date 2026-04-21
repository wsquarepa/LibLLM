//! Snapshot creation pipeline with automatic base/diff decision logic.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use rand::RngCore;
use rand::TryRngCore;

use crate::BackupConfig;
use crate::index::{
    BackupEntry, BackupIndex, BackupType, FingerprintField, WrappedDek, backup_filename,
    generate_backup_id, is_safe_backup_filename, open_index, parse_backup_filename, save_index,
};

/// Creates a new backup snapshot (base or diff) of the database at `data_dir/data.db`.
///
/// Automatically decides between a base and diff snapshot based on the rebase threshold
/// and hard ceiling in `config`. Runs retention thinning after writing the snapshot.
pub fn create_snapshot(
    data_dir: &Path,
    passkey: Option<&str>,
    config: &BackupConfig,
) -> Result<()> {
    let db_path = data_dir.join("data.db");
    let backups_dir = data_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).context("failed to create backups directory")?;

    let index_path = backups_dir.join("index.json");
    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;
    let mut index = open_index(&index_path, backup_key.as_ref())?;
    let db_key: Option<libllm::crypto::DerivedKey> = match passkey {
        Some(pk) => {
            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            Some(libllm::crypto::derive_key(pk, &salt)?)
        }
        None => None,
    };

    let kek_fingerprint = backup_key
        .as_ref()
        .map(crate::crypto::compute_kek_fingerprint);

    let existing_chain_dek: Option<[u8; 32]> = match (&backup_key, index.latest_base()) {
        (Some(kek), Some(_)) => Some(resolve_chain_dek(&index, kek)?),
        _ => None,
    };

    let plaintext = crate::export::export_plaintext_db(&db_path, db_key.as_ref())?;
    let plaintext_hash = crate::hash::hash_bytes(&plaintext);

    let (backup_type, compressed) =
        build_payload(&plaintext, &index, &backups_dir, &existing_chain_dek, config)?;

    let (dek_for_this_entry, wrapped_dek_for_base, fingerprint_for_base): (
        Option<[u8; 32]>,
        Option<WrappedDek>,
        Option<FingerprintField>,
    ) = match (&backup_key, &backup_type) {
        (Some(kek), BackupType::Base) => {
            let dek = generate_dek()?;
            let wrapped = crate::crypto::wrap_dek(&dek, kek)?;
            let fp = kek_fingerprint
                .clone()
                .expect("kek present => fingerprint present");
            (
                Some(dek),
                Some(wrapped),
                Some(FingerprintField::Known(fp)),
            )
        }
        (Some(_), BackupType::Diff) => {
            let dek = existing_chain_dek.expect(
                "existing_chain_dek resolved above whenever backup_key and latest_base are present",
            );
            (Some(dek), None, None)
        }
        (None, _) => (None, None, None),
    };

    let stored = match dek_for_this_entry {
        Some(ref dek) => crate::crypto::encrypt_payload(&compressed, dek)?,
        None => compressed,
    };

    let (id, filename, file_path) = unique_backup_id(&backups_dir, backup_type);

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
        wrapped_dek: wrapped_dek_for_base,
        kek_fingerprint: fingerprint_for_base,
    };

    index.entries.push(entry);
    crate::retention::run_retention(&mut index, config, &backups_dir);
    save_index(&index_path, &index)?;

    Ok(())
}

/// Returns a (id, filename, file_path) triple that does not yet exist on disk.
///
/// Uses millisecond-resolution timestamps as the base ID. If the derived filename already
/// exists (same-millisecond collision), a 4-hex random suffix is appended and the check
/// is retried until a free slot is found.
fn unique_backup_id(
    backups_dir: &Path,
    backup_type: BackupType,
) -> (String, String, std::path::PathBuf) {
    let base_id = generate_backup_id();
    let base_filename = backup_filename(&base_id, backup_type);
    let base_path = backups_dir.join(&base_filename);
    if !base_path.exists() {
        return (base_id, base_filename, base_path);
    }

    loop {
        let mut suffix_bytes = [0u8; 2];
        rand::rng().fill_bytes(&mut suffix_bytes);
        let suffix = format!("{:04x}", u16::from_le_bytes(suffix_bytes));
        let id = format!("{base_id}-{suffix}");
        let filename = backup_filename(&id, backup_type);
        let path = backups_dir.join(&filename);
        if !path.exists() {
            return (id, filename, path);
        }
    }
}

/// Returns (BackupType, compressed_payload). The payload is already zstd-compressed.
///
/// `chain_dek` is the DEK for the current chain. When present, the latest base file is
/// decrypted under the DEK (not the KEK) before diff computation.
fn build_payload(
    plaintext: &[u8],
    index: &BackupIndex,
    backups_dir: &Path,
    chain_dek: &Option<[u8; 32]>,
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

    let decrypted = match chain_dek {
        Some(dek) => crate::crypto::decrypt_payload(&base_file_bytes, dek)?,
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

/// Reconstructs `backups/index.json` from the on-disk backup files.
///
/// Currently supports unencrypted data dirs only. For encrypted dirs,
/// rebuilding requires per-chain DEK reconstruction which is not yet
/// implemented — call sites that attempt this will receive an
/// actionable error.
pub fn rebuild_index(backups_dir: &Path, passkey: Option<&str>) -> Result<BackupIndex> {
    let data_dir = backups_dir
        .parent()
        .with_context(|| format!("backups_dir has no parent: {}", backups_dir.display()))?;

    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;

    if backup_key.is_some() {
        anyhow::bail!(
            "rebuild_index temporarily does not support encrypted backups \
             (will be re-added with fingerprint-unknown labeling; see Task 21 \
             of the backup REPL overhaul plan). Run without a passkey to \
             rebuild unencrypted data dirs, or restore from the existing \
             index.json."
        );
    }

    let mut file_entries: Vec<(std::time::SystemTime, String, String, BackupType)> = Vec::new();

    for dir_entry in std::fs::read_dir(backups_dir).with_context(|| {
        format!(
            "failed to read backups directory: {}",
            backups_dir.display()
        )
    })? {
        let dir_entry = dir_entry.with_context(|| {
            format!(
                "failed to read directory entry in {}",
                backups_dir.display()
            )
        })?;

        let filename = dir_entry.file_name().to_string_lossy().into_owned();

        if !is_safe_backup_filename(&filename) {
            continue;
        }

        let Some((id, entry_type)) = parse_backup_filename(&filename) else {
            continue;
        };

        let mtime = dir_entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        file_entries.push((mtime, filename, id, entry_type));
    }

    file_entries.sort_by_key(|(mtime, _, _, _)| *mtime);

    let mut index = BackupIndex::new();

    for (_mtime, filename, id, entry_type) in file_entries {
        let file_path = backups_dir.join(&filename);
        let file_bytes = match std::fs::read(&file_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Warning: skipping {filename}: failed to read: {e}");
                continue;
            }
        };

        let file_hash = crate::hash::hash_bytes(&file_bytes);
        let stored_size = file_bytes.len() as u64;

        let encrypted = backup_key.is_some();

        let created_at = Utc::now();

        match entry_type {
            BackupType::Base => {
                let decrypted = match &backup_key {
                    Some(key) => match crate::crypto::decrypt_payload(&file_bytes, key) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("Warning: skipping {filename}: decryption failed: {e}");
                            continue;
                        }
                    },
                    None => file_bytes,
                };

                let plaintext = match crate::diff::decompress(&decrypted) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Warning: skipping {filename}: decompression failed: {e}");
                        continue;
                    }
                };

                let plaintext_hash = crate::hash::hash_bytes(&plaintext);
                let plaintext_size = plaintext.len() as u64;

                index.entries.push(BackupEntry {
                    id,
                    entry_type: BackupType::Base,
                    filename: filename.clone(),
                    base_id: None,
                    plaintext_hash,
                    file_hash,
                    plaintext_size,
                    stored_size,
                    encrypted,
                    created_at,
                    wrapped_dek: None,
                    kek_fingerprint: None,
                });
            }

            BackupType::Diff => {
                let base_entry = match index.latest_base() {
                    Some(e) => e.clone(),
                    None => {
                        eprintln!(
                            "Warning: skipping {filename}: no base entry found before this diff"
                        );
                        continue;
                    }
                };

                let base_id = base_entry.id.clone();

                let chain = match index.chain_to(&base_id) {
                    Ok(c) => c.into_iter().cloned().collect::<Vec<_>>(),
                    Err(e) => {
                        eprintln!("Warning: skipping {filename}: failed to build base chain: {e}");
                        continue;
                    }
                };

                let chain_refs: Vec<&BackupEntry> = chain.iter().collect();
                let base_plaintext =
                    match crate::restore::replay_chain(backups_dir, &chain_refs, &backup_key) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!(
                                "Warning: skipping {filename}: failed to replay base chain: {e}"
                            );
                            continue;
                        }
                    };

                let diff_decrypted = match &backup_key {
                    Some(key) => match crate::crypto::decrypt_payload(&file_bytes, key) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("Warning: skipping {filename}: decryption failed: {e}");
                            continue;
                        }
                    },
                    None => file_bytes,
                };

                let patch = match crate::diff::decompress(&diff_decrypted) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Warning: skipping {filename}: failed to decompress diff: {e}");
                        continue;
                    }
                };

                let plaintext = match crate::diff::apply_patch(&base_plaintext, &patch) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Warning: skipping {filename}: failed to apply patch: {e}");
                        continue;
                    }
                };

                let plaintext_hash = crate::hash::hash_bytes(&plaintext);
                let plaintext_size = plaintext.len() as u64;

                index.entries.push(BackupEntry {
                    id,
                    entry_type: BackupType::Diff,
                    filename: filename.clone(),
                    base_id: Some(base_id),
                    plaintext_hash,
                    file_hash,
                    plaintext_size,
                    stored_size,
                    encrypted,
                    created_at,
                    wrapped_dek: None,
                    kek_fingerprint: None,
                });
            }
        }
    }

    Ok(index)
}

fn generate_dek() -> Result<[u8; 32]> {
    let mut dek = [0u8; 32];
    rand::rng()
        .try_fill_bytes(&mut dek)
        .context("RNG fill_bytes failed for DEK")?;
    Ok(dek)
}

fn resolve_chain_dek(index: &BackupIndex, kek: &[u8; 32]) -> Result<[u8; 32]> {
    let base = index
        .latest_base()
        .ok_or_else(|| anyhow::anyhow!("diff created without a base"))?;
    let wrapped = base
        .wrapped_dek
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("base entry {} missing wrapped DEK", base.id))?;
    crate::crypto::unwrap_dek(wrapped, kek)
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
            assert!(
                file_path.exists(),
                "backup file missing: {}",
                entry.filename
            );
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

    #[test]
    fn rebuild_index_populates_diff_hash_and_size() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap();

        let original_idx = load_test_index(dir.path());
        assert_eq!(original_idx.entries.len(), 2);

        let diff_original = original_idx
            .entries
            .iter()
            .find(|e| e.entry_type == BackupType::Diff)
            .unwrap();
        let expected_hash = diff_original.plaintext_hash.clone();
        let expected_size = diff_original.plaintext_size;

        let backups_dir = dir.path().join("backups");
        let rebuilt = rebuild_index(&backups_dir, None).unwrap();

        let diff_rebuilt = rebuilt
            .entries
            .iter()
            .find(|e| e.entry_type == BackupType::Diff)
            .unwrap();

        assert_eq!(
            diff_rebuilt.plaintext_hash, expected_hash,
            "rebuilt diff plaintext_hash must match original"
        );
        assert_eq!(
            diff_rebuilt.plaintext_size, expected_size,
            "rebuilt diff plaintext_size must match original"
        );
        assert!(
            !diff_rebuilt.plaintext_hash.is_empty(),
            "diff hash must not be empty"
        );
        assert!(
            diff_rebuilt.plaintext_size > 0,
            "diff size must not be zero"
        );
    }

    #[test]
    fn rapid_snapshots_produce_unique_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        for _ in 0..5 {
            modify_test_db(&db_path);
            create_snapshot(dir.path(), None, &config).unwrap();
        }

        let idx = load_test_index(dir.path());
        let ids: Vec<&str> = idx.entries.iter().map(|e| e.id.as_str()).collect();
        let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(
            unique.len(),
            ids.len(),
            "all snapshot ids must be unique, got duplicates: {ids:?}"
        );

        let filenames: Vec<&str> = idx.entries.iter().map(|e| e.filename.as_str()).collect();
        let unique_filenames: std::collections::HashSet<&str> = filenames.iter().copied().collect();
        assert_eq!(
            unique_filenames.len(),
            filenames.len(),
            "all snapshot filenames must be unique"
        );
    }
}
