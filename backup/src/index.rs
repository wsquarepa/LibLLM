use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupType {
    Base,
    Diff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub entry_type: BackupType,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_id: Option<String>,
    pub plaintext_hash: String,
    pub file_hash: String,
    pub plaintext_size: u64,
    pub stored_size: u64,
    pub encrypted: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupIndex {
    pub version: u32,
    pub entries: Vec<BackupEntry>,
}

impl BackupIndex {
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }

    pub fn latest_base(&self) -> Option<&BackupEntry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.entry_type == BackupType::Base)
    }

    pub fn diffs_since_last_base(&self) -> usize {
        self.entries
            .iter()
            .rev()
            .take_while(|e| e.entry_type == BackupType::Diff)
            .count()
    }

    pub fn find_entry(&self, id: &str) -> Option<&BackupEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Returns the ordered chain of entries (base first, then diffs) leading to the target.
    ///
    /// Walks backward from the target entry until a base entry is found. Returns None if
    /// the target id does not exist or if the chain references a base_id that cannot be resolved.
    pub fn chain_to(&self, target_id: &str) -> Option<Vec<&BackupEntry>> {
        let target = self.find_entry(target_id)?;

        let mut chain: Vec<&BackupEntry> = Vec::new();
        let mut current = target;

        loop {
            chain.push(current);
            match current.entry_type {
                BackupType::Base => break,
                BackupType::Diff => {
                    let base_id = current.base_id.as_deref()?;
                    current = self.find_entry(base_id)?;
                }
            }
        }

        chain.reverse();
        Some(chain)
    }
}

/// Generates a backup identifier from the current UTC time.
///
/// Format: "YYYYMMDDTHHmmssZ" (16 characters, e.g. "20260414T153000Z").
pub fn generate_backup_id() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// Constructs the filename for a backup file given its id and type.
///
/// E.g. "20260414T153000Z-base.bak" or "20260414T153000Z-diff.bak".
pub fn backup_filename(id: &str, backup_type: BackupType) -> String {
    let suffix = match backup_type {
        BackupType::Base => "base",
        BackupType::Diff => "diff",
    };
    format!("{id}-{suffix}.bak")
}

/// Parses a backup filename back into its id and type components.
///
/// Returns None for any filename that does not match the expected pattern.
pub fn parse_backup_filename(filename: &str) -> Option<(String, BackupType)> {
    let stem = filename.strip_suffix(".bak")?;
    let (id, type_str) = stem.rsplit_once('-')?;

    let backup_type = match type_str {
        "base" => BackupType::Base,
        "diff" => BackupType::Diff,
        _ => return None,
    };

    Some((id.to_string(), backup_type))
}

/// Loads a `BackupIndex` from the given path.
///
/// Returns an empty index with version=1 when the file does not exist.
pub fn load_index(path: &Path) -> Result<BackupIndex> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BackupIndex::new()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read index file: {}", path.display()))
        }
    };

    serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse index file: {}", path.display()))
}

