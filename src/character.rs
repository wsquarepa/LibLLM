use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;
use crate::index;

const EXT_ENCRYPTED: &str = "character";
const EXT_PLAINTEXT: &str = "json";
const MAX_IMPORT_NAME_LENGTH: usize = 32;

fn sanitize_display(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() && c != '\n' { '\u{FFFD}' } else { c })
        .collect()
}

pub fn resolve_card_path(dir: &Path, slug: &str) -> PathBuf {
    crate::crypto::resolve_encrypted_path(dir, slug, EXT_ENCRYPTED)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterCard {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub mes_example: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub post_history_instructions: String,
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
}

pub struct CharacterEntry {
    pub name: String,
    pub slug: String,
}

pub struct PngImportReport {
    pub imported_count: usize,
    pub warnings: Vec<String>,
}

pub struct PlaintextCardEncryptionReport {
    pub encrypted_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct RawCard {
    name: Option<String>,
    description: Option<String>,
    personality: Option<String>,
    scenario: Option<String>,
    first_mes: Option<String>,
    mes_example: Option<String>,
    data: Option<RawCardData>,
}

#[derive(Deserialize)]
struct RawCardData {
    name: Option<String>,
    description: Option<String>,
    personality: Option<String>,
    scenario: Option<String>,
    first_mes: Option<String>,
    mes_example: Option<String>,
    system_prompt: Option<String>,
    post_history_instructions: Option<String>,
    alternate_greetings: Option<Vec<String>>,
}

pub fn parse_card_json(json_str: &str) -> Result<CharacterCard> {
    let raw: RawCard =
        serde_json::from_str(json_str).context("failed to parse character card JSON")?;

    let data = raw.data.as_ref();
    let pick = |data_field: Option<&str>, top_field: Option<&str>| -> String {
        data_field
            .filter(|s| !s.is_empty())
            .or(top_field.filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_owned()
    };

    let name = pick(data.and_then(|d| d.name.as_deref()), raw.name.as_deref());
    if name.is_empty() {
        bail!("character card has no name");
    }

    Ok(CharacterCard {
        name,
        description: pick(
            data.and_then(|d| d.description.as_deref()),
            raw.description.as_deref(),
        ),
        personality: pick(
            data.and_then(|d| d.personality.as_deref()),
            raw.personality.as_deref(),
        ),
        scenario: pick(
            data.and_then(|d| d.scenario.as_deref()),
            raw.scenario.as_deref(),
        ),
        first_mes: pick(
            data.and_then(|d| d.first_mes.as_deref()),
            raw.first_mes.as_deref(),
        ),
        mes_example: pick(
            data.and_then(|d| d.mes_example.as_deref()),
            raw.mes_example.as_deref(),
        ),
        system_prompt: data
            .and_then(|d| d.system_prompt.as_deref())
            .unwrap_or("")
            .to_owned(),
        post_history_instructions: data
            .and_then(|d| d.post_history_instructions.as_deref())
            .unwrap_or("")
            .to_owned(),
        alternate_greetings: data
            .and_then(|d| d.alternate_greetings.clone())
            .unwrap_or_default(),
    })
}

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

pub fn extract_png_card(png_bytes: &[u8]) -> Result<String> {
    if png_bytes.len() < 8 || png_bytes[..8] != PNG_SIGNATURE {
        bail!("not a valid PNG file");
    }

    let mut offset = 8;
    while offset + 12 <= png_bytes.len() {
        let length = u32::from_be_bytes([
            png_bytes[offset],
            png_bytes[offset + 1],
            png_bytes[offset + 2],
            png_bytes[offset + 3],
        ]) as usize;
        let chunk_type = &png_bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start + length;

        if data_end + 4 > png_bytes.len() {
            break;
        }

        if chunk_type == b"tEXt" {
            let data = &png_bytes[data_start..data_end];
            if let Some(null_pos) = data.iter().position(|&b| b == 0) {
                let keyword = std::str::from_utf8(&data[..null_pos]).unwrap_or("");
                if keyword == "chara" {
                    let b64_bytes = &data[null_pos + 1..];
                    let b64_str = std::str::from_utf8(b64_bytes)
                        .context("chara tEXt value is not valid UTF-8")?;
                    let decoded = STANDARD
                        .decode(b64_str)
                        .context("failed to base64-decode character card from PNG")?;
                    let json = String::from_utf8(decoded)
                        .context("decoded character card is not valid UTF-8")?;
                    return Ok(json);
                }
            }
        }

        offset = data_end + 4;
    }

    bail!("PNG file does not contain a 'chara' tEXt chunk")
}

const MAX_JSON_CARD_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PNG_CARD_BYTES: u64 = 50 * 1024 * 1024;

pub fn import_card(source: &Path) -> Result<CharacterCard> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let file_size = source
        .metadata()
        .context(format!(
            "failed to read file metadata: {}",
            source.display()
        ))?
        .len();

    match ext.as_str() {
        "json" => {
            ensure!(
                file_size <= MAX_JSON_CARD_BYTES,
                "character card JSON exceeds {} MB limit ({} bytes)",
                MAX_JSON_CARD_BYTES / (1024 * 1024),
                file_size
            );
            let contents = std::fs::read_to_string(source).context(format!(
                "failed to read character card: {}",
                source.display()
            ))?;
            parse_card_json(&contents)
        }
        "png" => {
            ensure!(
                file_size <= MAX_PNG_CARD_BYTES,
                "character card PNG exceeds {} MB limit ({} bytes)",
                MAX_PNG_CARD_BYTES / (1024 * 1024),
                file_size
            );
            let bytes = std::fs::read(source)
                .context(format!("failed to read PNG file: {}", source.display()))?;
            let json = extract_png_card(&bytes)?;
            parse_card_json(&json)
        }
        _ => bail!("unsupported character card format: .{ext} (expected .json or .png)"),
    }
}

pub fn build_system_prompt(
    card: &CharacterCard,
    template: Option<&crate::preset::ContextPreset>,
) -> String {
    if let Some(tpl) = template {
        let vars = crate::preset::ContextVars {
            system: card.system_prompt.clone(),
            description: if card.description.is_empty() {
                String::new()
            } else {
                format!("You are {}.\n{}", card.name, card.description)
            },
            personality: card.personality.clone(),
            scenario: card.scenario.clone(),
            persona: String::new(),
            wi_before: String::new(),
            wi_after: String::new(),
            mes_examples: card.mes_example.clone(),
        };
        return tpl.render_story_string(&vars);
    }

    let mut parts: Vec<String> = Vec::new();

    if !card.system_prompt.is_empty() {
        parts.push(card.system_prompt.clone());
    }

    parts.push(format!("You are {}.", card.name));

    if !card.description.is_empty() {
        parts.push(card.description.clone());
    }

    if !card.personality.is_empty() {
        parts.push(format!("Personality: {}", card.personality));
    }

    if !card.scenario.is_empty() {
        parts.push(format!("Scenario: {}", card.scenario));
    }

    if !card.mes_example.is_empty() {
        parts.push(format!("Example dialogue:\n{}", card.mes_example));
    }

    parts.join("\n\n")
}

pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

pub fn save_card(card: &CharacterCard, dir: &Path, key: Option<&DerivedKey>) -> Result<PathBuf> {
    let slug = slugify(&card.name);
    let ext = crate::crypto::encrypted_extension(key, EXT_ENCRYPTED);
    let path = dir.join(format!("{slug}.{ext}"));
    let json = serde_json::to_string_pretty(card).context("failed to serialize character card")?;
    crate::crypto::encrypt_and_write(&path, json.as_bytes(), key)?;
    if let Ok(stamp) = index::file_stamp(&path) {
        index::warn_if_save_fails(
            index::upsert_character(&path, stamp, slug, card.name.clone(), key),
            "failed to update character index",
        );
    }
    Ok(path)
}

pub fn load_card(path: &Path, key: Option<&DerivedKey>) -> Result<CharacterCard> {
    let contents = crate::crypto::read_and_decrypt(path, key)?;
    serde_json::from_str(&contents).context("failed to parse character card")
}

fn remove_source_file(
    path: &Path,
    warnings: &mut Vec<String>,
    action: &str,
    key: Option<&DerivedKey>,
) {
    if let Err(err) = std::fs::remove_file(path) {
        warnings.push(format!(
            "failed to remove {action} {}: {err}",
            path.display()
        ));
        return;
    }
    index::warn_if_save_fails(
        index::remove_character(path, key),
        "failed to remove character index entry",
    );
}

pub fn auto_import_png_cards(dir: &Path, key: Option<&DerivedKey>) -> PngImportReport {
    let mut warnings: Vec<String> = Vec::new();
    let mut imported_count = 0;

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read characters dir: {e}"));
            return PngImportReport {
                imported_count,
                warnings,
            };
        }
    };

    let png_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();

    for png_path in png_paths {
        let display = sanitize_display(&png_path.display().to_string());
        let bytes = match std::fs::read(&png_path) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };
        let json = match extract_png_card(&bytes) {
            Ok(j) => j,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };
        let card = match parse_card_json(&json) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };
        if card.name.chars().count() > MAX_IMPORT_NAME_LENGTH {
            warnings.push(format!(
                "skipped {display}: name exceeds {MAX_IMPORT_NAME_LENGTH} characters",
            ));
            continue;
        }
        match save_card(&card, dir, key) {
            Ok(_) => {
                imported_count += 1;
                remove_source_file(&png_path, &mut warnings, "imported PNG card", key);
            }
            Err(e) => {
                warnings.push(format!("failed to save {display}: {e}"));
            }
        }
    }

    PngImportReport {
        imported_count,
        warnings,
    }
}

