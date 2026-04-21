//! Backup index types and persistence for tracking backup chains.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Whether a backup entry is a full base snapshot or an incremental diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupType {
    Base,
    Diff,
}

impl fmt::Display for BackupType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupType::Base => f.write_str("base"),
            BackupType::Diff => f.write_str("diff"),
        }
    }
}

/// Identifies which key-encryption key wrapped the data-encryption key for a backup chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintField {
    Known(String),
    Unknown,
}

impl serde::Serialize for FingerprintField {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            FingerprintField::Known(hex) => s.serialize_str(&format!("known:{hex}")),
            FingerprintField::Unknown => s.serialize_str("unknown"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for FingerprintField {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        if raw == "unknown" {
            return Ok(FingerprintField::Unknown);
        }
        if let Some(hex) = raw.strip_prefix("known:") {
            if hex.is_empty() {
                return Err(serde::de::Error::custom("empty fingerprint hex"));
            }
            if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(serde::de::Error::custom(format!(
                    "fingerprint contains non-hex characters: {hex}"
                )));
            }
            return Ok(FingerprintField::Known(hex.to_string()));
        }
        Err(serde::de::Error::custom(format!(
            "invalid fingerprint field: {raw}"
        )))
    }
}

/// Metadata for a single backup file: type, hashes, sizes, and timestamps.
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

/// The persistent backup index: a versioned list of all backup entries in chronological order.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupIndex {
    pub version: u32,
    pub entries: Vec<BackupEntry>,
}

impl Default for BackupIndex {
    fn default() -> Self {
        Self::new()
    }
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
    /// Walks backward from the target entry until a base entry is found. Returns an error if
    /// the target id does not exist, if the chain references a base_id that cannot be resolved,
    /// if a cycle is detected in the base_id links, or if the chain exceeds the depth cap.
    pub fn chain_to(&self, target_id: &str) -> Result<Vec<&BackupEntry>> {
        const MAX_CHAIN_DEPTH: usize = 10_000;

        let target = self
            .find_entry(target_id)
            .with_context(|| format!("backup id not found in index: {target_id}"))?;

        let mut chain: Vec<&BackupEntry> = Vec::new();
        let mut visited: HashSet<&str> = HashSet::new();
        let mut current = target;

        loop {
            if !visited.insert(current.id.as_str()) {
                anyhow::bail!("cycle detected in backup chain at id: {}", current.id);
            }
            if chain.len() >= MAX_CHAIN_DEPTH {
                anyhow::bail!(
                    "backup chain exceeds maximum depth of {MAX_CHAIN_DEPTH} at id: {}",
                    current.id
                );
            }
            chain.push(current);
            match current.entry_type {
                BackupType::Base => break,
                BackupType::Diff => {
                    let base_id = current
                        .base_id
                        .as_deref()
                        .with_context(|| format!("diff entry {} has no base_id", current.id))?;
                    current = self.find_entry(base_id).with_context(|| {
                        format!(
                            "base_id {} referenced by {} not found in index",
                            base_id, current.id
                        )
                    })?;
                }
            }
        }

        chain.reverse();
        Ok(chain)
    }
}

/// Generates a backup identifier from the current UTC time.
///
/// Format: "YYYYMMDDTHHmmss.mmmZ" (20 characters, e.g. "20260414T153000.123Z").
/// Millisecond resolution prevents collisions between snapshots taken within the same second.
pub fn generate_backup_id() -> String {
    Utc::now().format("%Y%m%dT%H%M%S.%3fZ").to_string()
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

/// Returns true when `name` is safe to use as a backup filename inside the backups directory.
///
/// Rejects empty strings, path separators, parent-directory references, and absolute paths so
/// that `Path::join(&name)` cannot escape the backups directory regardless of the value.
pub fn is_safe_backup_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
        && !std::path::Path::new(name).is_absolute()
}

