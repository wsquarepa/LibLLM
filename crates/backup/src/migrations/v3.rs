use std::path::Path;

use crate::error::{BackupError, Result};
use crate::index::{BackupIndex, BackupType, save_index};

/// v2 -> v3: rewrite each encrypted base file into the self-describing header
/// format by prepending its wrapped DEK (already present in the index) ahead of
/// the unchanged payload. Idempotent: files already carrying an authenticated
/// header are left untouched (size/hash may still be reconciled). The KEK is
/// required for encrypted indexes both to gate the rewrite and to authenticate
/// any pre-existing header before reconciling metadata.
pub(super) fn migrate(
    index: &mut BackupIndex,
    backups_dir: &Path,
    kek: Option<&[u8; 32]>,
) -> Result<()> {
    let has_encrypted = index.entries.iter().any(|e| e.encrypted);
    if has_encrypted && kek.is_none() {
        return Err(BackupError::MigrationV3EncryptedNoKek);
    }

    let index_path = backups_dir.join("index.json");
    let base_ids: Vec<String> = index
        .entries
        .iter()
        .filter(|e| e.entry_type == BackupType::Base && e.encrypted)
        .map(|e| e.id.clone())
        .collect();

    for base_id in base_ids {
        let entry = index
            .find_entry(&base_id)
            .expect("base_id was collected from the same index");
        let path = backups_dir.join(&entry.filename);
        let bytes = std::fs::read(&path).map_err(|source| BackupError::MigrationReadFile {
            path: path.clone(),
            source,
        })?;

        if let Some((header_wrapped, _payload)) = crate::format::decode_base_blob(&bytes) {
            // The file already carries a header (e.g. a prior run wrote it but
            // crashed before persisting the index). Authenticate the header
            // against the index wrapped DEK before reconciling size/hash, so a
            // syntactically valid but tampered header cannot bless a new file_hash.
            let kek = kek.ok_or(BackupError::MigrationV3EncryptedNoKek)?;
            let index_wrapped =
                entry
                    .wrapped_dek
                    .clone()
                    .ok_or_else(|| BackupError::MigrationV3MissingWrappedDek {
                        id: base_id.clone(),
                    })?;
            let header_dek = crate::crypto::unwrap_dek(&header_wrapped, kek)?;
            let index_dek = crate::crypto::unwrap_dek(&index_wrapped, kek)?;
            if header_dek != index_dek {
                return Err(BackupError::MigrationV3HeaderMismatch {
                    id: base_id.clone(),
                });
            }

            let correct_size = bytes.len() as u64;
            let correct_hash = crate::hash::hash_bytes(&bytes);
            let entry = index
                .entries
                .iter_mut()
                .find(|e| e.id == base_id)
                .expect("base_id was collected from the same index");
            if entry.stored_size != correct_size || entry.file_hash != correct_hash {
                entry.stored_size = correct_size;
                entry.file_hash = correct_hash;
                save_index(&index_path, index).map_err(|source| {
                    BackupError::MigrationV3PersistReconciled {
                        id: base_id.clone(),
                        source: Box::new(source),
                    }
                })?;
            }
            continue;
        }

        let wrapped =
            entry
                .wrapped_dek
                .clone()
                .ok_or_else(|| BackupError::MigrationV3MissingWrappedDek {
                    id: base_id.clone(),
                })?;
        let new_blob = crate::format::encode_base_blob(&wrapped, &bytes)?;
        libllm_core::crypto::write_atomic(&path, &new_blob).map_err(|source| {
            BackupError::MigrationV3RewriteFile {
                path: path.clone(),
                source,
            }
        })?;

        let entry = index
            .entries
            .iter_mut()
            .find(|e| e.id == base_id)
            .expect("base_id was collected from the same index");
        entry.stored_size = new_blob.len() as u64;
        entry.file_hash = crate::hash::hash_bytes(&new_blob);

        // Persist per base so a mid-migration crash leaves completed files flagged
        // (decode_base_blob is Some), which the idempotency guard skips on retry.
        save_index(&index_path, index).map_err(|source| {
            BackupError::MigrationV3PersistEmbedded {
                id: base_id.clone(),
                source: Box::new(source),
            }
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::migrate;
    use crate::crypto::{encrypt_payload, generate_dek, resolve_backup_key, wrap_dek};
    use crate::index::{BackupEntry, BackupIndex, BackupType, backup_filename, save_index};
    use chrono::Utc;
    use libllm_core::crypto::write_atomic;
    use tempfile::TempDir;

    fn type2_base(data_dir: &std::path::Path, kek: &[u8; 32], id: &str) -> (BackupEntry, Vec<u8>) {
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let dek = generate_dek();
        let payload = encrypt_payload(b"db-bytes", &dek).unwrap();
        let filename = backup_filename(id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), &payload).unwrap();
        let wrapped = wrap_dek(&dek, kek).unwrap();
        let entry = BackupEntry {
            id: id.to_string(),
            entry_type: BackupType::Base,
            filename,
            base_id: None,
            plaintext_hash: "u".into(),
            file_hash: "u".into(),
            plaintext_size: 8,
            stored_size: payload.len() as u64,
            encrypted: true,
            created_at: Utc::now(),
            wrapped_dek: Some(wrapped),
            kek_fingerprint: None,
        };
        (entry, payload)
    }

    #[test]
    fn embeds_header_for_type2_base() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let (entry, _payload) = type2_base(data_dir, &kek, "20260601T010000.000Z");
        let backups_dir = data_dir.join("backups");
        let filename = entry.filename.clone();
        let mut index = BackupIndex {
            version: 2,
            entries: vec![entry],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();

        let bytes = std::fs::read(backups_dir.join(&filename)).unwrap();
        assert!(
            crate::format::decode_base_blob(&bytes).is_some(),
            "migrated base must carry a header"
        );
        assert_eq!(index.entries[0].stored_size, bytes.len() as u64);
        assert_eq!(index.entries[0].file_hash, crate::hash::hash_bytes(&bytes));
    }

    #[test]
    fn is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let (entry, _payload) = type2_base(data_dir, &kek, "20260601T020000.000Z");
        let backups_dir = data_dir.join("backups");
        let filename = entry.filename.clone();
        let mut index = BackupIndex {
            version: 2,
            entries: vec![entry],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();
        let after_first = std::fs::read(backups_dir.join(&filename)).unwrap();
        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();
        let after_second = std::fs::read(backups_dir.join(&filename)).unwrap();
        assert_eq!(
            after_first, after_second,
            "second run must not change the file"
        );
    }

    #[test]
    fn unencrypted_is_noop() {
        let tmp = TempDir::new().unwrap();
        let backups_dir = tmp.path().join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let id = "20260601T030000.000Z";
        let filename = backup_filename(id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), b"plain").unwrap();
        let mut index = BackupIndex {
            version: 2,
            entries: vec![BackupEntry {
                id: id.to_string(),
                entry_type: BackupType::Base,
                filename: filename.clone(),
                base_id: None,
                plaintext_hash: "u".into(),
                file_hash: "u".into(),
                plaintext_size: 5,
                stored_size: 5,
                encrypted: false,
                created_at: Utc::now(),
                wrapped_dek: None,
                kek_fingerprint: None,
            }],
        };
        migrate(&mut index, &backups_dir, None).unwrap();
        assert_eq!(
            std::fs::read(backups_dir.join(&filename)).unwrap(),
            b"plain"
        );
    }

    #[test]
    fn bails_encrypted_without_kek() {
        let tmp = TempDir::new().unwrap();
        let backups_dir = tmp.path().join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let id = "20260601T040000.000Z";
        let filename = backup_filename(id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), b"cipher").unwrap();
        let mut index = BackupIndex {
            version: 2,
            entries: vec![BackupEntry {
                id: id.to_string(),
                entry_type: BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: "u".into(),
                file_hash: "u".into(),
                plaintext_size: 0,
                stored_size: 0,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: None,
                kek_fingerprint: None,
            }],
        };
        let err = migrate(&mut index, &backups_dir, None).unwrap_err();
        assert!(err.to_string().contains("without a KEK"));
    }

    #[test]
    fn reconciles_metadata_when_header_already_present() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let (mut entry, payload) = type2_base(data_dir, &kek, "20260601T050000.000Z");
        let backups_dir = data_dir.join("backups");
        let filename = entry.filename.clone();

        // Simulate a crash after the file was rewritten with the header but before
        // the index was persisted: the on-disk file has the header, the index does not.
        // Header and index share the same DEK (re-wrapped for a distinct blob).
        let index_wrapped = entry.wrapped_dek.clone().unwrap();
        let index_dek = crate::crypto::unwrap_dek(&index_wrapped, &kek).unwrap();
        let header_wrapped = wrap_dek(&index_dek, &kek).unwrap();
        let headered = crate::format::encode_base_blob(&header_wrapped, &payload).unwrap();
        write_atomic(&backups_dir.join(&filename), &headered).unwrap();
        entry.stored_size = payload.len() as u64;
        entry.file_hash = crate::hash::hash_bytes(&payload);

        let mut index = BackupIndex {
            version: 2,
            entries: vec![entry],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();

        let on_disk = std::fs::read(backups_dir.join(&filename)).unwrap();
        assert_eq!(
            index.entries[0].stored_size,
            on_disk.len() as u64,
            "stale stored_size must be reconciled to the headered file"
        );
        assert_eq!(
            index.entries[0].file_hash,
            crate::hash::hash_bytes(&on_disk),
            "stale file_hash must be reconciled to the headered file"
        );
    }

    #[test]
    fn rejects_tampered_header_without_updating_file_hash() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let (mut entry, payload) = type2_base(data_dir, &kek, "20260601T070000.000Z");
        let backups_dir = data_dir.join("backups");
        let filename = entry.filename.clone();

        // Syntactically valid type-3 header whose wrapped DEK decrypts under the
        // KEK but yields a different DEK than the index. Must hard-error and
        // leave the index file_hash untouched.
        let foreign_dek = generate_dek();
        let foreign_wrapped = wrap_dek(&foreign_dek, &kek).unwrap();
        let tampered = crate::format::encode_base_blob(&foreign_wrapped, &payload).unwrap();
        write_atomic(&backups_dir.join(&filename), &tampered).unwrap();

        let stale_size = payload.len() as u64;
        let stale_hash = crate::hash::hash_bytes(&payload);
        entry.stored_size = stale_size;
        entry.file_hash = stale_hash.clone();

        let mut index = BackupIndex {
            version: 2,
            entries: vec![entry],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        let err = migrate(&mut index, &backups_dir, Some(&kek)).unwrap_err();
        match &err {
            crate::error::BackupError::MigrationV3HeaderMismatch { id } => {
                assert_eq!(id, "20260601T070000.000Z");
            }
            other => panic!("expected MigrationV3HeaderMismatch, got: {other}"),
        }
        assert_eq!(
            index.entries[0].file_hash, stale_hash,
            "tampered header must not update file_hash"
        );
        assert_eq!(
            index.entries[0].stored_size, stale_size,
            "tampered header must not update stored_size"
        );
        // On-disk index must also remain unblessed.
        let reloaded =
            crate::index::load_index(&backups_dir.join("index.json")).unwrap();
        assert_eq!(reloaded.entries[0].file_hash, stale_hash);
        assert_eq!(reloaded.entries[0].stored_size, stale_size);
    }

    #[test]
    fn migrated_type2_base_restores_via_replay_chain() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        let plaintext = b"db bytes that survive a v3 migration and restore";
        let compressed = crate::diff::compress(plaintext).unwrap();
        let dek = generate_dek();
        let payload = encrypt_payload(&compressed, &dek).unwrap();
        let id = "20260601T090000.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), &payload).unwrap();
        let wrapped = wrap_dek(&dek, &kek).unwrap();

        let mut index = BackupIndex {
            version: 2,
            entries: vec![BackupEntry {
                id: id.clone(),
                entry_type: BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: crate::hash::hash_bytes(plaintext),
                file_hash: crate::hash::hash_bytes(&payload),
                plaintext_size: plaintext.len() as u64,
                stored_size: payload.len() as u64,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: Some(wrapped),
                kek_fingerprint: None,
            }],
        };
        save_index(&backups_dir.join("index.json"), &index).unwrap();

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();

        let chain = index.chain_to(&id).unwrap();
        let restored = crate::restore::replay_chain(&backups_dir, &chain, &Some(kek)).unwrap();
        assert_eq!(
            restored, plaintext,
            "v3-migrated type-2 base must restore to the original bytes"
        );
    }

    #[test]
    fn bails_with_kek_but_missing_wrapped_dek() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let id = "20260601T060000.000Z";
        let filename = backup_filename(id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), b"cipher-no-header").unwrap();
        let mut index = BackupIndex {
            version: 2,
            entries: vec![BackupEntry {
                id: id.to_string(),
                entry_type: BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: "u".into(),
                file_hash: "u".into(),
                plaintext_size: 0,
                stored_size: 0,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: None,
                kek_fingerprint: None,
            }],
        };
        let err = migrate(&mut index, &backups_dir, Some(&kek)).unwrap_err();
        assert!(
            err.to_string().contains("no wrapped DEK"),
            "expected a missing-wrapped-DEK error, got: {err}"
        );
    }
}
