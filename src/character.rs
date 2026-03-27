use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

const EXT_ENCRYPTED: &str = "character";
const EXT_PLAINTEXT: &str = "json";

pub fn resolve_card_path(dir: &Path, slug: &str) -> PathBuf {
    let encrypted = dir.join(format!("{slug}.{EXT_ENCRYPTED}"));
    if encrypted.exists() {
        return encrypted;
    }
    dir.join(format!("{slug}.{EXT_PLAINTEXT}"))
}

fn card_extension(key: Option<&DerivedKey>) -> &'static str {
    if key.is_some() { EXT_ENCRYPTED } else { EXT_PLAINTEXT }
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
    let raw: RawCard = serde_json::from_str(json_str).context("failed to parse character card JSON")?;

    let data = raw.data.as_ref();
    let pick = |data_field: Option<&str>, top_field: Option<&str>| -> String {
        data_field
            .filter(|s| !s.is_empty())
            .or(top_field.filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_owned()
    };

    let name = pick(
        data.and_then(|d| d.name.as_deref()),
        raw.name.as_deref(),
    );
    if name.is_empty() {
        bail!("character card has no name");
    }

    Ok(CharacterCard {
        name,
        description: pick(data.and_then(|d| d.description.as_deref()), raw.description.as_deref()),
        personality: pick(data.and_then(|d| d.personality.as_deref()), raw.personality.as_deref()),
        scenario: pick(data.and_then(|d| d.scenario.as_deref()), raw.scenario.as_deref()),
        first_mes: pick(data.and_then(|d| d.first_mes.as_deref()), raw.first_mes.as_deref()),
        mes_example: pick(data.and_then(|d| d.mes_example.as_deref()), raw.mes_example.as_deref()),
        system_prompt: data.and_then(|d| d.system_prompt.as_deref()).unwrap_or("").to_owned(),
        post_history_instructions: data.and_then(|d| d.post_history_instructions.as_deref()).unwrap_or("").to_owned(),
        alternate_greetings: data.and_then(|d| d.alternate_greetings.clone()).unwrap_or_default(),
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

pub fn import_card(source: &Path) -> Result<CharacterCard> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => {
            let contents = std::fs::read_to_string(source)
                .context(format!("failed to read character card: {}", source.display()))?;
            parse_card_json(&contents)
        }
        "png" => {
            let bytes = std::fs::read(source)
                .context(format!("failed to read PNG file: {}", source.display()))?;
            let json = extract_png_card(&bytes)?;
            parse_card_json(&json)
        }
        _ => bail!("unsupported character card format: .{ext} (expected .json or .png)"),
    }
}

pub fn build_system_prompt(card: &CharacterCard) -> String {
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
    let ext = card_extension(key);
    let path = dir.join(format!("{slug}.{ext}"));
    let json = serde_json::to_string_pretty(card).context("failed to serialize character card")?;
    let data = match key {
        Some(k) => crate::crypto::encrypt(json.as_bytes(), k)?,
        None => json.into_bytes(),
    };
    std::fs::write(&path, data).context(format!("failed to write character card: {}", path.display()))?;
    Ok(path)
}

pub fn load_card(path: &Path, key: Option<&DerivedKey>) -> Result<CharacterCard> {
    let raw = std::fs::read(path)
        .context(format!("failed to read character card: {}", path.display()))?;
    let contents = if crate::crypto::is_encrypted(&raw) {
        let key = key.ok_or_else(|| anyhow::anyhow!("encrypted character card but no passkey available"))?;
        let decrypted = crate::crypto::decrypt(&raw, key)?;
        String::from_utf8(decrypted).context("decrypted character card is not valid UTF-8")?
    } else {
        String::from_utf8(raw).context("character card is not valid UTF-8")?
    };
    serde_json::from_str(&contents).context("failed to parse character card")
}

pub fn auto_import_png_cards(dir: &Path, key: Option<&DerivedKey>) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read characters dir: {e}"));
            return warnings;
        }
    };

    let png_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();

    for png_path in png_paths {
        let display = png_path.display().to_string();
        let bytes = match std::fs::read(&png_path) {
            Ok(b) => b,
            Err(e) => { warnings.push(format!("skipped {display}: {e}")); continue; }
        };
        let json = match extract_png_card(&bytes) {
            Ok(j) => j,
            Err(e) => { warnings.push(format!("skipped {display}: {e}")); continue; }
        };
        let card = match parse_card_json(&json) {
            Ok(c) => c,
            Err(e) => { warnings.push(format!("skipped {display}: {e}")); continue; }
        };
        match save_card(&card, dir, key) {
            Ok(_) => { let _ = std::fs::remove_file(&png_path); }
            Err(e) => { warnings.push(format!("failed to save {display}: {e}")); }
        }
    }

    warnings
}

pub fn encrypt_plaintext_cards(dir: &Path, key: &DerivedKey) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read characters dir: {e}"));
            return warnings;
        }
    };

    let json_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    for path in json_paths {
        let display = path.display().to_string();
        let raw = match std::fs::read(&path) {
            Ok(r) => r,
            Err(e) => { warnings.push(format!("skipped {display}: {e}")); continue; }
        };
        if crate::crypto::is_encrypted(&raw) {
            continue;
        }
        let card = match serde_json::from_slice::<CharacterCard>(&raw) {
            Ok(c) => c,
            Err(e) => { warnings.push(format!("skipped {display}: {e}")); continue; }
        };
        match save_card(&card, dir, Some(key)) {
            Ok(_) => { let _ = std::fs::remove_file(&path); }
            Err(e) => { warnings.push(format!("failed to encrypt {display}: {e}")); }
        }
    }

    warnings
}

pub fn list_cards(dir: &Path, key: Option<&DerivedKey>) -> Vec<CharacterEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut cards: Vec<CharacterEntry> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == EXT_ENCRYPTED || ext == EXT_PLAINTEXT))
        .filter_map(|path| {
            let slug = path.file_stem()?.to_string_lossy().to_string();
            let card = load_card(&path, key).ok()?;
            Some(CharacterEntry {
                name: card.name,
                slug,
            })
        })
        .collect();

    cards.sort_by(|a, b| a.name.cmp(&b.name));
    cards
}
