use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    BUILTIN_TEMPLATE, DEFAULT_TEMPLATE_PRESET, list_json_names_in_dir, load_json_from_dir,
};

/// A context template that controls how character description, persona, and worldbook entries
/// are assembled into the story string portion of the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextPreset {
    pub name: String,
    #[serde(default)]
    pub story_string: String,
    #[serde(default)]
    pub example_separator: String,
    #[serde(default)]
    pub chat_start: String,
    #[serde(default)]
    pub story_string_position: u32,
    #[serde(default)]
    pub story_string_depth: u32,
    #[serde(default)]
    pub story_string_role: u32,
}

/// Variable bindings for context template rendering (handlebars-style `{{variable}}`).
pub struct ContextVars {
    pub system: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub persona: String,
    pub wi_before: String,
    pub wi_after: String,
    pub mes_examples: String,
    pub characters_block: String,
    pub roster_block: String,
    pub active_speaker: String,
    pub other_speakers: String,
}

impl ContextPreset {
    pub fn render_story_string(&self, vars: &ContextVars) -> String {
        render_handlebars_template(&self.story_string, vars)
    }
}

fn render_handlebars_template(template: &str, vars: &ContextVars) -> String {
    let mut result = String::with_capacity(template.len());
    let mut cursor = 0;

    while cursor < template.len() {
        if let Some(pos) = template[cursor..].find("{{") {
            let abs_pos = cursor + pos;
            result.push_str(&template[cursor..abs_pos]);

            if template[abs_pos..].starts_with("{{#if ") {
                if let Some(block_end) = parse_if_block(&template[abs_pos..]) {
                    let (var_name, body) = block_end;
                    let value = lookup_var(vars, var_name);
                    if !value.is_empty() {
                        result.push_str(&render_handlebars_template(body, vars));
                    }
                    let close_tag = "{{/if}}";
                    let skip = template[abs_pos..]
                        .find(close_tag)
                        .map(|p| p + close_tag.len())
                        .unwrap_or(template.len() - abs_pos);
                    cursor = abs_pos + skip;
                } else {
                    result.push_str("{{");
                    cursor = abs_pos + 2;
                }
            } else if template[abs_pos..].starts_with("{{trim}}") {
                let trimmed = result.trim_end().len();
                result.truncate(trimmed);
                cursor = abs_pos + "{{trim}}".len();
            } else if let Some(close) = template[abs_pos + 2..].find("}}") {
                let var_name = &template[abs_pos + 2..abs_pos + 2 + close];
                let value = lookup_var(vars, var_name);
                result.push_str(&value);
                cursor = abs_pos + 2 + close + 2;
            } else {
                result.push_str("{{");
                cursor = abs_pos + 2;
            }
        } else {
            result.push_str(&template[cursor..]);
            break;
        }
    }

    result
}

fn parse_if_block(s: &str) -> Option<(&str, &str)> {
    let prefix = "{{#if ";
    if !s.starts_with(prefix) {
        return None;
    }
    let after_prefix = &s[prefix.len()..];
    let close_brace = after_prefix.find("}}")?;
    let var_name = after_prefix[..close_brace].trim();
    let body_start = prefix.len() + close_brace + 2;
    let close_tag = "{{/if}}";
    let body_end = s[body_start..].find(close_tag)?;
    let body = &s[body_start..body_start + body_end];
    Some((var_name, body))
}

fn lookup_var(vars: &ContextVars, name: &str) -> String {
    match name {
        "system" => vars.system.clone(),
        "description" => vars.description.clone(),
        "personality" => vars.personality.clone(),
        "scenario" => vars.scenario.clone(),
        "persona" => vars.persona.clone(),
        "wiBefore" => vars.wi_before.clone(),
        "wiAfter" => vars.wi_after.clone(),
        "mesExamples" | "mesExamplesRaw" | "dialogueExamples" => vars.mes_examples.clone(),
        "characters_block" | "charactersBlock" => vars.characters_block.clone(),
        "roster_block" | "rosterBlock" => vars.roster_block.clone(),
        "active_speaker" | "activeSpeaker" => vars.active_speaker.clone(),
        "other_speakers" | "otherSpeakers" => vars.other_speakers.clone(),
        _ => String::new(),
    }
}