/// Loads a `BackupIndex` from the given path.
///
/// Returns an empty index with version=1 when the file does not exist.
/// Returns an error if any entry contains an unsafe filename (absolute path, `..`, separators).
pub fn load_index(path: &Path) -> Result<BackupIndex> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BackupIndex::new()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read index file: {}", path.display()));
        }
    };

    let index: BackupIndex = serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse index file: {}", path.display()))?;

    for entry in &index.entries {
        if !is_safe_backup_filename(&entry.filename) {
            anyhow::bail!("backup index contains unsafe filename: {}", entry.filename);
        }
    }

    Ok(index)
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
        assert_eq!(restored.base_id.as_deref(), Some("20260414T153000Z"));
    }

    #[test]
    fn generate_id_format() {
        let id = generate_backup_id();
        assert_eq!(id.len(), 20, "id must be 20 characters: {id}");
        assert!(id.ends_with('Z'), "id must end with Z: {id}");
        assert!(id.contains('T'), "id must contain T: {id}");
        assert!(
            id.contains('.'),
            "id must contain millisecond separator: {id}"
        );
    }

    #[test]
    fn generate_id_millisecond_resolution_differs_across_ms_boundary() {
        let id1 = generate_backup_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = generate_backup_id();
        assert_ne!(
            id1, id2,
            "ids separated by 2ms must differ: id1={id1} id2={id2}"
        );
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
        index
            .entries
            .push(make_diff_entry("20260414T153001Z", "20260414T153000Z"));

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
    fn is_safe_backup_filename_accepts_valid_name() {
        assert!(is_safe_backup_filename("20260414T073656Z-base.bak"));
        assert!(is_safe_backup_filename("20260414T073656Z-diff.bak"));
        assert!(is_safe_backup_filename("20260414T073656.123Z-base.bak"));
        assert!(is_safe_backup_filename("20260414T073656.123Z-diff.bak"));
    }

    #[test]
    fn is_safe_backup_filename_rejects_unsafe_names() {
        assert!(!is_safe_backup_filename(""));
        assert!(!is_safe_backup_filename("."));
        assert!(!is_safe_backup_filename(".."));
        assert!(!is_safe_backup_filename("../evil"));
        assert!(!is_safe_backup_filename("/etc/passwd"));
        assert!(!is_safe_backup_filename("sub/dir"));
        assert!(!is_safe_backup_filename("back\\slash"));
    }

    #[test]
    fn load_index_rejects_unsafe_filename_in_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.json");

        let mut index = BackupIndex::new();
        let mut entry = make_base_entry("20260414T153000Z");
        entry.filename = "../evil.bak".to_string();
        index.entries.push(entry);

        save_index(&path, &index).unwrap();

        let result = load_index(&path);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unsafe filename"), "unexpected error: {msg}");
    }

    #[test]
    fn diffs_since_last_base_counts_correctly() {
        let mut index = BackupIndex::new();
        assert_eq!(index.diffs_since_last_base(), 0);

        index.entries.push(make_base_entry("20260414T153000Z"));
        assert_eq!(index.diffs_since_last_base(), 0);

        index
            .entries
            .push(make_diff_entry("20260414T153001Z", "20260414T153000Z"));
        index
            .entries
            .push(make_diff_entry("20260414T153002Z", "20260414T153000Z"));
        assert_eq!(index.diffs_since_last_base(), 2);

        index.entries.push(make_base_entry("20260414T153003Z"));
        assert_eq!(index.diffs_since_last_base(), 0);

        index
            .entries
            .push(make_diff_entry("20260414T153004Z", "20260414T153003Z"));
        assert_eq!(index.diffs_since_last_base(), 1);
    }

    #[test]
    fn latest_base_returns_most_recent() {
        let mut index = BackupIndex::new();
        assert!(index.latest_base().is_none());

        index.entries.push(make_base_entry("20260414T150000Z"));
        index
            .entries
            .push(make_diff_entry("20260414T150001Z", "20260414T150000Z"));
        index.entries.push(make_base_entry("20260414T160000Z"));
        index
            .entries
            .push(make_diff_entry("20260414T160001Z", "20260414T160000Z"));

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
    fn chain_to_missing_id_returns_err() {
        let index = BackupIndex::new();
        assert!(index.chain_to("doesnotexist").is_err());
    }

    #[test]
    fn chain_to_cycle_returns_err() {
        let mut index = BackupIndex::new();

        let mut a = make_diff_entry("id-a", "id-b");
        a.base_id = Some("id-b".to_string());
        let mut b = make_diff_entry("id-b", "id-a");
        b.base_id = Some("id-a".to_string());

        index.entries.push(a);
        index.entries.push(b);

        let result = index.chain_to("id-a");
        assert!(result.is_err(), "expected Err for cyclic chain");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cycle"), "expected cycle error, got: {msg}");
    }
}

#[cfg(test)]
mod fingerprint_field_tests {
    use super::FingerprintField;

    #[test]
    fn serializes_known_as_prefixed_string() {
        let f = FingerprintField::Known("abc12345".into());
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"known:abc12345\"");
    }

    #[test]
    fn serializes_unknown_as_literal() {
        let f = FingerprintField::Unknown;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"unknown\"");
    }

    #[test]
    fn deserializes_known() {
        let f: FingerprintField = serde_json::from_str("\"known:deadbeef\"").unwrap();
        assert!(matches!(f, FingerprintField::Known(ref h) if h == "deadbeef"));
    }

    #[test]
    fn deserializes_unknown() {
        let f: FingerprintField = serde_json::from_str("\"unknown\"").unwrap();
        assert!(matches!(f, FingerprintField::Unknown));
    }

    #[test]
    fn rejects_malformed() {
        assert!(serde_json::from_str::<FingerprintField>("\"garbage\"").is_err());
        assert!(serde_json::from_str::<FingerprintField>("\"known:\"").is_err());
    }

    #[test]
    fn rejects_non_hex_characters() {
        assert!(serde_json::from_str::<FingerprintField>("\"known:xyz!\"").is_err());
    }
}
