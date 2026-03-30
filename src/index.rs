use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStorageMode {
    Plaintext,
    Encrypted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub modified_unix_ms: u128,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndexEntry {
    pub stamp: FileStamp,
    pub display_name: String,
    pub message_count: usize,
    #[serde(default)]
    pub first_user_preview: Option<String>,
    pub storage_mode: SessionStorageMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterIndexEntry {
    pub stamp: FileStamp,
    pub slug: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldbookIndexEntry {
    pub stamp: FileStamp,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataIndex {
    #[serde(default = "index_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sessions: HashMap<String, SessionIndexEntry>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub characters: HashMap<String, CharacterIndexEntry>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub worldbooks: HashMap<String, WorldbookIndexEntry>,
}

fn index_version() -> u32 {
    1
}

impl Default for MetadataIndex {
    fn default() -> Self {
        Self {
            version: index_version(),
            sessions: HashMap::new(),
            characters: HashMap::new(),
            worldbooks: HashMap::new(),
        }
    }
}

pub fn load_index(key: Option<&DerivedKey>) -> MetadataIndex {
    let path = crate::config::index_path();
    let read_start = Instant::now();
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            crate::debug_log::log_kv(
                "index.load",
                &[
                    crate::debug_log::field("phase", "read"),
                    crate::debug_log::field("result", "missing"),
                    crate::debug_log::field("path", path.display()),
                    crate::debug_log::field(
                        "elapsed_ms",
                        format!("{:.3}", read_start.elapsed().as_secs_f64() * 1000.0),
                    ),
                ],
            );
            return MetadataIndex::default();
        }
        Err(err) => {
            crate::debug_log::log_kv(
                "index.load",
                &[
                    crate::debug_log::field("phase", "read"),
                    crate::debug_log::field("result", "error"),
                    crate::debug_log::field("path", path.display()),
                    crate::debug_log::field(
                        "elapsed_ms",
                        format!("{:.3}", read_start.elapsed().as_secs_f64() * 1000.0),
                    ),
                    crate::debug_log::field("error", &err),
                ],
            );
            eprintln!("Warning: failed to read {}: {err}", path.display());
            return MetadataIndex::default();
        }
    };
    crate::debug_log::log_kv(
        "index.load",
        &[
            crate::debug_log::field("phase", "read"),
            crate::debug_log::field("result", "ok"),
            crate::debug_log::field("path", path.display()),
            crate::debug_log::field("bytes", raw.len()),
            crate::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", read_start.elapsed().as_secs_f64() * 1000.0),
            ),
        ],
    );

    let contents = if crate::crypto::is_encrypted(&raw) {
        let Some(key) = key else {
            crate::debug_log::log_kv(
                "index.load",
                &[
                    crate::debug_log::field("phase", "decrypt"),
                    crate::debug_log::field("result", "no_key"),
                    crate::debug_log::field("path", path.display()),
                ],
            );
            return MetadataIndex::default();
        };
        match crate::crypto::decrypt(&raw, key) {
            Ok(plaintext) => match String::from_utf8(plaintext) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("Warning: decrypted index is not valid UTF-8: {err}");
                    return MetadataIndex::default();
                }
            },
            Err(err) => {
                eprintln!("Warning: failed to decrypt {}: {err}", path.display());
                return MetadataIndex::default();
            }
        }
    } else {
        match String::from_utf8(raw) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Warning: index is not valid UTF-8: {err}");
                return MetadataIndex::default();
            }
        }
    };

    let parse_start = Instant::now();
    match serde_json::from_str::<MetadataIndex>(&contents) {
        Ok(index) => {
            crate::debug_log::log_kv(
                "index.load",
                &[
                    crate::debug_log::field("phase", "parse"),
                    crate::debug_log::field("result", "ok"),
                    crate::debug_log::field("sessions", index.sessions.len()),
                    crate::debug_log::field("characters", index.characters.len()),
                    crate::debug_log::field("worldbooks", index.worldbooks.len()),
                    crate::debug_log::field(
                        "elapsed_ms",
                        format!("{:.3}", parse_start.elapsed().as_secs_f64() * 1000.0),
                    ),
                ],
            );
            index
        }
        Err(err) => {
            crate::debug_log::log_kv(
                "index.load",
                &[
                    crate::debug_log::field("phase", "parse"),
                    crate::debug_log::field("result", "error"),
                    crate::debug_log::field(
                        "elapsed_ms",
                        format!("{:.3}", parse_start.elapsed().as_secs_f64() * 1000.0),
                    ),
                    crate::debug_log::field("error", &err),
                ],
            );
            eprintln!("Warning: failed to parse {}: {err}", path.display());
            MetadataIndex::default()
        }
    }
}

