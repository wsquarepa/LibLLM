//! Chain replay and database restoration from backup points.

use std::path::Path;

use zeroize::Zeroizing;

use crate::error::{BackupError, Result};
use crate::index::{BackupEntry, FingerprintField, open_index};

/// Replays a backup chain (base + diffs) and returns the resulting plaintext bytes.
///
/// `chain` must be ordered base-first (as returned by `BackupIndex::chain_to`).
/// Each file is read from `backups_dir`, optionally decrypted under the chain DEK,
/// decompressed, and diffs are applied sequentially over the base.
pub(crate) fn replay_chain(
    backups_dir: &Path,
    chain: &[&BackupEntry],
    backup_key: &Option<[u8; 32]>,
) -> Result<Vec<u8>> {
    if backup_key.is_none()
        && let Some(encrypted) = chain.iter().find(|e| e.encrypted)
    {
        return Err(BackupError::EncryptedWithoutPasskey {
            id: encrypted.id.clone(),
        });
    }

    let base_entry = chain[0];
    let base_bytes = std::fs::read(backups_dir.join(&base_entry.filename)).map_err(|source| {
        BackupError::ReplayReadBase {
            filename: base_entry.filename.clone(),
            source,
        }
    })?;

    // Resolve the chain DEK and the base payload slice. Precedence: self-describing
    // header (type 3) -> index wrapped_dek (type 2) -> legacy KEK-direct (type 1).
    let type3_result = match backup_key {
        Some(kek) if base_entry.encrypted => match crate::format::decode_base_blob(&base_bytes) {
            Some((wrapped, payload)) => {
                // A matched magic whose DEK fails to unwrap (a ~2^-32 nonce collision,
                // or a wrong KEK) falls through to the index/legacy paths, which surface
                // any genuine authentication error via `?`.
                crate::crypto::unwrap_dek(&wrapped, kek)
                    .ok()
                    .map(|dek| (dek, payload))
            }
            None => None,
        },
        _ => None,
    };

    let (chain_dek, base_payload): (Option<[u8; 32]>, &[u8]) = match (backup_key, type3_result) {
        (_, Some((dek, payload))) => (Some(dek), payload),
        (Some(kek), None) if base_entry.encrypted => {
            let dek = match base_entry.wrapped_dek.as_ref() {
                Some(wrapped) => crate::crypto::unwrap_dek(wrapped, kek)?,
                None => *kek,
            };
            (Some(dek), base_bytes.as_slice())
        }
        _ => (None, base_bytes.as_slice()),
    };

    let base_decrypted = match chain_dek.as_ref() {
        Some(key) => crate::crypto::decrypt_payload(base_payload, key)?,
        None => base_payload.to_vec(),
    };
    let mut plaintext = crate::diff::decompress(&base_decrypted)?;

    for diff_entry in &chain[1..] {
        let diff_bytes =
            std::fs::read(backups_dir.join(&diff_entry.filename)).map_err(|source| {
                BackupError::ReplayReadDiff {
                    filename: diff_entry.filename.clone(),
                    source,
                }
            })?;

        let diff_decrypted = match chain_dek.as_ref() {
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
/// The pre-restore safety backup is written atomically (copy to `.tmp`, then rename) to
/// `data_dir/pre_restore/pre-restore-<timestamp>.db`. It lives outside `data_dir/backups/`
/// so that `parse_backup_filename` and retention logic never attempt to index or prune it.
///
/// When `passkey` is provided, backup files are decrypted before use and the restored database
/// is written as an encrypted SQLCipher database using the DB key derived from that passkey.
/// When `passkey` is None, backup files are read as plaintext and the restored database is
/// written as a plaintext SQLite file.
pub fn restore_to_point(
    data_dir: &Path,
    target_id: &str,
    passkey: Option<&str>,
    archived_passkey: Option<&str>,
) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    let index_path = backups_dir.join("index.json");
    let backup_key = crate::crypto::resolve_backup_key(data_dir, passkey)?;
    let index = open_index(&index_path, backup_key.as_ref())?;

    let current_fp = backup_key
        .as_ref()
        .map(crate::crypto::compute_kek_fingerprint);
    let target_entry =
        index
            .find_entry(target_id)
            .ok_or_else(|| BackupError::RestoreUnknownId {
                id: target_id.to_owned(),
            })?;
    let root =
        if target_entry.entry_type == crate::index::BackupType::Base {
            target_entry
        } else {
            let base_id = target_entry.base_id.as_deref().ok_or_else(|| {
                BackupError::RestoreDiffNoBaseId {
                    id: target_entry.id.clone(),
                }
            })?;
            index
                .find_entry(base_id)
                .ok_or_else(|| BackupError::RestoreChainRootMissing {
                    id: base_id.to_owned(),
                })?
        };

    let effective_kek: Option<[u8; 32]> = match &root.kek_fingerprint {
        None => backup_key,
        Some(FingerprintField::Known(fp)) if Some(fp) == current_fp.as_ref() => backup_key,
        Some(_) if backup_key.is_none() => {
            // No passkey was given at all; let replay_chain produce the standard
            // "encrypted but no passkey" error rather than the archived-chain error.
            backup_key
        }
        Some(other) => {
            let pw = archived_passkey.ok_or_else(|| archived_chain_error(target_id, other))?;
            crate::crypto::resolve_backup_key(data_dir, Some(pw))?
        }
    };

    let chain = index.chain_to(target_id)?;

    let plaintext = replay_chain(&backups_dir, &chain, &effective_kek)?;

    let target_entry = chain.last().expect("chain is non-empty");
    let actual_hash = crate::hash::hash_bytes(&plaintext);
    if actual_hash != target_entry.plaintext_hash {
        return Err(BackupError::RestoreHashMismatch {
            expected: target_entry.plaintext_hash.clone(),
            actual: actual_hash,
        });
    }

    let db_path = data_dir.join("data.db");
    if db_path.exists() {
        let pre_restore_dir = data_dir.join("pre_restore");
        std::fs::create_dir_all(&pre_restore_dir).map_err(BackupError::CreatePreRestoreDir)?;
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let safety_path = pre_restore_dir.join(format!("pre-restore-{timestamp}.db"));
        let tmp_path = {
            let mut s = safety_path.as_os_str().to_owned();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        std::fs::copy(&db_path, &tmp_path).map_err(BackupError::StagePreRestoreBackup)?;
        std::fs::rename(&tmp_path, &safety_path).map_err(BackupError::CommitPreRestoreBackup)?;
    }

    match passkey {
        None => {
            libllm::crypto::write_atomic(&db_path, &plaintext)
                .map_err(BackupError::WriteRestoredDatabase)?;
        }
        Some(pk) => {
            let temp_plain =
                tempfile::NamedTempFile::new().map_err(BackupError::RestoreTempFile)?;
            let temp_plain_path = temp_plain.path().to_path_buf();
            std::fs::write(&temp_plain_path, &plaintext)
                .map_err(BackupError::RestoreTempFileWrite)?;

            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))
                .map_err(BackupError::LibllmCrypto)?;
            let db_key =
                libllm::crypto::derive_key(pk, &salt).map_err(BackupError::LibllmCrypto)?;

            // Remove the existing DB file so the destination connection creates a fresh
            // unencrypted database. We then use sqlcipher_export to write an encrypted copy.
            if db_path.exists() {
                std::fs::remove_file(&db_path).map_err(BackupError::RemoveExistingDatabase)?;
            }

            // Open the plaintext source and export it directly into an encrypted destination.
            // SQLCipher's backup API does not support plaintext->encrypted transfers, so we use
            // ATTACH + sqlcipher_export which is the canonical SQLCipher migration path.
            let src = rusqlite::Connection::open(&temp_plain_path)
                .map_err(BackupError::OpenPlaintextTempDb)?;
            let key_hex = db_key.hex();
            let attach_sql = Zeroizing::new(format!(
                "ATTACH DATABASE '{}' AS encrypted KEY \"x'{}'\";\
                 SELECT sqlcipher_export('encrypted');\
                 DETACH DATABASE encrypted;",
                db_path.display().to_string().replace('\'', "''"),
                &*key_hex,
            ));
            src.execute_batch(&attach_sql)
                .map_err(BackupError::ExportAsEncrypted)?;
        }
    }

    Ok(())
}

fn archived_chain_error(target_id: &str, fingerprint: &FingerprintField) -> BackupError {
    match fingerprint {
        FingerprintField::Known(fp) => BackupError::ArchivedChainKnown {
            target_id: target_id.to_owned(),
            fingerprint: fp.clone(),
        },
        FingerprintField::Unknown => BackupError::ArchivedChainUnknown {
            target_id: target_id.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::load_index;

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

        restore_to_point(dir.path(), &base_id, None, None).unwrap();

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

        restore_to_point(dir.path(), &diff_id, None, None).unwrap();

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

        restore_to_point(dir.path(), &target_id, None, None).unwrap();

        // The safety backup now lives in data_dir/pre_restore/, not in backups/.
        let pre_restore_dir = dir.path().join("pre_restore");
        let pre_restore_exists = std::fs::read_dir(&pre_restore_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("pre-restore-"));

        assert!(
            pre_restore_exists,
            "expected a pre-restore-* file in pre_restore dir"
        );
    }

    #[test]
    fn replay_reads_type2_base_via_index_wrapped_dek() {
        let dir = tempfile::TempDir::new().unwrap();
        let data_dir = dir.path();
        let kek = crate::crypto::resolve_backup_key(data_dir, Some("pw"))
            .unwrap()
            .unwrap();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        let plaintext = b"type-2 database bytes that must round-trip";
        let compressed = crate::diff::compress(plaintext).unwrap();
        let dek = crate::crypto::generate_dek();
        let payload = crate::crypto::encrypt_payload(&compressed, &dek).unwrap();
        let id = "20260601T070000.000Z".to_string();
        let filename = crate::index::backup_filename(&id, crate::index::BackupType::Base);
        libllm::crypto::write_atomic(&backups_dir.join(&filename), &payload).unwrap();
        let wrapped = crate::crypto::wrap_dek(&dek, &kek).unwrap();

        let index = crate::index::BackupIndex {
            version: crate::index::SCHEMA_VERSION,
            entries: vec![crate::index::BackupEntry {
                id: id.clone(),
                entry_type: crate::index::BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: crate::hash::hash_bytes(plaintext),
                file_hash: crate::hash::hash_bytes(&payload),
                plaintext_size: plaintext.len() as u64,
                stored_size: payload.len() as u64,
                encrypted: true,
                created_at: chrono::Utc::now(),
                wrapped_dek: Some(wrapped),
                kek_fingerprint: None,
            }],
        };

        let chain = index.chain_to(&id).unwrap();
        let restored = replay_chain(&backups_dir, &chain, &Some(kek)).unwrap();
        assert_eq!(
            restored, plaintext,
            "type-2 base must restore via the index wrapped DEK"
        );
    }

    #[test]
    fn replay_reads_self_describing_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let data_dir = dir.path();

        let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt")).unwrap();
        let key = libllm::crypto::derive_key("pw", &salt).unwrap();
        let db_path = data_dir.join("data.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&key.key_pragma()).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);")
            .unwrap();
        conn.execute("INSERT INTO test (value) VALUES (?1)", ["hello"])
            .unwrap();
        drop(conn);

        let config = crate::BackupConfig::default();
        crate::snapshot::create_snapshot(data_dir, Some("pw"), &config).unwrap();

        let backups_dir = data_dir.join("backups");
        let index = crate::index::load_index(&backups_dir.join("index.json")).unwrap();
        let base = index
            .entries
            .iter()
            .find(|e| e.entry_type == crate::index::BackupType::Base)
            .unwrap();

        let kek = crate::crypto::resolve_backup_key(data_dir, Some("pw"))
            .unwrap()
            .unwrap();
        let chain = index.chain_to(&base.id).unwrap();
        let plaintext = replay_chain(&backups_dir, &chain, &Some(kek)).unwrap();
        assert!(
            !plaintext.is_empty(),
            "restored plaintext from a type-3 base must be non-empty"
        );
    }
}

#[cfg(test)]
mod archived_tests {
    use crate::crypto::{compute_kek_fingerprint, encrypt_payload, resolve_backup_key, wrap_dek};
    use crate::index::{
        BackupEntry, BackupIndex, BackupType, FingerprintField, SCHEMA_VERSION, backup_filename,
        save_index,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn restore_refuses_archived_chain_without_archived_passkey() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        let _current_kek = resolve_backup_key(data_dir, Some("current"))
            .unwrap()
            .unwrap();
        let foreign_kek = resolve_backup_key(data_dir, Some("foreign"))
            .unwrap()
            .unwrap();
        let dek = [7u8; 32];
        let id = "20260421T030000.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        libllm::crypto::write_atomic(
            &backups_dir.join(&filename),
            &encrypt_payload(b"x", &dek).unwrap(),
        )
        .unwrap();

        let index = BackupIndex {
            version: SCHEMA_VERSION,
            entries: vec![BackupEntry {
                id: id.clone(),
                entry_type: BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: "u".into(),
                file_hash: "u".into(),
                plaintext_size: 1,
                stored_size: 0,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: Some(wrap_dek(&dek, &foreign_kek).unwrap()),
                kek_fingerprint: Some(FingerprintField::Known(compute_kek_fingerprint(
                    &foreign_kek,
                ))),
            }],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        let err = super::restore_to_point(data_dir, &id, Some("current"), None).unwrap_err();
        assert!(err.to_string().contains("archived"));
    }
}
