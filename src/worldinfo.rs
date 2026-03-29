use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

const EXT_ENCRYPTED: &str = "worldbook";
const EXT_PLAINTEXT: &str = "json";

pub fn resolve_worldbook_path(dir: &Path, name: &str) -> std::path::PathBuf {
    crate::crypto::resolve_encrypted_path(dir, name, EXT_ENCRYPTED)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBook {
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub keys: Vec<String>,
    pub secondary_keys: Vec<String>,
    pub selective: bool,
    pub content: String,
    pub constant: bool,
    pub enabled: bool,
    pub order: i64,
    pub depth: usize,
    pub case_sensitive: bool,
}

pub struct WorldBookEntry {
    pub name: String,
}

#[derive(Deserialize)]
struct RawWorldBook {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    scan_depth: Option<usize>,
    entries: HashMap<String, RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    key: Option<Vec<String>>,
    #[serde(default)]
    keys: Option<Vec<String>>,
    #[serde(default)]
    keysecondary: Option<Vec<String>>,
    #[serde(default)]
    secondary_keys: Option<Vec<String>>,
    #[serde(default)]
    selective: Option<bool>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    constant: Option<bool>,
    #[serde(default)]
    disable: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default, alias = "caseSensitive")]
    case_sensitive: Option<bool>,
}

const DEFAULT_SCAN_DEPTH: usize = 4;

pub fn save_worldbook_to(
    worldbook: &WorldBook,
    path: &Path,
    key: Option<&DerivedKey>,
) -> Result<()> {
    let json = serde_json::to_string_pretty(worldbook).context("failed to serialize worldbook")?;
    crate::crypto::encrypt_and_write(path, json.as_bytes(), key)
}

pub fn save_worldbook(
    worldbook: &WorldBook,
    dir: &Path,
    key: Option<&DerivedKey>,
) -> Result<std::path::PathBuf> {
    let ext = crate::crypto::encrypted_extension(key, EXT_ENCRYPTED);
    let path = dir.join(format!("{}.{ext}", worldbook.name));
    save_worldbook_to(worldbook, &path, key)?;
    Ok(path)
}

pub fn load_worldbook(path: &Path, key: Option<&DerivedKey>) -> Result<WorldBook> {
    let contents = crate::crypto::read_and_decrypt(path, key)?;

    if let Ok(normalized) = serde_json::from_str::<WorldBook>(&contents) {
        return Ok(normalized);
    }

    let raw: RawWorldBook =
        serde_json::from_str(&contents).context("failed to parse worldbook JSON")?;

    let name = raw
        .name
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_default();

    let scan_depth = raw.scan_depth.unwrap_or(DEFAULT_SCAN_DEPTH);

    let mut entries: Vec<Entry> = raw
        .entries
        .into_values()
        .map(|raw_entry| {
            let keys = raw_entry.keys.or(raw_entry.key).unwrap_or_default();
            let secondary_keys = raw_entry
                .secondary_keys
                .or(raw_entry.keysecondary)
                .unwrap_or_default();
            let enabled = raw_entry
                .enabled
                .unwrap_or_else(|| !raw_entry.disable.unwrap_or(false));

            Entry {
                keys,
                secondary_keys,
                selective: raw_entry.selective.unwrap_or(false),
                content: raw_entry.content.unwrap_or_default(),
                constant: raw_entry.constant.unwrap_or(false),
                enabled,
                order: raw_entry.order.unwrap_or(10),
                depth: raw_entry.depth.unwrap_or(scan_depth),
                case_sensitive: raw_entry.case_sensitive.unwrap_or(false),
            }
        })
        .filter(|e| e.enabled && !e.content.is_empty())
        .collect();

    entries.sort_by_key(|e| e.order);
    Ok(WorldBook { name, entries })
}

pub struct ActivatedEntry {
    pub content: String,
    pub depth: usize,
    pub order: i64,
}

#[derive(Clone)]
pub struct RuntimeWorldBook {
    entries: Vec<RuntimeEntry>,
}

#[derive(Clone)]
struct RuntimeEntry {
    primary_keys: Vec<String>,
    secondary_keys: Vec<String>,
    content: String,
    selective: bool,
    constant: bool,
    order: i64,
    depth: usize,
    case_sensitive: bool,
}