/// Persists a `BackupIndex` to the given path using an atomic write.
pub fn save_index(path: &Path, index: &BackupIndex) -> Result<()> {
    let data = serde_json::to_vec_pretty(index).context("failed to serialize index")?;
    libllm::crypto::write_atomic(path, &data)
        .with_context(|| format!("failed to write index file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_base_entry(id: &str) -> BackupEntry {
        BackupEntry {
            id: id.to_string(),
            entry_type: BackupType::Base,
            filename: backup_filename(id, BackupType::Base),
            base_id: None,
            plaintext_hash: "aabbcc".to_string(),
            file_hash: "ddeeff".to_string(),
            plaintext_size: 1024,
            stored_size: 512,
            encrypted: true,
            created_at: Utc::now(),
        }
    }

    fn make_diff_entry(id: &str, base_id: &str) -> BackupEntry {
        BackupEntry {
            id: id.to_string(),
            entry_type: BackupType::Diff,
            filename: backup_filename(id, BackupType::Diff),
            base_id: Some(base_id.to_string()),
            plaintext_hash: "112233".to_string(),
            file_hash: "445566".to_string(),
            plaintext_size: 100,
            stored_size: 50,
            encrypted: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn round_trip_empty_index() {
        let index = BackupIndex::new();
        let json = serde_json::to_string(&index).unwrap();
        let restored: BackupIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.version, 1);
        assert!(restored.entries.is_empty());
    }

    #[test]
    fn round_trip_base_entry() {
        let entry = make_base_entry("20260414T153000Z");
        let json = serde_json::to_string(&entry).unwrap();
        let restored: BackupEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, entry.id);
        assert_eq!(restored.entry_type, BackupType::Base);
        assert!(restored.base_id.is_none());
        // base_id must not appear in JSON for base entries
        assert!(!json.contains("base_id"));
    }

    #[test]
    fn round_trip_diff_entry() {
        let entry = make_diff_entry("20260414T153001Z", "20260414T153000Z");
        let json = serde_json::to_string(&entry).unwrap();
        let restored: BackupEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.entry_type, BackupType::Diff);
        assert_eq!(
            restored.base_id.as_deref(),
            Some("20260414T153000Z")
        );
    }

    #[test]
    fn generate_id_format() {
        let id = generate_backup_id();
        assert_eq!(id.len(), 16, "id must be 16 characters: {id}");
        assert!(id.ends_with('Z'), "id must end with Z: {id}");
        assert!(id.contains('T'), "id must contain T: {id}");
    }

    #[test]
    fn filename_generation_base() {
        let name = backup_filename("20260414T153000Z", BackupType::Base);
        assert_eq!(name, "20260414T153000Z-base.bak");
    }

    #[test]
    fn filename_generation_diff() {
        let name = backup_filename("20260414T153000Z", BackupType::Diff);
        assert_eq!(name, "20260414T153000Z-diff.bak");
    }

    #[test]
    fn parse_backup_filename_base() {
        let result = parse_backup_filename("20260414T153000Z-base.bak");
        assert_eq!(
            result,
            Some(("20260414T153000Z".to_string(), BackupType::Base))
        );
    }

    #[test]
    fn parse_backup_filename_diff() {
        let result = parse_backup_filename("20260414T153001Z-diff.bak");
        assert_eq!(
            result,
            Some(("20260414T153001Z".to_string(), BackupType::Diff))
        );
    }

    #[test]
    fn parse_backup_filename_invalid() {
        assert!(parse_backup_filename("notafile.txt").is_none());
        assert!(parse_backup_filename("20260414T153000Z-unknown.bak").is_none());
        assert!(parse_backup_filename("").is_none());
        assert!(parse_backup_filename("nodash.bak").is_none());
    }

    #[test]
    fn load_and_save_index_file_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.json");

        let mut index = BackupIndex::new();
        index.entries.push(make_base_entry("20260414T153000Z"));
        index.entries.push(make_diff_entry("20260414T153001Z", "20260414T153000Z"));

        save_index(&path, &index).unwrap();

        let loaded = load_index(&path).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].id, "20260414T153000Z");
        assert_eq!(loaded.entries[1].id, "20260414T153001Z");
    }

    #[test]
    fn load_index_returns_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");

        let index = load_index(&path).unwrap();
        assert_eq!(index.version, 1);
        assert!(index.entries.is_empty());
    }

    #[test]
    fn diffs_since_last_base_counts_correctly() {
        let mut index = BackupIndex::new();
        assert_eq!(index.diffs_since_last_base(), 0);

        index.entries.push(make_base_entry("20260414T153000Z"));
        assert_eq!(index.diffs_since_last_base(), 0);

        index.entries.push(make_diff_entry("20260414T153001Z", "20260414T153000Z"));
        index.entries.push(make_diff_entry("20260414T153002Z", "20260414T153000Z"));
        assert_eq!(index.diffs_since_last_base(), 2);

        index.entries.push(make_base_entry("20260414T153003Z"));
        assert_eq!(index.diffs_since_last_base(), 0);

        index.entries.push(make_diff_entry("20260414T153004Z", "20260414T153003Z"));
        assert_eq!(index.diffs_since_last_base(), 1);
    }

    #[test]
    fn latest_base_returns_most_recent() {
        let mut index = BackupIndex::new();
        assert!(index.latest_base().is_none());

        index.entries.push(make_base_entry("20260414T150000Z"));
        index.entries.push(make_diff_entry("20260414T150001Z", "20260414T150000Z"));
        index.entries.push(make_base_entry("20260414T160000Z"));
        index.entries.push(make_diff_entry("20260414T160001Z", "20260414T160000Z"));

        let base = index.latest_base().unwrap();
        assert_eq!(base.id, "20260414T160000Z");
    }

    #[test]
    fn chain_to_returns_base_and_diffs() {
        let mut index = BackupIndex::new();
        let base_id = "20260414T153000Z";
        let diff1_id = "20260414T153001Z";
        let diff2_id = "20260414T153002Z";

        index.entries.push(make_base_entry(base_id));
        index.entries.push(make_diff_entry(diff1_id, base_id));
        index.entries.push(make_diff_entry(diff2_id, diff1_id));

        let chain = index.chain_to(diff2_id).unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].id, base_id);
        assert_eq!(chain[1].id, diff1_id);
        assert_eq!(chain[2].id, diff2_id);
    }

    #[test]
    fn chain_to_base_only() {
        let mut index = BackupIndex::new();
        index.entries.push(make_base_entry("20260414T153000Z"));

        let chain = index.chain_to("20260414T153000Z").unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].entry_type, BackupType::Base);
    }

    #[test]
    fn chain_to_missing_id_returns_none() {
        let index = BackupIndex::new();
        assert!(index.chain_to("doesnotexist").is_none());
    }
}