pub fn encrypt_plaintext_cards(dir: &Path, key: &DerivedKey) -> PlaintextCardEncryptionReport {
    let mut warnings: Vec<String> = Vec::new();
    let mut encrypted_count = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read characters dir: {e}"));
            return PlaintextCardEncryptionReport {
                encrypted_count,
                warnings,
            };
        }
    };

    let json_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    for path in json_paths {
        let display = sanitize_display(&path.display().to_string());
        let raw = match std::fs::read(&path) {
            Ok(r) => r,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };
        if crate::crypto::is_encrypted(&raw) {
            continue;
        }
        let card = match serde_json::from_slice::<CharacterCard>(&raw) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("skipped {display}: {e}"));
                continue;
            }
        };
        match save_card(&card, dir, Some(key)) {
            Ok(_) => {
                encrypted_count += 1;
                remove_source_file(&path, &mut warnings, "plaintext card", Some(key));
            }
            Err(e) => {
                warnings.push(format!("failed to encrypt {display}: {e}"));
            }
        }
    }

    PlaintextCardEncryptionReport {
        encrypted_count,
        warnings,
    }
}

pub fn list_cards(dir: &Path, key: Option<&DerivedKey>) -> Vec<CharacterEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut cards: Vec<CharacterEntry> = Vec::new();
    let mut index_state = index::load_index(key);
    let mut hit_count = 0usize;
    let mut miss_count = 0usize;
    let mut refreshed_count = 0usize;
    let mut changed = false;

    for path in entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == EXT_ENCRYPTED || ext == EXT_PLAINTEXT)
        })
    {
        let stamp = match index::file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(err) => {
                miss_count += 1;
                crate::debug_log::log_kv(
                    "index.characters",
                    &[
                        crate::debug_log::field("phase", "stamp"),
                        crate::debug_log::field("result", "error"),
                        crate::debug_log::field("path", path.display()),
                        crate::debug_log::field("error", err),
                    ],
                );
                continue;
            }
        };

        let Some(relative_path) = index::relative_data_path(&path) else {
            miss_count += 1;
            continue;
        };

        if let Some(indexed) = index_state.characters.get(&relative_path) {
            if indexed.stamp == stamp {
                hit_count += 1;
                cards.push(CharacterEntry {
                    name: indexed.display_name.clone(),
                    slug: indexed.slug.clone(),
                });
                continue;
            }
        }

        miss_count += 1;
        let Some(slug) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
        else {
            continue;
        };
        match load_card(&path, key) {
            Ok(card) => {
                refreshed_count += 1;
                changed = true;
                index_state.characters.insert(
                    relative_path,
                    index::CharacterIndexEntry {
                        stamp,
                        slug: slug.clone(),
                        display_name: card.name.clone(),
                    },
                );
                cards.push(CharacterEntry {
                    name: card.name,
                    slug,
                });
            }
            Err(err) => {
                crate::debug_log::log_kv(
                    "index.characters",
                    &[
                        crate::debug_log::field("phase", "refresh"),
                        crate::debug_log::field("result", "error"),
                        crate::debug_log::field("path", path.display()),
                        crate::debug_log::field("error", err),
                    ],
                );
                changed |= index_state.characters.remove(&relative_path).is_some();
            }
        }
    }

    crate::debug_log::log_kv(
        "index.characters",
        &[
            crate::debug_log::field("hits", hit_count),
            crate::debug_log::field("misses", miss_count),
            crate::debug_log::field("refreshed", refreshed_count),
            crate::debug_log::field("count", cards.len()),
            crate::debug_log::field("rewrote_index", changed),
        ],
    );

    if changed {
        index::warn_if_save_fails(
            index::save_index(&index_state, key),
            "failed to refresh character index",
        );
    }

    cards.sort_by(|a, b| a.name.cmp(&b.name));
    cards
}
