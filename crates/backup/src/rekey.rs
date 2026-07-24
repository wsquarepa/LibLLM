use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::crypto::{compute_kek_fingerprint, unwrap_dek, wrap_dek};
use crate::error::{BackupError, Result};
use crate::format::{decode_base_blob, encode_base_blob};
use crate::hash::hash_bytes;
use crate::index::{BackupType, FingerprintField, WrappedDek, open_index, save_index};

pub const JOURNAL_FILENAME: &str = ".rekey.journal";
pub const PRE_REKEY_SIDECAR: &str = "index.json.pre-rekey";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RekeyJournal {
    pub old_fp: String,
    pub new_fp: String,
}

pub fn journal_path(backups_dir: &Path) -> PathBuf {
    backups_dir.join(JOURNAL_FILENAME)
}

pub fn sidecar_path(backups_dir: &Path) -> PathBuf {
    backups_dir.join(PRE_REKEY_SIDECAR)
}

pub fn write_journal(backups_dir: &Path, journal: &RekeyJournal) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(BackupError::RekeyJournalSerialize)?;
    libllm_core::crypto::write_atomic(&journal_path(backups_dir), &bytes)
        .map_err(BackupError::RekeyJournalWrite)
}

pub fn read_journal(backups_dir: &Path) -> Result<Option<RekeyJournal>> {
    let p = journal_path(backups_dir);
    if !p.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&p).map_err(|source| BackupError::RekeyJournalRead {
        path: p.clone(),
        source,
    })?;
    let j: RekeyJournal = serde_json::from_slice(&bytes).map_err(BackupError::RekeyJournalParse)?;
    Ok(Some(j))
}

pub fn delete_journal(backups_dir: &Path) -> Result<()> {
    let p = journal_path(backups_dir);
    if p.exists() {
        std::fs::remove_file(&p).map_err(|source| BackupError::RekeyRemoveFile {
            path: p.clone(),
            source,
        })?;
    }
    let s = sidecar_path(backups_dir);
    if s.exists() {
        std::fs::remove_file(&s).map_err(|source| BackupError::RekeyRemoveFile {
            path: s.clone(),
            source,
        })?;
    }
    Ok(())
}

