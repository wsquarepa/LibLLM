use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBook {
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

pub fn parse_worldbook_json(contents: &str, fallback_name: &str) -> Result<WorldBook> {
    if let Ok(normalized) = serde_json::from_str::<WorldBook>(contents) {
        return Ok(normalized);
    }

    let raw: RawWorldBook =
        serde_json::from_str(contents).context("failed to parse worldbook JSON")?;

    let name = raw
        .name
        .unwrap_or_else(|| fallback_name.to_owned());

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

