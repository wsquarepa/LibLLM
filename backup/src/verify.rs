//! Backup chain integrity verification via file hash checks and optional full replay.

use std::path::Path;

use anyhow::Result;

use crate::index::open_index;

/// Summary of a backup verification run: how many entries were checked and any errors found.
pub struct VerifyResult {
    pub checked_count: usize,
    pub errors: Vec<String>,
}

/// Verifies backup integrity by checking file hashes and optionally replaying chains.
///
/// For each index entry: confirms the file exists, then compares its hash against the recorded
/// `file_hash`. When `full_replay` is true, each entry's chain is also replayed end-to-end and
/// the resulting plaintext is hashed against the stored `plaintext_hash`.
///
/// `passkey` is required when backups were stored encrypted. Entries whose chain files are already
/// reported as missing or corrupted are skipped during replay (they were caught in the hash phase).
pub fn verify_chain(
    data_dir: &Path,
    passkey: Option<&str>,
    archived_passkey: Option<&str>,
    full_replay: bool,
) -> Result<VerifyResult> {
    let backups_dir = data_dir.join("backups");
    let index_path = backups_dir.join("index.json");
    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;
    let index = open_index(&index_path, backup_key.as_ref())?;

    let mut result = VerifyResult {
        checked_count: 0,
        errors: Vec::new(),
    };

    let mut missing_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in &index.entries {
        let file_path = backups_dir.join(&entry.filename);

        if !file_path.exists() {
            result.errors.push(format!(
                "missing backup file: {} (id: {})",
                entry.filename, entry.id
            ));
            missing_ids.insert(entry.id.clone());
            result.checked_count += 1;
            continue;
        }

        match crate::hash::hash_file(&file_path) {
            Err(e) => {
                result.errors.push(format!(
                    "failed to hash file {} (id: {}): {e}",
                    entry.filename, entry.id
                ));
                missing_ids.insert(entry.id.clone());
            }
            Ok(actual_hash) => {
                if actual_hash != entry.file_hash {
                    result.errors.push(format!(
                        "hash mismatch for {} (id: {}): expected {}, got {actual_hash}",
                        entry.filename, entry.id, entry.file_hash
                    ));
                    missing_ids.insert(entry.id.clone());
                }
            }
        }

        result.checked_count += 1;
    }

    if !full_replay {
        return Ok(result);
    }

    if backup_key.is_none() && index.entries.iter().any(|e| e.encrypted) {
        result.errors.push(
            "cannot replay encrypted backup chain without a passkey: \
             re-run the verify command with --passkey (or set LIBLLM_PASSKEY)"
                .to_string(),
        );
        return Ok(result);
    }

    let current_fp = backup_key
        .as_ref()
        .map(crate::crypto::compute_kek_fingerprint);
    let archived_key = crate::crypto::resolve_backup_key(data_dir, archived_passkey)?;

    for entry in &index.entries {
        let Ok(chain) = index.chain_to(&entry.id) else {
            continue;
        };

        let chain_has_missing = chain.iter().any(|e| missing_ids.contains(&e.id));
        if chain_has_missing {
            continue;
        }

        let chain_root = chain[0];
        let effective_kek: Option<[u8; 32]> = match &chain_root.kek_fingerprint {
            None => backup_key,
            Some(crate::index::FingerprintField::Known(fp))
                if Some(fp) == current_fp.as_ref() =>
            {
                backup_key
            }
            Some(other) => {
                if archived_passkey.is_none() {
                    let msg = match other {
                        crate::index::FingerprintField::Known(fp) => format!(
                            "chain {} is archived under passkey fingerprint {fp}; \
                             provide --archived-passkey to verify",
                            entry.id
                        ),
                        crate::index::FingerprintField::Unknown => format!(
                            "chain {} has no recorded passkey fingerprint; \
                             provide --archived-passkey to verify",
                            entry.id
                        ),
                    };
                    result.errors.push(msg);
                    continue;
                }
                archived_key
            }
        };

        let replay_result = crate::restore::replay_chain(&backups_dir, &chain, &effective_kek);
        match replay_result {
            Err(e) => {
                result
                    .errors
                    .push(format!("chain replay failed for id {}: {e}", entry.id));
            }
            Ok(plaintext) => {
                let actual_hash = crate::hash::hash_bytes(&plaintext);
                if actual_hash != entry.plaintext_hash {
                    result.errors.push(format!(
                        "plaintext hash mismatch after chain replay for id {}: expected {}, got {actual_hash}",
                        entry.id, entry.plaintext_hash
                    ));
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackupConfig;

    fn setup_test_db(dir: &Path) {
        let db_path = dir.join("data.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);")
            .unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", ["hello"])
            .unwrap();
    }

    #[test]
    fn verify_intact_chain_passes() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_db(dir.path());
        let config = BackupConfig::default();

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let result = verify_chain(dir.path(), None, None, false).unwrap();

        assert_eq!(result.checked_count, 1);
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn verify_detects_corrupted_file() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_db(dir.path());
        let config = BackupConfig::default();

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let backups_dir = dir.path().join("backups");
        let index_path = backups_dir.join("index.json");
        let index = crate::index::load_index(&index_path).unwrap();
        let entry = &index.entries[0];
        let file_path = backups_dir.join(&entry.filename);

        std::fs::write(&file_path, b"garbage data that is not a valid backup").unwrap();

        let result = verify_chain(dir.path(), None, None, false).unwrap();

        assert_eq!(result.checked_count, 1);
        assert!(
            !result.errors.is_empty(),
            "expected errors for corrupted file"
        );
    }

    #[test]
    fn verify_detects_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_db(dir.path());
        let config = BackupConfig::default();

        crate::snapshot::create_snapshot(dir.path(), None, &config).unwrap();

        let backups_dir = dir.path().join("backups");
        let index_path = backups_dir.join("index.json");
        let index = crate::index::load_index(&index_path).unwrap();
        let entry = &index.entries[0];
        let file_path = backups_dir.join(&entry.filename);

        std::fs::remove_file(&file_path).unwrap();

        let result = verify_chain(dir.path(), None, None, false).unwrap();

        assert_eq!(result.checked_count, 1);
        assert!(
            !result.errors.is_empty(),
            "expected errors for missing file"
        );
    }
}