fn load_builtin_template(name: &str) -> Option<ContextPreset> {
    let name_lower = name.to_lowercase();
    for (builtin_name, json) in BUILTIN_TEMPLATE {
        if builtin_name.to_lowercase() == name_lower {
            return serde_json::from_str(json).ok();
        }
    }
    None
}

/// Resolves a context template preset by name, falling back to the "Default" builtin.
#[expect(
    clippy::expect_used,
    reason = "the default preset is embedded at compile time; a parse failure is a build defect"
)]
pub fn resolve_template_preset(name: &str, presets_dir: &Path) -> ContextPreset {
    if let Some(preset) = load_json_from_dir(presets_dir, name) {
        return preset;
    }
    if let Some(preset) = load_builtin_template(name) {
        return preset;
    }
    load_builtin_template(DEFAULT_TEMPLATE_PRESET)
        .expect("DEFAULT_TEMPLATE_PRESET is a compile-time builtin guaranteed to parse")
}

/// Returns all available context template preset names, merging user files with builtins.
pub fn list_template_preset_names(presets_dir: &Path) -> Vec<String> {
    let mut names = list_json_names_in_dir(presets_dir);
    let mut seen: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();

    for (builtin_name, _) in BUILTIN_TEMPLATE {
        if seen.insert(builtin_name.to_lowercase()) {
            names.push((*builtin_name).to_owned());
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn all_template_presets_load() {
        let dir = setup_data_dir();
        let names = list_template_preset_names(dir.path());
        assert!(
            !names.is_empty(),
            "should have at least one template preset"
        );
        for name in &names {
            let _p = resolve_template_preset(name, dir.path());
        }
    }

    #[test]
    fn render_story_string_populated() {
        let dir = setup_data_dir();
        let preset = resolve_template_preset("Default", dir.path());
        let vars = ContextVars {
            system: "SystemText".to_string(),
            description: "DescText".to_string(),
            personality: "PersonText".to_string(),
            scenario: "ScenarioText".to_string(),
            persona: "PersonaText".to_string(),
            wi_before: "WiBeforeText".to_string(),
            wi_after: "WiAfterText".to_string(),
            mes_examples: "ExampleText".to_string(),
            characters_block: String::new(),
            roster_block: String::new(),
            active_speaker: String::new(),
            other_speakers: String::new(),
        };
        let output = preset.render_story_string(&vars);

        let expected_vars = [
            ("system", "SystemText"),
            ("description", "DescText"),
            ("personality", "PersonText"),
            ("scenario", "ScenarioText"),
            ("persona", "PersonaText"),
            ("wi_before", "WiBeforeText"),
            ("wi_after", "WiAfterText"),
        ];
        for (label, text) in &expected_vars {
            assert!(
                output.contains(text),
                "{label} not substituted, output: {output:?}"
            );
        }
    }

    #[test]
    fn render_story_string_empty_vars() {
        let dir = setup_data_dir();
        let preset = resolve_template_preset("Default", dir.path());
        let vars = ContextVars {
            system: String::new(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            persona: String::new(),
            wi_before: String::new(),
            wi_after: String::new(),
            mes_examples: String::new(),
            characters_block: String::new(),
            roster_block: String::new(),
            active_speaker: String::new(),
            other_speakers: String::new(),
        };
        let output = preset.render_story_string(&vars);

        assert!(
            !output.contains("{{"),
            "leftover template markers in output: {output:?}"
        );
    }

    #[test]
    fn render_story_string_resolves_group_chat_vars() {
        let preset = ContextPreset {
            name: "test".to_owned(),
            story_string:
                "{{characters_block}}|{{roster_block}}|{{active_speaker}}|{{other_speakers}}"
                    .to_owned(),
            ..Default::default()
        };
        let vars = ContextVars {
            system: String::new(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            persona: String::new(),
            wi_before: String::new(),
            wi_after: String::new(),
            mes_examples: String::new(),
            characters_block: "<characters>...</characters>".to_owned(),
            roster_block: "<roster>...</roster>".to_owned(),
            active_speaker: "Alice".to_owned(),
            other_speakers: "Bob, Charlie".to_owned(),
        };
        let out = preset.render_story_string(&vars);
        assert_eq!(
            out,
            "<characters>...</characters>|<roster>...</roster>|Alice|Bob, Charlie"
        );
    }
}
