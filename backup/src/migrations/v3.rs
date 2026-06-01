use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::index::{BackupIndex, BackupType, save_index};

/// v2 -> v3: rewrite each encrypted base file into the self-describing header
/// format by prepending its wrapped DEK (already present in the index) ahead of
/// the unchanged payload. Idempotent: files already carrying the header are left
/// untouched. The KEK is not needed for the rewrite; it is required only to refuse
/// migrating an encrypted index without one, matching the v2 contract.
pub(super) fn migrate(
    index: &mut BackupIndex,
    backups_dir: &Path,
    kek: Option<&[u8; 32]>,
) -> Result<()> {
    let has_encrypted = index.entries.iter().any(|e| e.encrypted);
    if has_encrypted && kek.is_none() {
        bail!(
            "cannot migrate encrypted backup index to v3 without a KEK: \
             re-run with a passkey set (LIBLLM_PASSKEY or --passkey)"
        );
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
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;

        if crate::format::decode_base_blob(&bytes).is_some() {
            continue;
        }

        let wrapped = entry.wrapped_dek.clone().ok_or_else(|| {
            anyhow::anyhow!("base {base_id} is encrypted but has no wrapped DEK to embed at v3")
        })?;
        let new_blob = crate::format::encode_base_blob(&wrapped, &bytes);
        libllm::crypto::write_atomic(&path, &new_blob)
            .with_context(|| format!("rewrite {} with header", path.display()))?;

        let entry = index
            .entries
            .iter_mut()
            .find(|e| e.id == base_id)
            .expect("base_id was collected from the same index");
        entry.stored_size = new_blob.len() as u64;
        entry.file_hash = crate::hash::hash_bytes(&new_blob);

        // Persist per base so a mid-migration crash leaves completed files flagged
        // (decode_base_blob is Some), which the idempotency guard skips on retry.
        save_index(&index_path, index)
            .with_context(|| format!("persist index after embedding header for {base_id}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::migrate;
    use crate::crypto::{encrypt_payload, generate_dek, resolve_backup_key, wrap_dek};
    use crate::index::{BackupEntry, BackupIndex, BackupType, backup_filename, save_index};
    use chrono::Utc;
    use libllm::crypto::write_atomic;
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
        assert_eq!(after_first, after_second, "second run must not change the file");
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
        assert_eq!(std::fs::read(backups_dir.join(&filename)).unwrap(), b"plain");
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
}
