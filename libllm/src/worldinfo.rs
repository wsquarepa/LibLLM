//! World info / lorebook types with keyword-triggered entry activation.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::debug_log;

/// A named collection of lorebook entries with keyword-activated content injection.
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

/// Parses a worldbook from JSON, accepting both the normalized format and the SillyTavern legacy format.
///
/// Disabled and empty-content entries are filtered out. Uses `fallback_name` when the
/// JSON does not include a `name` field.
pub fn parse_worldbook_json(contents: &str, fallback_name: &str) -> Result<WorldBook> {
    debug_log::timed_result(
        "worldinfo.parse",
        &[debug_log::field("bytes", contents.len())],
        || {
            if let Ok(normalized) = serde_json::from_str::<WorldBook>(contents) {
                debug_log::log_kv(
                    "worldinfo.parse",
                    &[
                        debug_log::field("phase", "normalized"),
                        debug_log::field("entry_count", normalized.entries.len()),
                        debug_log::field("fallback_name_used", false),
                    ],
                );
                return Ok(normalized);
            }

            let raw: RawWorldBook =
                serde_json::from_str(contents).context("failed to parse worldbook JSON")?;

            let fallback_name_used = raw.name.is_none();
            let name = raw
                .name
                .unwrap_or_else(|| fallback_name.to_owned());

            let scan_depth = raw.scan_depth.unwrap_or(DEFAULT_SCAN_DEPTH);
            let raw_entry_count = raw.entries.len();

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
            debug_log::log_kv(
                "worldinfo.parse",
                &[
                    debug_log::field("phase", "legacy"),
                    debug_log::field("entry_count", entries.len()),
                    debug_log::field("filtered_count", raw_entry_count - entries.len()),
                    debug_log::field("fallback_name_used", fallback_name_used),
                    debug_log::field("scan_depth", scan_depth),
                ],
            );
            Ok(WorldBook { name, entries })
        },
    )
}

/// A worldbook entry whose keywords matched the recent message window.
pub struct ActivatedEntry {
    pub content: String,
    pub depth: usize,
    pub order: i64,
}

/// Pre-processed worldbook with case-normalized keys for efficient repeated scanning.
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
        let entries: Vec<RuntimeEntry> = worldbook
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

        let case_sensitive_count = entries.iter().filter(|e| e.case_sensitive).count();
        let constant_count = entries.iter().filter(|e| e.constant).count();
        debug_log::log_kv(
            "worldinfo.runtime",
            &[
                debug_log::field("phase", "build"),
                debug_log::field("entry_count", entries.len()),
                debug_log::field("case_sensitive_count", case_sensitive_count),
                debug_log::field("constant_count", constant_count),
            ],
        );

        Self { entries }
    }
}