pub fn save_index(index: &MetadataIndex, key: Option<&DerivedKey>) -> Result<()> {
    let path = crate::config::index_path();
    let serialize_start = Instant::now();
    let json = serde_json::to_string_pretty(index).context("failed to serialize metadata index")?;
    crate::debug_log::log_kv(
        "index.save",
        &[
            crate::debug_log::field("phase", "serialize"),
            crate::debug_log::field("result", "ok"),
            crate::debug_log::field("bytes", json.len()),
            crate::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", serialize_start.elapsed().as_secs_f64() * 1000.0),
            ),
        ],
    );
    crate::debug_log::timed_result(
        "index.save",
        &[
            crate::debug_log::field("phase", "write"),
            crate::debug_log::field("path", path.display()),
            crate::debug_log::field("bytes", json.len()),
        ],
        || {
            crate::crypto::encrypt_and_write(&path, json.as_bytes(), key).context(format!(
                "failed to write metadata index: {}",
                path.display()
            ))
        },
    )
}

pub fn warn_if_save_fails(result: Result<()>, action: &str) {
    if let Err(err) = result {
        crate::debug_log::log_kv(
            "index.save",
            &[
                crate::debug_log::field("phase", "warn"),
                crate::debug_log::field("result", "error"),
                crate::debug_log::field("action", action),
                crate::debug_log::field("error", &err),
            ],
        );
        eprintln!("Warning: {action}: {err}");
    }
}

pub fn file_stamp(path: &Path) -> Result<FileStamp> {
    let metadata = std::fs::metadata(path)
        .context(format!("failed to read metadata for {}", path.display()))?;
    let modified = metadata.modified().context(format!(
        "failed to read modified time for {}",
        path.display()
    ))?;
    let modified_unix_ms = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(FileStamp {
        modified_unix_ms,
        size: metadata.len(),
    })
}

pub fn relative_data_path(path: &Path) -> Option<String> {
    path.strip_prefix(crate::config::data_dir())
        .ok()
        .map(|relative| relative.to_string_lossy().into_owned())
}

pub fn upsert_session(
    path: &Path,
    stamp: FileStamp,
    display_name: String,
    message_count: usize,
    first_user_preview: Option<String>,
    storage_mode: SessionStorageMode,
    key: Option<&DerivedKey>,
) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.sessions.insert(
        relative_path,
        SessionIndexEntry {
            stamp,
            display_name,
            message_count,
            first_user_preview,
            storage_mode,
        },
    );
    save_index(&index, key)
}

pub fn upsert_character(
    path: &Path,
    stamp: FileStamp,
    slug: String,
    display_name: String,
    key: Option<&DerivedKey>,
) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.characters.insert(
        relative_path,
        CharacterIndexEntry {
            stamp,
            slug,
            display_name,
        },
    );
    save_index(&index, key)
}

pub fn upsert_worldbook(
    path: &Path,
    stamp: FileStamp,
    display_name: String,
    key: Option<&DerivedKey>,
) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.worldbooks.insert(
        relative_path,
        WorldbookIndexEntry {
            stamp,
            display_name,
        },
    );
    save_index(&index, key)
}

pub fn remove_session(path: &Path, key: Option<&DerivedKey>) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.sessions.remove(&relative_path);
    save_index(&index, key)
}

pub fn remove_character(path: &Path, key: Option<&DerivedKey>) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.characters.remove(&relative_path);
    save_index(&index, key)
}

pub fn remove_worldbook(path: &Path, key: Option<&DerivedKey>) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index(key);
    index.worldbooks.remove(&relative_path);
    save_index(&index, key)
}