/// Rewrap all active chains from old_kek to new_kek, stage a sidecar copy of
/// the current index, and write both the new index and the journal. Does NOT
/// rekey the database — the caller must do that next, then call finalize_rekey.
pub fn prepare_rekey(data_dir: &Path, old_kek: &[u8; 32], new_kek: &[u8; 32]) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    if !backups_dir.exists() {
        return Ok(());
    }
    let index_path = backups_dir.join("index.json");
    if !index_path.exists() {
        return Ok(());
    }

    // open_index runs pending migrations + journal recovery using the OLD kek
    // (still the current key at this point). After it returns, the on-disk
    // index is at SCHEMA_VERSION and the journal/sidecar are clean.
    let mut index = open_index(&index_path, Some(old_kek))?;
    let old_fp = compute_kek_fingerprint(old_kek);
    let new_fp = compute_kek_fingerprint(new_kek);

    struct RewrapUpdate {
        id: String,
        new_wrapped: WrappedDek,
        file_hash: String,
        stored_size: u64,
    }

    let mut rewrapped: Vec<RewrapUpdate> = Vec::new();
    for entry in &index.entries {
        if entry.entry_type != BackupType::Base {
            continue;
        }
        let stored_fp = match &entry.kek_fingerprint {
            Some(FingerprintField::Known(fp)) => fp,
            _ => continue,
        };
        if stored_fp != &old_fp {
            continue;
        }

        let path = backups_dir.join(&entry.filename);
        let bytes = std::fs::read(&path).map_err(|source| BackupError::RekeyReadBase {
            id: entry.id.clone(),
            path: path.clone(),
            source,
        })?;

        let index_wrapped =
            entry
                .wrapped_dek
                .as_ref()
                .ok_or_else(|| BackupError::RekeyMissingWrappedDek {
                    id: entry.id.clone(),
                })?;

        // Type-3: payload follows the header. Legacy type-2: whole file is payload.
        // Prefer the on-disk header wrap; fall back to the index wrap so a crash after
        // rewriting a base file but before saving the index can still be retried.
        let (dek, payload) = match decode_base_blob(&bytes) {
            Some((header_wrapped, payload)) => {
                let dek = match unwrap_dek(&header_wrapped, old_kek) {
                    Ok(dek) => dek,
                    Err(_) => unwrap_dek(index_wrapped, old_kek).map_err(|source| {
                        BackupError::RekeyUnwrapDek {
                            id: entry.id.clone(),
                            source: Box::new(source),
                        }
                    })?,
                };
                (dek, payload.to_vec())
            }
            None => {
                let dek = unwrap_dek(index_wrapped, old_kek).map_err(|source| {
                    BackupError::RekeyUnwrapDek {
                        id: entry.id.clone(),
                        source: Box::new(source),
                    }
                })?;
                (dek, bytes)
            }
        };

        let new_wrapped = wrap_dek(&dek, new_kek)?;
        // Diff ciphertext is unchanged: only the base header's KEK wrap is rewritten.
        let new_blob = encode_base_blob(&new_wrapped, &payload)?;
        libllm_core::crypto::write_atomic(&path, &new_blob).map_err(|source| {
            BackupError::RekeyRewriteBase {
                id: entry.id.clone(),
                path: path.clone(),
                source,
            }
        })?;

        rewrapped.push(RewrapUpdate {
            id: entry.id.clone(),
            new_wrapped,
            file_hash: hash_bytes(&new_blob),
            stored_size: new_blob.len() as u64,
        });
    }

    if rewrapped.is_empty() {
        return Ok(());
    }

    for update in rewrapped {
        let root = index
            .entries
            .iter_mut()
            .find(|e| e.id == update.id)
            .expect("id was collected from the same index");
        root.wrapped_dek = Some(update.new_wrapped);
        root.kek_fingerprint = Some(FingerprintField::Known(new_fp.clone()));
        root.file_hash = update.file_hash;
        root.stored_size = update.stored_size;
    }

    std::fs::copy(&index_path, sidecar_path(&backups_dir))
        .map_err(BackupError::RekeyStageIndexSidecar)?;
    write_journal(&backups_dir, &RekeyJournal { old_fp, new_fp })?;
    save_index(&index_path, &index)?;
    Ok(())
}

/// Called after the caller's db.rekey() succeeds. Removes the journal + sidecar.
pub fn finalize_rekey(data_dir: &Path) -> Result<()> {
    delete_journal(&data_dir.join("backups"))
}

/// Called by the caller's rollback path if db.rekey() failed. Restores the
/// pre-rekey index.json and removes the journal.
pub fn rollback_rekey(data_dir: &Path) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    let sidecar = sidecar_path(&backups_dir);
    if sidecar.exists() {
        std::fs::rename(&sidecar, backups_dir.join("index.json"))
            .map_err(BackupError::RekeyRestoreIndexSidecar)?;
    }
    let j = journal_path(&backups_dir);
    if j.exists() {
        std::fs::remove_file(&j).map_err(|source| BackupError::RekeyRemoveFile {
            path: j.clone(),
            source,
        })?;
    }
    Ok(())
}

