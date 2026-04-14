//! Character card types and SillyTavern-compatible import/export.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// A character card following the SillyTavern V2 spec, with optional V1 fallback fields.
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

/// Parses a character card from JSON, accepting both V1 (top-level fields) and V2 (`data` object) formats.
///
/// Fields in `data` take precedence over top-level fields. Returns an error if the
/// card has no name or the JSON is malformed.
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

/// Extracts a base64-encoded character card JSON from a PNG `tEXt` chunk with keyword "chara".
///
/// Returns the decoded JSON string, or an error if the PNG has no `chara` chunk.
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

/// Imports a character card from a `.json` or `.png` file on disk.
///
/// Enforces size limits (10 MB for JSON, 50 MB for PNG) and delegates to
/// `parse_card_json` or `extract_png_card` accordingly.
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

/// Assembles a system prompt from a character card's fields, optionally rendered through a context template.
///
/// Without a template, concatenates non-empty fields in a fixed order. With a template,
/// renders the story string using `ContextVars` populated from the card.
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

/// Converts a display name to a lowercase URL-safe slug (e.g. "Hello World" -> "hello-world").
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

#[cfg(test)]
mod tests {
    use super::*;

    fn full_character() -> CharacterCard {
        CharacterCard {
            name: "TestChar".to_string(),
            description: "A test character for integration tests.".to_string(),
            personality: "Helpful and precise.".to_string(),
            scenario: "In a testing environment.".to_string(),
            first_mes: "Hello, I am TestChar.".to_string(),
            mes_example: "<START>\n{{user}}: Hi\n{{char}}: Hello!".to_string(),
            system_prompt: "You are TestChar.".to_string(),
            post_history_instructions: "Stay in character.".to_string(),
            alternate_greetings: vec!["Greetings!".to_string()],
        }
    }

    #[test]
    fn parse_old_format() {
        let json = r#"{
            "name": "Alice",
            "description": "A curious adventurer",
            "personality": "Bold and inquisitive",
            "scenario": "Lost in a forest",
            "first_mes": "Where am I?",
            "mes_example": "<START>\n{{user}}: Hello\n{{char}}: Hi there!"
        }"#;

        let card = parse_card_json(json).unwrap();
        assert_eq!(card.name, "Alice");
        assert_eq!(card.description, "A curious adventurer");
        assert_eq!(card.personality, "Bold and inquisitive");
        assert_eq!(card.scenario, "Lost in a forest");
        assert_eq!(card.first_mes, "Where am I?");
    }

    #[test]
    fn parse_new_format() {
        let json = r#"{
            "data": {
                "name": "Alice",
                "description": "A curious adventurer",
                "personality": "Bold and inquisitive",
                "scenario": "Lost in a forest",
                "first_mes": "Where am I?"
            }
        }"#;

        let card = parse_card_json(json).unwrap();
        assert_eq!(card.name, "Alice");
        assert_eq!(card.description, "A curious adventurer");
        assert_eq!(card.personality, "Bold and inquisitive");
    }

    #[test]
    fn slugify_rules() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("foo--bar"), "foo-bar");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("CamelCase"), "camelcase");
        assert_eq!(slugify("special!@#chars"), "special-chars");
    }

    #[test]
    fn build_system_prompt_without_template() {
        let card = full_character();
        let prompt = build_system_prompt(&card, None);

        assert!(
            prompt.contains(&card.description),
            "prompt should contain description"
        );
        assert!(
            prompt.contains(&card.personality),
            "prompt should contain personality"
        );
        assert!(
            prompt.contains(&card.scenario),
            "prompt should contain scenario"
        );
    }

    #[test]
    fn missing_fields_default() {
        let json = r#"{ "name": "Minimal", "description": "Just a desc" }"#;
        let card = parse_card_json(json).unwrap();

        assert_eq!(card.name, "Minimal");
        assert_eq!(card.description, "Just a desc");
        assert!(card.personality.is_empty());
        assert!(card.scenario.is_empty());
        assert!(card.first_mes.is_empty());
        assert!(card.mes_example.is_empty());
        assert!(card.system_prompt.is_empty());
        assert!(card.post_history_instructions.is_empty());
        assert!(card.alternate_greetings.is_empty());
    }

    #[test]
    fn parse_empty_name_errors() {
        let json = r#"{"name": "", "description": "x"}"#;
        let result = parse_card_json(json);
        assert!(result.is_err(), "empty name should return an error");
    }

    #[test]
    fn build_system_prompt_with_template_preset() {
        let card = full_character();
        let preset = crate::preset::resolve_template_preset("Default");
        let result = build_system_prompt(&card, Some(&preset));

        assert!(!result.is_empty(), "rendered prompt should not be empty");
        assert!(
            result.contains(&card.name) || result.contains(&card.description),
            "rendered prompt should contain character info"
        );
    }

    #[test]
    fn data_fields_override_top_level() {
        let json = r#"{
            "name": "TopLevel",
            "description": "top desc",
            "data": {
                "name": "DataLevel",
                "description": "data desc"
            }
        }"#;

        let card = parse_card_json(json).unwrap();
        assert_eq!(card.name, "DataLevel");
        assert_eq!(card.description, "data desc");
    }
}

