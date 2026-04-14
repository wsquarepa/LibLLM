use std::path::Path;

use anyhow::Result;

use crate::index::load_index;

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
    full_replay: bool,
) -> Result<VerifyResult> {
    let backups_dir = data_dir.join("backups");
    let index_path = backups_dir.join("index.json");
    let index = load_index(&index_path)?;

    let backup_key: Option<[u8; 32]> = match passkey {
        Some(pk) => {
            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            Some(crate::crypto::derive_backup_key(pk, &salt)?)
        }
        None => None,
    };

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

    for entry in &index.entries {
        let Some(chain) = index.chain_to(&entry.id) else {
            continue;
        };

        let chain_has_missing = chain.iter().any(|e| missing_ids.contains(&e.id));
        if chain_has_missing {
            continue;
        }

        let replay_result = replay_chain(&backups_dir, &chain, &backup_key);
        match replay_result {
            Err(e) => {
                result.errors.push(format!(
                    "chain replay failed for id {}: {e}",
                    entry.id
                ));
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

fn replay_chain(
    backups_dir: &Path,
    chain: &[&crate::index::BackupEntry],
    backup_key: &Option<[u8; 32]>,
) -> Result<Vec<u8>> {
    let base_entry = chain[0];
    let base_file = backups_dir.join(&base_entry.filename);
    let base_bytes = std::fs::read(&base_file)?;

    let base_decrypted = match backup_key {
        Some(key) => crate::crypto::decrypt_payload(&base_bytes, key)?,
        None => base_bytes,
    };
    let mut plaintext = crate::diff::decompress(&base_decrypted)?;

    for diff_entry in &chain[1..] {
        let diff_file = backups_dir.join(&diff_entry.filename);
        let diff_bytes = std::fs::read(&diff_file)?;

        let diff_decrypted = match backup_key {
            Some(key) => crate::crypto::decrypt_payload(&diff_bytes, key)?,
            None => diff_bytes,
        };
        let patch = crate::diff::decompress(&diff_decrypted)?;
        plaintext = crate::diff::apply_patch(&plaintext, &patch)?;
    }

    Ok(plaintext)
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

        let result = verify_chain(dir.path(), None, false).unwrap();

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

        let result = verify_chain(dir.path(), None, false).unwrap();

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

        let result = verify_chain(dir.path(), None, false).unwrap();

        assert_eq!(result.checked_count, 1);
        assert!(
            !result.errors.is_empty(),
            "expected errors for missing file"
        );
    }
}
