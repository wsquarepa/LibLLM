//! Snapshot creation pipeline with automatic base/diff decision logic.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use rand::Rng;

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

    let (backup_type, compressed) = build_payload(
        &plaintext,
        &index,
        &backups_dir,
        &existing_chain_dek,
        config,
    )?;

    let (dek_for_this_entry, wrapped_dek_for_base, fingerprint_for_base): (
        Option<[u8; 32]>,
        Option<WrappedDek>,
        Option<FingerprintField>,
    ) = match (&backup_key, &backup_type) {
        (Some(kek), BackupType::Base) => {
            let dek = crate::crypto::generate_dek();
            let wrapped = crate::crypto::wrap_dek(&dek, kek)?;
            let fp = kek_fingerprint
                .clone()
                .expect("kek present => fingerprint present");
            (Some(dek), Some(wrapped), Some(FingerprintField::Known(fp)))
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
    let retention_result = crate::retention::run_retention(&mut index, config, &backups_dir);
    save_index(&index_path, &index).or_else(|e| {
        if retention_result.is_err() {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    retention_result?;

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
/// Returns the rebuilt index and a (possibly empty) list of human-readable
/// warning strings for files that were skipped due to read/decompression/patch
/// failures. Callers should surface these warnings to the user; they are also
/// emitted via `tracing::warn!` for debug-log subscribers.
///
/// For unencrypted data dirs, the rebuilt index carries accurate
/// `plaintext_hash` and `plaintext_size` values. For encrypted data dirs where
/// backup files were encrypted directly with the KEK (pre-v2 format), this
/// function decrypts each base file with the KEK, generates a fresh DEK,
/// re-encrypts the file under the new DEK, and stores the wrapped DEK in the
/// rebuilt index entry. When `passkey` is `None` or decryption fails, the
/// affected entry is skipped and a warning is appended to the return value.
pub fn rebuild_index(
    backups_dir: &Path,
    passkey: Option<&str>,
) -> Result<(BackupIndex, Vec<String>)> {
    let data_dir = backups_dir
        .parent()
        .with_context(|| format!("backups_dir has no parent: {}", backups_dir.display()))?;

    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;
    let dir_is_encrypted = data_dir.join(".salt").exists();

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
    let mut warnings: Vec<String> = Vec::new();

    for (mtime, filename, id, entry_type) in file_entries {
        let file_path = backups_dir.join(&filename);
        let file_bytes = match std::fs::read(&file_path) {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("skipping {filename}: failed to read file: {e}");
                tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.read_failed");
                warnings.push(msg);
                continue;
            }
        };

        let file_hash = crate::hash::hash_bytes(&file_bytes);
        let stored_size = file_bytes.len() as u64;

        let created_at = chrono::DateTime::<Utc>::from(mtime);

        match entry_type {
            BackupType::Base => {
                let (plaintext_hash, plaintext_size, kek_fingerprint, wrapped_dek) =
                    if dir_is_encrypted {
                        if backup_key.is_none() {
                            let msg = format!(
                                "skipping {filename}: encrypted backup requires a passkey to rebuild DEK"
                            );
                            tracing::warn!(
                                result = "error",
                                filename = %filename,
                                "backup.rebuild_index.encrypted_no_kek"
                            );
                            warnings.push(msg);
                            continue;
                        }
                        let kek = backup_key
                            .as_ref()
                            .expect("backup_key is Some: None case continued above");

                        let plaintext = match crate::crypto::decrypt_payload(&file_bytes, kek) {
                            Ok(p) => p,
                            Err(e) => {
                                let msg = format!("skipping {filename}: decryption failed: {e}");
                                tracing::warn!(
                                    result = "error",
                                    filename = %filename,
                                    error = %e,
                                    "backup.rebuild_index.decrypt_failed"
                                );
                                warnings.push(msg);
                                continue;
                            }
                        };

                        let dek = crate::crypto::generate_dek();
                        let new_blob = match crate::crypto::encrypt_payload(&plaintext, &dek) {
                            Ok(b) => b,
                            Err(e) => {
                                let msg = format!("skipping {filename}: re-encryption failed: {e}");
                                tracing::warn!(
                                    result = "error",
                                    filename = %filename,
                                    error = %e,
                                    "backup.rebuild_index.reencrypt_failed"
                                );
                                warnings.push(msg);
                                continue;
                            }
                        };

                        // Wrap the DEK before overwriting the on-disk file: if the wrap
                        // fails after the file is re-encrypted, the fresh DEK exists only
                        // in this stack frame and is lost, leaving the backup unrecoverable.
                        let wrapped = match crate::crypto::wrap_dek(&dek, kek) {
                            Ok(w) => w,
                            Err(e) => {
                                let msg = format!("skipping {filename}: DEK wrap failed: {e}");
                                tracing::warn!(
                                    result = "error",
                                    filename = %filename,
                                    error = %e,
                                    "backup.rebuild_index.wrap_dek_failed"
                                );
                                warnings.push(msg);
                                continue;
                            }
                        };

                        if let Err(e) = libllm::crypto::write_atomic(&file_path, &new_blob) {
                            let msg = format!(
                                "skipping {filename}: failed to persist re-encrypted file: {e}"
                            );
                            tracing::warn!(
                                result = "error",
                                filename = %filename,
                                error = %e,
                                "backup.rebuild_index.write_failed"
                            );
                            warnings.push(msg);
                            continue;
                        }

                        let fp = crate::crypto::compute_kek_fingerprint(kek);
                        (
                            "unknown".to_string(),
                            stored_size,
                            Some(FingerprintField::Known(fp)),
                            Some(wrapped),
                        )
                    } else {
                        let plaintext = match crate::diff::decompress(&file_bytes) {
                            Ok(p) => p,
                            Err(e) => {
                                let msg = format!("skipping {filename}: decompression failed: {e}");
                                tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.decompress_failed");
                                warnings.push(msg);
                                continue;
                            }
                        };
                        (
                            crate::hash::hash_bytes(&plaintext),
                            plaintext.len() as u64,
                            None,
                            None,
                        )
                    };

                index.entries.push(BackupEntry {
                    id,
                    entry_type: BackupType::Base,
                    filename: filename.clone(),
                    base_id: None,
                    plaintext_hash,
                    file_hash,
                    plaintext_size,
                    stored_size,
                    encrypted: dir_is_encrypted,
                    created_at,
                    wrapped_dek,
                    kek_fingerprint,
                });
            }

            BackupType::Diff => {
                let base_entry = match index.latest_base() {
                    Some(e) => e.clone(),
                    None => {
                        let msg =
                            format!("skipping {filename}: no base entry in rebuilt index yet");
                        tracing::warn!(result = "error", filename = %filename, "backup.rebuild_index.missing_base");
                        warnings.push(msg);
                        continue;
                    }
                };

                let base_id = base_entry.id.clone();

                let (plaintext_hash, plaintext_size) = if dir_is_encrypted {
                    ("unknown".to_string(), stored_size)
                } else {
                    let chain = match index.chain_to(&base_id) {
                        Ok(c) => c.into_iter().cloned().collect::<Vec<_>>(),
                        Err(e) => {
                            let msg = format!("skipping {filename}: failed to build chain: {e}");
                            tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.chain_build_failed");
                            warnings.push(msg);
                            continue;
                        }
                    };

                    let chain_refs: Vec<&BackupEntry> = chain.iter().collect();
                    let base_plaintext = match crate::restore::replay_chain(
                        backups_dir,
                        &chain_refs,
                        &backup_key,
                    ) {
                        Ok(p) => p,
                        Err(e) => {
                            let msg = format!("skipping {filename}: chain replay failed: {e}");
                            tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.chain_replay_failed");
                            warnings.push(msg);
                            continue;
                        }
                    };

                    let patch = match crate::diff::decompress(&file_bytes) {
                        Ok(p) => p,
                        Err(e) => {
                            let msg =
                                format!("skipping {filename}: diff decompression failed: {e}");
                            tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.diff_decompress_failed");
                            warnings.push(msg);
                            continue;
                        }
                    };

                    let plaintext = match crate::diff::apply_patch(&base_plaintext, &patch) {
                        Ok(p) => p,
                        Err(e) => {
                            let msg = format!("skipping {filename}: patch apply failed: {e}");
                            tracing::warn!(result = "error", filename = %filename, error = %e, "backup.rebuild_index.patch_apply_failed");
                            warnings.push(msg);
                            continue;
                        }
                    };

                    (crate::hash::hash_bytes(&plaintext), plaintext.len() as u64)
                };

                index.entries.push(BackupEntry {
                    id,
                    entry_type: BackupType::Diff,
                    filename: filename.clone(),
                    base_id: Some(base_id),
                    plaintext_hash,
                    file_hash,
                    plaintext_size,
                    stored_size,
                    encrypted: dir_is_encrypted,
                    created_at,
                    wrapped_dek: None,
                    kek_fingerprint: None,
                });
            }
        }
    }

    Ok((index, warnings))
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

    fn setup_encrypted_test_db(dir: &std::path::Path, passkey: &str) -> std::path::PathBuf {
        let salt = libllm::crypto::load_or_create_salt(&dir.join(".salt")).unwrap();
        let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
        let db_path = dir.join("data.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&key.key_pragma()).unwrap();
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
        let (rebuilt, _warnings) = rebuild_index(&backups_dir, None).unwrap();

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
    fn rebuild_index_encrypted_base_without_valid_ciphertext_produces_warning() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        // Pre-create the salt so resolve_backup_key succeeds without needing a DB.
        let _ = crate::crypto::resolve_backup_key(data_dir, Some("pw")).unwrap();

        // Drop an encrypted-looking base file with no corresponding index.
        // The 128 bytes of zeros are not valid XChaCha20-Poly1305 ciphertext.
        let id = "20260421T040000.000Z".to_string();
        let filename = crate::index::backup_filename(&id, BackupType::Base);
        libllm::crypto::write_atomic(&backups_dir.join(&filename), &[0u8; 128]).unwrap();

        let (idx, warnings) = rebuild_index(&backups_dir, Some("pw")).unwrap();

        assert_eq!(idx.version, crate::index::SCHEMA_VERSION);
        assert!(
            idx.entries.is_empty(),
            "undecodable encrypted file must be skipped, not added as Unknown"
        );
        assert!(
            !warnings.is_empty(),
            "a warning must be emitted for the undecodable file"
        );
    }

    #[test]
    fn rebuild_index_encrypted_chain_gets_wrapped_dek() {
        use crate::index::backup_filename;

        let dir = tempfile::TempDir::new().unwrap();
        let data_dir = dir.path();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        // Files created with create_snapshot are DEK-encrypted and cannot be decrypted with the
        // KEK alone. rebuild_index is designed for the pre-v2 (v1) format where files were
        // encrypted directly with the KEK — the same scenario v2 migration handles. We
        // simulate that here by encrypting files directly with the KEK.
        let kek = crate::crypto::resolve_backup_key(data_dir, Some("pw"))
            .unwrap()
            .unwrap();

        let base_id = "20260530T000000.000Z".to_string();
        let diff_id = "20260530T000001.000Z".to_string();
        let base_filename = backup_filename(&base_id, BackupType::Base);
        let diff_filename = backup_filename(&diff_id, BackupType::Diff);

        let base_blob = crate::crypto::encrypt_payload(b"db-snapshot-base-content", &kek).unwrap();
        let diff_blob = crate::crypto::encrypt_payload(b"db-snapshot-diff-content", &kek).unwrap();
        let base_path = backups_dir.join(&base_filename);
        let diff_path = backups_dir.join(&diff_filename);
        libllm::crypto::write_atomic(&base_path, &base_blob).unwrap();
        libllm::crypto::write_atomic(&diff_path, &diff_blob).unwrap();

        // Set the base file's mtime earlier than the diff so that rebuild_index processes
        // the base before the diff (it sorts by mtime).
        let base_mtime = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&base_path)
            .unwrap()
            .set_modified(base_mtime)
            .unwrap();

        let (rebuilt, warnings) = rebuild_index(&backups_dir, Some("pw")).unwrap();

        assert!(
            warnings.is_empty(),
            "expected no warnings, got: {warnings:?}"
        );
        assert_eq!(
            rebuilt.entries.len(),
            2,
            "both entries must be in the rebuilt index"
        );

        let base = rebuilt
            .entries
            .iter()
            .find(|e| e.entry_type == BackupType::Base)
            .expect("base entry must be present");
        assert!(
            base.wrapped_dek.is_some(),
            "rebuilt encrypted base must have a wrapped_dek"
        );
        assert!(
            matches!(
                base.kek_fingerprint,
                Some(crate::index::FingerprintField::Known(_))
            ),
            "rebuilt encrypted base must have a Known kek_fingerprint, got {:?}",
            base.kek_fingerprint
        );
    }

    #[test]
    fn rebuild_index_encrypted_without_passkey_adds_warning() {
        let dir = tempfile::TempDir::new().unwrap();
        let data_dir = dir.path();
        setup_encrypted_test_db(data_dir, "pw");
        let config = BackupConfig::default();

        create_snapshot(data_dir, Some("pw"), &config).unwrap();

        let backups_dir = data_dir.join("backups");
        std::fs::remove_file(backups_dir.join("index.json")).unwrap();

        let (rebuilt, warnings) = rebuild_index(&backups_dir, None).unwrap();

        assert!(
            !warnings.is_empty(),
            "expected a warning for encrypted entry without passkey"
        );
        assert!(
            rebuilt.entries.is_empty(),
            "encrypted entries without KEK must not appear in rebuilt index"
        );
    }

    #[test]
    fn rebuild_index_uses_file_mtime_for_created_at() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = setup_test_db(dir.path());
        let config = BackupConfig::default();

        create_snapshot(dir.path(), None, &config).unwrap();
        modify_test_db(&db_path);
        create_snapshot(dir.path(), None, &config).unwrap();

        let backups_dir = dir.path().join("backups");
        let idx = load_test_index(dir.path());

        // Backdate every backup file's mtime by exactly one hour so that the
        // rebuilt `created_at` values are definitively distant from `Utc::now()`.
        let past = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap();
        for entry in &idx.entries {
            let path = backups_dir.join(&entry.filename);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(past)
                .unwrap();
        }

        let (rebuilt, _warnings) = rebuild_index(&backups_dir, None).unwrap();

        let now = Utc::now();
        let expected_mtime: chrono::DateTime<Utc> = past.into();

        for entry in &rebuilt.entries {
            let delta_from_mtime = (entry.created_at - expected_mtime).abs().num_seconds();
            let delta_from_now = (entry.created_at - now).abs().num_seconds();

            assert!(
                delta_from_mtime < 2,
                "entry {} created_at should be within 2s of file mtime ({}), got {} (delta from now: {}s)",
                entry.id,
                expected_mtime,
                entry.created_at,
                delta_from_now,
            );
            assert!(
                delta_from_now > 3500,
                "entry {} created_at must not be close to Utc::now(); expected ~3600s gap, got {}s",
                entry.id,
                delta_from_now,
            );
        }
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

    #[test]
    fn rebuild_index_reports_warnings_for_corrupt_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let backups_dir = dir.path().join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        // Write a valid base snapshot.
        let data_dir = dir.path();
        setup_test_db(data_dir);
        let config = BackupConfig::default();
        create_snapshot(data_dir, None, &config).unwrap();

        // Write a truncated (undecompressable) base file that will fail decompression.
        let corrupt_id = "19700101T000000.001Z".to_string();
        let corrupt_filename = crate::index::backup_filename(&corrupt_id, BackupType::Base);
        std::fs::write(backups_dir.join(&corrupt_filename), b"not-valid-zstd").unwrap();

        let (rebuilt, warnings) = rebuild_index(&backups_dir, None).unwrap();

        assert!(
            !warnings.is_empty(),
            "expected at least one warning for the corrupt file, got none"
        );
        assert!(
            warnings.iter().any(|w| w.contains(&corrupt_filename)),
            "warning must mention the corrupt filename; warnings: {warnings:?}"
        );
        assert_eq!(
            rebuilt.entries.len(),
            1,
            "only the valid entry should be in the rebuilt index"
        );
    }
}