/// Scans recent messages against a runtime worldbook and returns all activated entries.
///
/// Constant entries are always included. Non-constant entries activate when at least one
/// primary key matches within the entry's depth window. Selective entries additionally
/// require all secondary keys to match.
pub fn scan_runtime_entries(
    worldbook: &RuntimeWorldBook,
    messages: &[&str],
) -> Vec<ActivatedEntry> {
    debug_log::timed_kv(
        "worldinfo.scan",
        &[
            debug_log::field("message_count", messages.len()),
            debug_log::field("entry_count", worldbook.entries.len()),
        ],
        || {
            let mut activated: Vec<ActivatedEntry> = Vec::new();
            let mut constant_activated = 0usize;
            let mut keyword_activated = 0usize;
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
                    constant_activated += 1;
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
                keyword_activated += 1;
            }

            activated.sort_by_key(|e| e.order);
            debug_log::log_kv(
                "worldinfo.scan",
                &[
                    debug_log::field("phase", "done"),
                    debug_log::field("activated", activated.len()),
                    debug_log::field("constant_activated", constant_activated),
                    debug_log::field("keyword_activated", keyword_activated),
                ],
            );
            activated
        },
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(keys: Vec<&str>, content: &str) -> Entry {
        Entry {
            keys: keys.into_iter().map(String::from).collect(),
            secondary_keys: Vec::new(),
            selective: false,
            content: content.to_string(),
            constant: false,
            enabled: true,
            order: 0,
            depth: 4,
            case_sensitive: false,
        }
    }

    fn make_constant_entry(content: &str) -> Entry {
        Entry {
            keys: Vec::new(),
            secondary_keys: Vec::new(),
            selective: false,
            content: content.to_string(),
            constant: true,
            enabled: true,
            order: 0,
            depth: 4,
            case_sensitive: false,
        }
    }

    fn make_worldbook(name: &str, entries: Vec<Entry>) -> WorldBook {
        WorldBook {
            name: name.to_string(),
            entries,
        }
    }

    #[test]
    fn keyword_scanning() {
        let entries = vec![
            make_entry(vec!["dragon"], "Fire creature"),
            make_entry(vec!["elf"], "Pointy ears"),
        ];
        let wb = make_worldbook("Lore", entries);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec!["I saw a dragon in the sky"];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].content, "Fire creature");
    }

    #[test]
    fn selective_entries() {
        let selective_entry = Entry {
            keys: vec!["king".to_string()],
            secondary_keys: vec!["castle".to_string()],
            selective: true,
            content: "The king lives in the castle".to_string(),
            constant: false,
            enabled: true,
            order: 0,
            depth: 4,
            case_sensitive: false,
        };
        let wb = make_worldbook("Selective", vec![selective_entry]);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let only_primary = vec!["The king walked through town"];
        let activated = scan_runtime_entries(&runtime, &only_primary);
        assert!(
            activated.is_empty(),
            "should not activate with only primary key"
        );

        let both_keys = vec!["The king returned to the castle"];
        let activated = scan_runtime_entries(&runtime, &both_keys);
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].content, "The king lives in the castle");
    }

    #[test]
    fn constant_entries() {
        let entries = vec![
            make_constant_entry("Always present lore"),
            make_entry(vec!["specific"], "Only when matched"),
        ];
        let wb = make_worldbook("Mixed", entries);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec!["No keywords here"];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].content, "Always present lore");
    }

    #[test]
    fn disabled_entries() {
        let disabled = Entry {
            keys: vec!["trigger".to_string()],
            secondary_keys: Vec::new(),
            selective: false,
            content: "Should not appear".to_string(),
            constant: false,
            enabled: false,
            order: 0,
            depth: 4,
            case_sensitive: false,
        };
        let wb = make_worldbook("Disabled", vec![disabled]);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec!["This message contains the trigger word"];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(activated.len(), 1);
    }

    #[test]
    fn case_sensitivity() {
        let case_sensitive = Entry {
            keys: vec!["Dragon".to_string()],
            secondary_keys: Vec::new(),
            selective: false,
            content: "Case sensitive entry".to_string(),
            constant: false,
            enabled: true,
            order: 0,
            depth: 4,
            case_sensitive: true,
        };
        let case_insensitive = Entry {
            keys: vec!["Elf".to_string()],
            secondary_keys: Vec::new(),
            selective: false,
            content: "Case insensitive entry".to_string(),
            constant: false,
            enabled: true,
            order: 0,
            depth: 4,
            case_sensitive: false,
        };
        let wb = make_worldbook("Case", vec![case_sensitive, case_insensitive]);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec!["I saw a dragon and an elf"];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(
            activated.len(),
            1,
            "only case-insensitive should match lowercase"
        );
        assert_eq!(activated[0].content, "Case insensitive entry");
    }

    #[test]
    fn entry_ordering() {
        let entries = vec![
            {
                let mut e = make_entry(vec!["a"], "Order 30");
                e.order = 30;
                e
            },
            {
                let mut e = make_entry(vec!["a"], "Order 10");
                e.order = 10;
                e
            },
            {
                let mut e = make_entry(vec!["a"], "Order 20");
                e.order = 20;
                e
            },
        ];
        let wb = make_worldbook("Ordered", entries);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec!["Message about a"];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(activated.len(), 3);
        assert_eq!(activated[0].content, "Order 10");
        assert_eq!(activated[1].content, "Order 20");
        assert_eq!(activated[2].content, "Order 30");
    }

    #[test]
    fn depth_filtering() {
        let mut shallow = make_entry(vec!["keyword"], "Shallow (depth 1)");
        shallow.depth = 1;

        let mut deep = make_entry(vec!["keyword"], "Deep (depth 4)");
        deep.depth = 4;

        let wb = make_worldbook("Depth", vec![shallow, deep]);
        let runtime = RuntimeWorldBook::from_worldbook(&wb);

        let messages = vec![
            "Old message without keyword",
            "Another old message",
            "Still old",
            "Recent message with keyword",
        ];
        let activated = scan_runtime_entries(&runtime, &messages);

        assert_eq!(
            activated.len(),
            2,
            "both entries should match since keyword is in recent message"
        );

        let far_messages = vec![
            "Message with keyword here",
            "No keyword",
            "No keyword",
            "No keyword",
        ];
        let activated = scan_runtime_entries(&runtime, &far_messages);

        assert!(
            activated.iter().any(|a| a.content == "Deep (depth 4)"),
            "deep entry should match"
        );
    }

    #[test]
    fn parse_legacy_format() {
        let legacy_json = serde_json::json!({
            "entries": {
                "0": {
                    "key": ["dragon", "wyrm"],
                    "keysecondary": ["fire"],
                    "content": "Dragons breathe fire.",
                    "disable": false,
                    "order": 5,
                    "depth": 4
                }
            }
        });

        let wb = parse_worldbook_json(&legacy_json.to_string(), "test-wb").unwrap();
        assert_eq!(wb.name, "test-wb");
        assert_eq!(wb.entries.len(), 1);
        let entry = &wb.entries[0];
        assert_eq!(entry.keys, vec!["dragon", "wyrm"]);
        assert_eq!(entry.secondary_keys, vec!["fire"]);
        assert_eq!(entry.content, "Dragons breathe fire.");
        assert!(entry.enabled);
    }

    #[test]
    fn parse_direct_worldbook_format() {
        let direct_json = serde_json::json!({
            "name": "Direct Format WB",
            "entries": [
                {
                    "keys": ["sword", "blade"],
                    "secondary_keys": [],
                    "selective": false,
                    "content": "A sharp weapon.",
                    "constant": false,
                    "enabled": true,
                    "order": 5,
                    "depth": 4,
                    "case_sensitive": false
                }
            ]
        });

        let wb = parse_worldbook_json(&direct_json.to_string(), "fallback").unwrap();
        assert_eq!(wb.name, "Direct Format WB");
        assert_eq!(wb.entries.len(), 1);
        let entry = &wb.entries[0];
        assert_eq!(entry.keys, vec!["sword", "blade"]);
        assert_eq!(entry.content, "A sharp weapon.");
        assert!(!entry.constant);
        assert!(entry.enabled);
    }

    #[test]
    fn build_window_fewer_messages_than_depth() {
        let messages = vec!["hello", "world"];
        let window = build_window(&messages, 5);
        assert_eq!(window, "hello\nworld");
    }

    #[test]
    fn build_window_exact_depth() {
        let messages = vec!["one", "two", "three"];
        let window = build_window(&messages, 3);
        assert_eq!(window, "one\ntwo\nthree");
    }

    #[test]
    fn build_window_more_messages_than_depth() {
        let messages = vec!["first", "second", "third", "fourth"];
        let window = build_window(&messages, 2);
        assert_eq!(window, "third\nfourth");
    }
}