/// Detects a partial-rekey state and converges to a clean state. Caller supplies
/// whichever KEK is active now (typically derived from the passkey the user just
/// entered). If the journal is absent, this is a no-op.
pub fn recover_journal_if_present(data_dir: &Path, current_kek: Option<&[u8; 32]>) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    let journal = read_journal(&backups_dir)?;
    match journal {
        None => {
            // No in-flight rekey. If a sidecar exists, it's an orphan from a
            // crash between sidecar-copy and journal-write in a prior attempt.
            let sidecar = sidecar_path(&backups_dir);
            if sidecar.exists() {
                std::fs::remove_file(&sidecar).map_err(|source| {
                    BackupError::RemoveOrphanSidecar {
                        path: sidecar.clone(),
                        source,
                    }
                })?;
            }
            Ok(())
        }
        Some(journal) => {
            let fp = match current_kek {
                Some(k) => compute_kek_fingerprint(k),
                None => return Err(BackupError::RekeyJournalNoKek),
            };
            if fp == journal.new_fp {
                delete_journal(&backups_dir)?;
                return Ok(());
            }
            if fp == journal.old_fp {
                rollback_rekey(data_dir)?;
                return Ok(());
            }
            Err(BackupError::RekeyJournalUnresolvable(Box::new(
                crate::error::RekeyJournalUnresolvable {
                    current: fp,
                    old_fp: journal.old_fp.clone(),
                    new_fp: journal.new_fp.clone(),
                    sidecar: sidecar_path(&backups_dir),
                    index: backups_dir.join("index.json"),
                    journal: journal_path(&backups_dir),
                },
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{encrypt_payload, resolve_backup_key};
    use crate::format::decode_base_blob;
    use crate::hash::hash_bytes;
    use crate::index::{
        BackupEntry, BackupIndex, SCHEMA_VERSION, backup_filename, save_index as save_idx,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn journal_round_trip() {
        let tmp = TempDir::new().unwrap();
        let j = RekeyJournal {
            old_fp: "a".into(),
            new_fp: "b".into(),
        };
        write_journal(tmp.path(), &j).unwrap();
        assert_eq!(read_journal(tmp.path()).unwrap(), Some(j.clone()));
        delete_journal(tmp.path()).unwrap();
        assert_eq!(read_journal(tmp.path()).unwrap(), None);
    }

    /// Builds an encrypted type-3 base (header + payload) under `passkey`.
    fn make_populated(data_dir: &Path, passkey: &str) -> ([u8; 32], [u8; 32], String) {
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let kek = resolve_backup_key(data_dir, Some(passkey))
            .unwrap()
            .unwrap();
        let dek = [42u8; 32];
        let id = "20260421T020000.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        let payload = encrypt_payload(b"hi", &dek).unwrap();
        let wrapped = wrap_dek(&dek, &kek).unwrap();
        let blob = encode_base_blob(&wrapped, &payload).unwrap();
        libllm_core::crypto::write_atomic(&backups_dir.join(&filename), &blob).unwrap();
        let entry = BackupEntry {
            id: id.clone(),
            entry_type: BackupType::Base,
            filename,
            base_id: None,
            plaintext_hash: "u".into(),
            file_hash: hash_bytes(&blob),
            plaintext_size: 2,
            stored_size: blob.len() as u64,
            encrypted: true,
            created_at: Utc::now(),
            wrapped_dek: Some(wrapped),
            kek_fingerprint: Some(FingerprintField::Known(compute_kek_fingerprint(&kek))),
        };
        let index = BackupIndex {
            version: SCHEMA_VERSION,
            entries: vec![entry],
        };
        save_idx(&backups_dir.join("index.json"), &index).unwrap();
        (kek, dek, id)
    }

    /// Type-3 base plus a diff encrypted under the same chain DEK.
    fn make_populated_with_diff(
        data_dir: &Path,
        passkey: &str,
    ) -> ([u8; 32], [u8; 32], String, String) {
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let kek = resolve_backup_key(data_dir, Some(passkey))
            .unwrap()
            .unwrap();
        let dek = [7u8; 32];

        let base_plain = b"base-database-bytes";
        let base_compressed = crate::diff::compress(base_plain).unwrap();
        let base_payload = encrypt_payload(&base_compressed, &dek).unwrap();
        let base_id = "20260421T040000.000Z".to_string();
        let base_filename = backup_filename(&base_id, BackupType::Base);
        let base_wrapped = wrap_dek(&dek, &kek).unwrap();
        let base_blob = encode_base_blob(&base_wrapped, &base_payload).unwrap();
        libllm_core::crypto::write_atomic(&backups_dir.join(&base_filename), &base_blob).unwrap();

        let final_plain = b"base-database-bytes-with-diff";
        let patch = crate::diff::compute_diff(base_plain, final_plain).unwrap();
        let diff_compressed = crate::diff::compress(&patch).unwrap();
        let diff_payload = encrypt_payload(&diff_compressed, &dek).unwrap();
        let diff_id = "20260421T040001.000Z".to_string();
        let diff_filename = backup_filename(&diff_id, BackupType::Diff);
        libllm_core::crypto::write_atomic(&backups_dir.join(&diff_filename), &diff_payload)
            .unwrap();

        let base_entry = BackupEntry {
            id: base_id.clone(),
            entry_type: BackupType::Base,
            filename: base_filename,
            base_id: None,
            plaintext_hash: hash_bytes(base_plain),
            file_hash: hash_bytes(&base_blob),
            plaintext_size: base_plain.len() as u64,
            stored_size: base_blob.len() as u64,
            encrypted: true,
            created_at: Utc::now(),
            wrapped_dek: Some(base_wrapped),
            kek_fingerprint: Some(FingerprintField::Known(compute_kek_fingerprint(&kek))),
        };
        let diff_entry = BackupEntry {
            id: diff_id.clone(),
            entry_type: BackupType::Diff,
            filename: diff_filename,
            base_id: Some(base_id.clone()),
            plaintext_hash: hash_bytes(final_plain),
            file_hash: hash_bytes(&diff_payload),
            plaintext_size: final_plain.len() as u64,
            stored_size: diff_payload.len() as u64,
            encrypted: true,
            created_at: Utc::now(),
            wrapped_dek: None,
            kek_fingerprint: None,
        };
        let index = BackupIndex {
            version: SCHEMA_VERSION,
            entries: vec![base_entry, diff_entry],
        };
        save_idx(&backups_dir.join("index.json"), &index).unwrap();
        (kek, dek, base_id, diff_id)
    }

    #[test]
    fn rekey_rewraps_active_chain_and_creates_journal_sidecar() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, dek, _id) = make_populated(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();

        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();

        let backups_dir = tmp.path().join("backups");
        assert!(journal_path(&backups_dir).exists());
        assert!(sidecar_path(&backups_dir).exists());

        // Exercise finalize_rekey directly (not via open_index recovery).
        finalize_rekey(tmp.path()).unwrap();
        assert!(!journal_path(&backups_dir).exists());
        assert!(!sidecar_path(&backups_dir).exists());

        // Now verify the on-disk index content — read with load_index to avoid
        // running migrations/recovery (they're both no-ops at this point, but
        // the test intent is to inspect raw disk state).
        let idx = crate::index::load_index(&backups_dir.join("index.json")).unwrap();
        let wrapped = idx.entries[0].wrapped_dek.as_ref().unwrap();
        assert_eq!(unwrap_dek(wrapped, &new_kek).unwrap(), dek);
    }

    #[test]
    fn prepare_rekey_skips_entries_whose_fingerprint_does_not_match_old_kek() {
        // When wrong_kek is passed as old_kek, no entry fingerprints match it,
        // so prepare_rekey completes without rewrapping anything and without
        // writing the journal or sidecar.
        let tmp = TempDir::new().unwrap();
        let _ = make_populated(tmp.path(), "old-pw");
        let wrong_kek = resolve_backup_key(tmp.path(), Some("wrong"))
            .unwrap()
            .unwrap();
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();

        prepare_rekey(tmp.path(), &wrong_kek, &new_kek).unwrap();
        assert!(!journal_path(&tmp.path().join("backups")).exists());
        assert!(!sidecar_path(&tmp.path().join("backups")).exists());
    }

    #[test]
    fn recover_cleanup_when_current_matches_new_fp() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, _dek, _id) = make_populated(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();
        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();
        recover_journal_if_present(tmp.path(), Some(&new_kek)).unwrap();
        let backups_dir = tmp.path().join("backups");
        assert!(!journal_path(&backups_dir).exists());
        assert!(!sidecar_path(&backups_dir).exists());
    }

    #[test]
    fn recover_rollback_when_current_matches_old_fp() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, _dek, _id) = make_populated(tmp.path(), "old-pw");
        let pre = std::fs::read(tmp.path().join("backups/index.json")).unwrap();
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();
        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();
        recover_journal_if_present(tmp.path(), Some(&old_kek)).unwrap();
        let backups_dir = tmp.path().join("backups");
        assert!(!journal_path(&backups_dir).exists());
        assert!(!sidecar_path(&backups_dir).exists());
        let post = std::fs::read(backups_dir.join("index.json")).unwrap();
        assert_eq!(post, pre, "index.json restored from sidecar");
    }

    #[test]
    fn recover_errors_when_current_matches_neither() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, _dek, _id) = make_populated(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();
        let other_kek = resolve_backup_key(tmp.path(), Some("other"))
            .unwrap()
            .unwrap();
        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();
        let err = recover_journal_if_present(tmp.path(), Some(&other_kek)).unwrap_err();
        assert!(err.to_string().contains("matches neither"));
    }

    #[test]
    fn prepare_rekey_rewrites_base_headers_so_old_kek_cannot_unwrap() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, dek, base_id, diff_id) = make_populated_with_diff(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw"))
            .unwrap()
            .unwrap();

        let backups_dir = tmp.path().join("backups");
        let pre_index = crate::index::load_index(&backups_dir.join("index.json")).unwrap();
        let pre_base = pre_index.find_entry(&base_id).unwrap();
        let pre_bytes = std::fs::read(backups_dir.join(&pre_base.filename)).unwrap();
        let (pre_header, pre_payload) = decode_base_blob(&pre_bytes).expect("type-3 base");
        let pre_diff = pre_index.find_entry(&diff_id).unwrap();
        let pre_diff_bytes = std::fs::read(backups_dir.join(&pre_diff.filename)).unwrap();

        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();

        let post_index = crate::index::load_index(&backups_dir.join("index.json")).unwrap();
        let post_base = post_index.find_entry(&base_id).unwrap();
        let post_bytes = std::fs::read(backups_dir.join(&post_base.filename)).unwrap();
        let (post_header, post_payload) = decode_base_blob(&post_bytes).expect("type-3 base");

        assert_eq!(
            post_payload, pre_payload,
            "rekey must not rewrite ciphertext payload"
        );
        assert_eq!(
            std::fs::read(backups_dir.join(&pre_diff.filename)).unwrap(),
            pre_diff_bytes,
            "rekey must not rewrite diff ciphertext"
        );
        assert_eq!(post_base.file_hash, hash_bytes(&post_bytes));
        assert_eq!(post_base.stored_size, post_bytes.len() as u64);

        assert!(
            unwrap_dek(&pre_header, &old_kek).is_ok(),
            "pre-rekey header must unwrap under old KEK"
        );
        assert!(
            unwrap_dek(&post_header, &old_kek).is_err(),
            "post-rekey header must not unwrap under old KEK"
        );
        assert_eq!(unwrap_dek(&post_header, &new_kek).unwrap(), dek);
        assert_eq!(
            unwrap_dek(post_base.wrapped_dek.as_ref().unwrap(), &new_kek).unwrap(),
            dek
        );
        assert!(
            unwrap_dek(post_base.wrapped_dek.as_ref().unwrap(), &old_kek).is_err(),
            "post-rekey index wrap must not unwrap under old KEK"
        );

        let base_chain = post_index.chain_to(&base_id).unwrap();
        let diff_chain = post_index.chain_to(&diff_id).unwrap();
        assert!(
            crate::restore::replay_chain(&backups_dir, &base_chain, &Some(old_kek)).is_err(),
            "replay base with old KEK must fail after rekey"
        );
        assert!(
            crate::restore::replay_chain(&backups_dir, &diff_chain, &Some(old_kek)).is_err(),
            "replay diff with old KEK must fail after rekey"
        );
        let restored_base =
            crate::restore::replay_chain(&backups_dir, &base_chain, &Some(new_kek)).unwrap();
        let restored_diff =
            crate::restore::replay_chain(&backups_dir, &diff_chain, &Some(new_kek)).unwrap();
        assert_eq!(restored_base, b"base-database-bytes");
        assert_eq!(restored_diff, b"base-database-bytes-with-diff");
    }
}
