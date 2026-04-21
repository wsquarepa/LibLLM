use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::crypto::{compute_kek_fingerprint, unwrap_dek, wrap_dek};
use crate::index::{open_index, save_index, BackupType, FingerprintField, WrappedDek};

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
    let bytes = serde_json::to_vec_pretty(journal).context("serialize rekey journal")?;
    libllm::crypto::write_atomic(&journal_path(backups_dir), &bytes)
        .context("atomic write of rekey journal")
}

pub fn read_journal(backups_dir: &Path) -> Result<Option<RekeyJournal>> {
    let p = journal_path(backups_dir);
    if !p.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&p).with_context(|| format!("read {}", p.display()))?;
    let j: RekeyJournal = serde_json::from_slice(&bytes).context("parse rekey journal")?;
    Ok(Some(j))
}

pub fn delete_journal(backups_dir: &Path) -> Result<()> {
    let p = journal_path(backups_dir);
    if p.exists() {
        std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
    }
    let s = sidecar_path(backups_dir);
    if s.exists() {
        std::fs::remove_file(&s).with_context(|| format!("remove {}", s.display()))?;
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

    let mut rewrapped: Vec<(String, WrappedDek)> = Vec::new();
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
        let wrapped = entry
            .wrapped_dek
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("active chain {} missing wrapped DEK", entry.id))?;
        let dek = unwrap_dek(wrapped, old_kek)
            .with_context(|| format!("unwrap DEK of active chain {}", entry.id))?;
        let new_wrapped = wrap_dek(&dek, new_kek)?;
        rewrapped.push((entry.id.clone(), new_wrapped));
    }

    if rewrapped.is_empty() {
        return Ok(());
    }

    for (id, new_wrapped) in rewrapped {
        let root = index
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .expect("id was collected from the same index");
        root.wrapped_dek = Some(new_wrapped);
        root.kek_fingerprint = Some(FingerprintField::Known(new_fp.clone()));
    }

    std::fs::copy(&index_path, sidecar_path(&backups_dir))
        .context("stage index.json.pre-rekey")?;
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
            .context("restore index.json from sidecar")?;
    }
    let j = journal_path(&backups_dir);
    if j.exists() {
        std::fs::remove_file(&j).context("remove journal during rollback")?;
    }
    Ok(())
}

/// Detects a partial-rekey state and converges to a clean state. Caller supplies
/// whichever KEK is active now (typically derived from the passkey the user just
/// entered). If the journal is absent, this is a no-op.
pub fn recover_journal_if_present(
    data_dir: &Path,
    current_kek: Option<&[u8; 32]>,
) -> Result<()> {
    let backups_dir = data_dir.join("backups");
    let journal = read_journal(&backups_dir)?;
    match journal {
        None => {
            // No in-flight rekey. If a sidecar exists, it's an orphan from a
            // crash between sidecar-copy and journal-write in a prior attempt.
            let sidecar = sidecar_path(&backups_dir);
            if sidecar.exists() {
                std::fs::remove_file(&sidecar)
                    .with_context(|| format!("remove orphan sidecar {}", sidecar.display()))?;
            }
            Ok(())
        }
        Some(journal) => {
            let fp = match current_kek {
                Some(k) => compute_kek_fingerprint(k),
                None => {
                    anyhow::bail!(
                        "rekey journal found but no KEK supplied; run with the passkey active at rekey time"
                    );
                }
            };
            if fp == journal.new_fp {
                delete_journal(&backups_dir)?;
                return Ok(());
            }
            if fp == journal.old_fp {
                rollback_rekey(data_dir)?;
                return Ok(());
            }
            anyhow::bail!(
                "rekey journal present but current passkey fingerprint ({fp}) matches neither old_fp ({}) nor new_fp ({}); cannot auto-recover",
                journal.old_fp,
                journal.new_fp
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{encrypt_payload, resolve_backup_key};
    use crate::index::{
        backup_filename, save_index as save_idx, BackupEntry, BackupIndex, SCHEMA_VERSION,
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

    fn make_populated(data_dir: &Path, passkey: &str) -> ([u8; 32], [u8; 32], String) {
        let backups_dir = data_dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let kek = resolve_backup_key(data_dir, Some(passkey)).unwrap().unwrap();
        let dek = [42u8; 32];
        let id = "20260421T020000.000Z".to_string();
        let filename = backup_filename(&id, BackupType::Base);
        let payload = encrypt_payload(b"hi", &dek).unwrap();
        libllm::crypto::write_atomic(&backups_dir.join(&filename), &payload).unwrap();
        let entry = BackupEntry {
            id: id.clone(),
            entry_type: BackupType::Base,
            filename,
            base_id: None,
            plaintext_hash: "u".into(),
            file_hash: "u".into(),
            plaintext_size: 2,
            stored_size: payload.len() as u64,
            encrypted: true,
            created_at: Utc::now(),
            wrapped_dek: Some(wrap_dek(&dek, &kek).unwrap()),
            kek_fingerprint: Some(FingerprintField::Known(compute_kek_fingerprint(&kek))),
        };
        let index = BackupIndex {
            version: SCHEMA_VERSION,
            entries: vec![entry],
        };
        save_idx(&backups_dir.join("index.json"), &index).unwrap();
        (kek, dek, id)
    }

    #[test]
    fn rekey_rewraps_active_chain_and_creates_journal_sidecar() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, dek, _id) = make_populated(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw")).unwrap().unwrap();

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
        let wrong_kek = resolve_backup_key(tmp.path(), Some("wrong")).unwrap().unwrap();
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw")).unwrap().unwrap();

        prepare_rekey(tmp.path(), &wrong_kek, &new_kek).unwrap();
        assert!(!journal_path(&tmp.path().join("backups")).exists());
        assert!(!sidecar_path(&tmp.path().join("backups")).exists());
    }

    #[test]
    fn recover_cleanup_when_current_matches_new_fp() {
        let tmp = TempDir::new().unwrap();
        let (old_kek, _dek, _id) = make_populated(tmp.path(), "old-pw");
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw")).unwrap().unwrap();
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
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw")).unwrap().unwrap();
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
        let new_kek = resolve_backup_key(tmp.path(), Some("new-pw")).unwrap().unwrap();
        let other_kek = resolve_backup_key(tmp.path(), Some("other")).unwrap().unwrap();
        prepare_rekey(tmp.path(), &old_kek, &new_kek).unwrap();
        let err = recover_journal_if_present(tmp.path(), Some(&other_kek)).unwrap_err();
        assert!(err.to_string().contains("matches neither"));
    }
}
