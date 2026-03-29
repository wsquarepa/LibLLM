use std::collections::HashMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

pub fn load_index() -> MetadataIndex {
    let path = crate::config::index_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return MetadataIndex::default(),
        Err(err) => {
            crate::debug_log::log("index.load", &format!("read failed: {err}"));
            eprintln!("Warning: failed to read {}: {err}", path.display());
            return MetadataIndex::default();
        }
    };

    match serde_json::from_str::<MetadataIndex>(&contents) {
        Ok(index) => {
            crate::debug_log::log(
                "index.load",
                &format!(
                    "ok sessions={} characters={} worldbooks={}",
                    index.sessions.len(),
                    index.characters.len(),
                    index.worldbooks.len()
                ),
            );
            index
        }
        Err(err) => {
            crate::debug_log::log("index.load", &format!("parse failed: {err}"));
            eprintln!("Warning: failed to parse {}: {err}", path.display());
            MetadataIndex::default()
        }
    }
}

pub fn save_index(index: &MetadataIndex) -> Result<()> {
    let path = crate::config::index_path();
    let json = serde_json::to_string_pretty(index).context("failed to serialize metadata index")?;
    crate::crypto::write_atomic(&path, json.as_bytes()).context(format!(
        "failed to write metadata index: {}",
        path.display()
    ))
}

pub fn warn_if_save_fails(result: Result<()>, action: &str) {
    if let Err(err) = result {
        crate::debug_log::log("index.save", &format!("{action}: {err}"));
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
) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
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
    save_index(&index)
}

pub fn upsert_character(
    path: &Path,
    stamp: FileStamp,
    slug: String,
    display_name: String,
) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
    index.characters.insert(
        relative_path,
        CharacterIndexEntry {
            stamp,
            slug,
            display_name,
        },
    );
    save_index(&index)
}

pub fn upsert_worldbook(path: &Path, stamp: FileStamp, display_name: String) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
    index.worldbooks.insert(
        relative_path,
        WorldbookIndexEntry {
            stamp,
            display_name,
        },
    );
    save_index(&index)
}

pub fn remove_session(path: &Path) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
    index.sessions.remove(&relative_path);
    save_index(&index)
}

pub fn remove_character(path: &Path) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
    index.characters.remove(&relative_path);
    save_index(&index)
}

pub fn remove_worldbook(path: &Path) -> Result<()> {
    let Some(relative_path) = relative_data_path(path) else {
        return Ok(());
    };

    let mut index = load_index();
    index.worldbooks.remove(&relative_path);
    save_index(&index)
}