impl RuntimeWorldBook {
    pub fn from_worldbook(worldbook: &WorldBook) -> Self {
        let entries = worldbook
            .entries
            .iter()
            .map(|entry| RuntimeEntry {
                primary_keys: if entry.case_sensitive {
                    entry.keys.clone()
                } else {
                    entry.keys.iter().map(|key| key.to_lowercase()).collect()
                },
                secondary_keys: if entry.case_sensitive {
                    entry.secondary_keys.clone()
                } else {
                    entry
                        .secondary_keys
                        .iter()
                        .map(|key| key.to_lowercase())
                        .collect()
                },
                content: entry.content.clone(),
                selective: entry.selective,
                constant: entry.constant,
                order: entry.order,
                depth: entry.depth,
                case_sensitive: entry.case_sensitive,
            })
            .collect();

        Self { entries }
    }
}

pub fn scan_runtime_entries(
    worldbook: &RuntimeWorldBook,
    messages: &[&str],
) -> Vec<ActivatedEntry> {
    let mut activated: Vec<ActivatedEntry> = Vec::new();
    let mut case_sensitive_windows: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();
    let mut case_insensitive_windows: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();

    for entry in &worldbook.entries {
        if entry.constant {
            activated.push(ActivatedEntry {
                content: entry.content.clone(),
                depth: entry.depth,
                order: entry.order,
            });
            continue;
        }

        let haystack = if entry.case_sensitive {
            case_sensitive_windows
                .entry(entry.depth)
                .or_insert_with(|| build_window(messages, entry.depth))
        } else {
            case_insensitive_windows
                .entry(entry.depth)
                .or_insert_with(|| build_window(messages, entry.depth).to_lowercase())
        };

        let primary_match = entry.primary_keys.iter().any(|k| {
            if k.is_empty() {
                return false;
            }
            haystack.contains(k)
        });

        if !primary_match {
            continue;
        }

        if entry.selective && !entry.secondary_keys.is_empty() {
            let secondary_match = entry.secondary_keys.iter().all(|k| {
                if k.is_empty() {
                    return true;
                }
                haystack.contains(k)
            });
            if !secondary_match {
                continue;
            }
        }

        activated.push(ActivatedEntry {
            content: entry.content.clone(),
            depth: entry.depth,
            order: entry.order,
        });
    }

    activated.sort_by_key(|e| e.order);
    activated
}

fn build_window(messages: &[&str], depth: usize) -> String {
    let scan_messages = if messages.len() > depth {
        &messages[messages.len() - depth..]
    } else {
        messages
    };

    let total_len: usize = scan_messages.iter().map(|msg| msg.len()).sum::<usize>()
        + scan_messages.len().saturating_sub(1);
    let mut combined = String::with_capacity(total_len);
    for (idx, message) in scan_messages.iter().enumerate() {
        if idx > 0 {
            combined.push('\n');
        }
        combined.push_str(message);
    }
    combined
}

pub fn normalize_worldbooks(dir: &Path, key: Option<&DerivedKey>) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read worldinfo dir: {e}"));
            return warnings;
        }
    };

    let file_paths: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == EXT_PLAINTEXT || ext == EXT_ENCRYPTED)
        })
        .collect();

    for path in file_paths {
        if key.is_none() && path.extension().is_some_and(|ext| ext == EXT_ENCRYPTED) {
            continue;
        }
        let display = path.display().to_string();
        let contents = match crate::crypto::read_and_decrypt(&path, key) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };

        let needs_normalize = serde_json::from_str::<WorldBook>(&contents).is_err();
        let is_json_ext = path.extension().is_some_and(|ext| ext == EXT_PLAINTEXT);
        let needs_migrate = is_json_ext && key.is_some();

        if !needs_normalize && !needs_migrate {
            continue;
        }

        let wb = if needs_normalize {
            match load_worldbook(&path, key) {
                Ok(w) => w,
                Err(e) => {
                    warnings.push(format!("skipped {display}: {e}"));
                    continue;
                }
            }
        } else {
            serde_json::from_str::<WorldBook>(&contents).unwrap()
        };

        match save_worldbook(&wb, dir, key) {
            Ok(_) => {
                if needs_migrate {
                    let _ = std::fs::remove_file(&path);
                }
            }
            Err(e) => {
                warnings.push(format!("failed to write {display}: {e}"));
            }
        }
    }

    warnings
}

pub fn list_worldbooks(dir: &Path, key: Option<&DerivedKey>) -> Vec<WorldBookEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut books: Vec<WorldBookEntry> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == EXT_ENCRYPTED || ext == EXT_PLAINTEXT)
        })
        .filter_map(|path| {
            let fallback_name = path.file_stem()?.to_string_lossy().to_string();
            let name = load_worldbook(&path, key)
                .map(|wb| wb.name)
                .unwrap_or(fallback_name);
            Some(WorldBookEntry { name })
        })
        .collect();

    books.sort_by(|a, b| a.name.cmp(&b.name));
    books
}
