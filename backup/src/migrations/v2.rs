use anyhow::{Context, Result};
use rand::TryRngCore;
use std::path::{Path, PathBuf};

use crate::crypto::{compute_kek_fingerprint, decrypt_payload, encrypt_payload, wrap_dek};
use crate::index::{BackupIndex, BackupType, FingerprintField};

pub(super) fn migrate(
    index: &mut BackupIndex,
    backups_dir: &Path,
    kek: Option<&[u8; 32]>,
) -> Result<()> {
    match kek {
        Some(k) => migrate_encrypted(index, backups_dir, k),
        None => migrate_unencrypted(index),
    }
}

fn migrate_encrypted(
    index: &mut BackupIndex,
    backups_dir: &Path,
    kek: &[u8; 32],
) -> Result<()> {
    // Staging + batch-rename + late root stamp means a mid-chain crash is
    // recoverable: the idempotency guard re-enters the chain, generates a
    // new DEK, and overwrites any orphaned .tmp files from the prior attempt.
    let fingerprint = compute_kek_fingerprint(kek);
    let chain_roots: Vec<String> = index
        .entries
        .iter()
        .filter(|e| e.entry_type == BackupType::Base)
        .map(|e| e.id.clone())
        .collect();

    for root_id in chain_roots {
        let root_done = index
            .find_entry(&root_id)
            .expect("root_id was collected from the same index")
            .wrapped_dek
            .is_some();
        if root_done {
            continue;
        }

        let chain_ids = collect_chain_ids(index, &root_id);
        let dek = generate_dek()?;
        let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();

        for entry_id in &chain_ids {
            let entry = index
                .find_entry(entry_id)
                .with_context(|| format!("entry {entry_id} missing from index"))?;
            let src = backups_dir.join(&entry.filename);
            let ciphertext = std::fs::read(&src)
                .with_context(|| format!("read {}", src.display()))?;
            let plaintext = decrypt_payload(&ciphertext, kek)
                .with_context(|| format!("decrypt {entry_id} with current KEK"))?;
            let new_blob = encrypt_payload(&plaintext, &dek)
                .with_context(|| format!("re-encrypt {entry_id} under DEK"))?;
            let dst_tmp = backups_dir.join(format!("{}.tmp", entry.filename));
            libllm::crypto::write_atomic(&dst_tmp, &new_blob)
                .with_context(|| format!("stage {}", dst_tmp.display()))?;
            staged.push((dst_tmp, src));
        }

        for (tmp, final_path) in staged {
            std::fs::rename(&tmp, &final_path)
                .with_context(|| format!("rename {} -> {}", tmp.display(), final_path.display()))?;
        }

        let wrapped = wrap_dek(&dek, kek)?;
        let root = index
            .entries
            .iter_mut()
            .find(|e| e.id == root_id)
            .expect("root_id was collected from the index moments ago");
        root.wrapped_dek = Some(wrapped);
        root.kek_fingerprint = Some(FingerprintField::Known(fingerprint.clone()));
    }

    Ok(())
}

fn migrate_unencrypted(_index: &mut BackupIndex) -> Result<()> {
    // Unencrypted payloads need no re-encryption; only the SCHEMA_VERSION
    // stamp changes, which run_migrations handles after this returns.
    Ok(())
}

/// Collects the chain root and every Diff that points directly at it. Assumes
/// a flat chain model (all Diffs in a chain share the same `base_id` pointing
/// to the Base). Snapshots in this codebase always produce that shape; a
/// hypothetical future Diff-chained-on-Diff layout would require walking
/// transitively, which this function does not.
fn collect_chain_ids(index: &BackupIndex, root_id: &str) -> Vec<String> {
    let mut ids = vec![root_id.to_string()];
    for entry in &index.entries {
        if entry.entry_type != BackupType::Diff {
            continue;
        }
        if let Some(base) = &entry.base_id
            && base == root_id
        {
            ids.push(entry.id.clone());
        }
    }
    ids
}

fn generate_dek() -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    rand::rng()
        .try_fill_bytes(&mut bytes)
        .context("RNG fill_bytes failed for DEK")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::migrate;
    use crate::crypto::{encrypt_payload, resolve_backup_key, unwrap_dek};
    use crate::index::{
        backup_filename, save_index, BackupEntry, BackupIndex, BackupType, FingerprintField,
    };
    use chrono::Utc;
    use libllm::crypto::write_atomic;
    use tempfile::TempDir;

    fn fake_payload(plaintext: &[u8], key: &[u8; 32]) -> Vec<u8> {
        encrypt_payload(plaintext, key).unwrap()
    }

    #[test]
    fn migrates_encrypted_single_base_chain() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();

        let kek = resolve_backup_key(data_dir, Some("pw"))
            .unwrap()
            .expect("passkey -> key");

        let plaintext = b"hello world" as &[u8];
        let payload = fake_payload(plaintext, &kek);
        let id = "20260421T000000.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), &payload).unwrap();

        let mut index = BackupIndex {
            version: 1,
            entries: vec![BackupEntry {
                id,
                entry_type: BackupType::Base,
                filename: filename.clone(),
                base_id: None,
                plaintext_hash: "unused".into(),
                file_hash: "unused".into(),
                plaintext_size: plaintext.len() as u64,
                stored_size: payload.len() as u64,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: None,
                kek_fingerprint: None,
            }],
        };

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();

        let entry = &index.entries[0];
        let wrapped = entry.wrapped_dek.as_ref().expect("DEK wrapped");
        let dek = unwrap_dek(wrapped, &kek).unwrap();
        assert!(matches!(entry.kek_fingerprint, Some(FingerprintField::Known(_))));

        let bytes = std::fs::read(backups_dir.join(&filename)).unwrap();
        let recovered = crate::crypto::decrypt_payload(&bytes, &dek).unwrap();
        assert_eq!(recovered, plaintext);

        save_index(&backups_dir.join("index.json"), &index).unwrap();
    }

    #[test]
    fn is_idempotent_when_chain_already_migrated() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let kek = resolve_backup_key(data_dir, Some("pw")).unwrap().unwrap();

        let plaintext = b"abc" as &[u8];
        let id = "20260421T000001.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        write_atomic(&backups_dir.join(&filename), &fake_payload(plaintext, &kek)).unwrap();
        let mut index = BackupIndex {
            version: 1,
            entries: vec![BackupEntry {
                id,
                entry_type: BackupType::Base,
                filename,
                base_id: None,
                plaintext_hash: "u".into(),
                file_hash: "u".into(),
                plaintext_size: plaintext.len() as u64,
                stored_size: 0,
                encrypted: true,
                created_at: Utc::now(),
                wrapped_dek: None,
                kek_fingerprint: None,
            }],
        };
        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();
        let snapshot = index.entries[0].wrapped_dek.clone();

        migrate(&mut index, &backups_dir, Some(&kek)).unwrap();
        assert_eq!(index.entries[0].wrapped_dek, snapshot);
    }
}
