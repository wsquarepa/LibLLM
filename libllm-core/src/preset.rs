use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::{Message, Role};

const DEFAULT_INSTRUCT_PRESET: &str = "Mistral V3-Tekken";
const REASONING_OFF: &str = "OFF";

const BUILTIN_INSTRUCT: &[(&str, &str)] = &[
    (
        "Mistral V3-Tekken",
        include_str!("presets/instruct/mistral_v3_tekken.json"),
    ),
    (
        "Llama 3 Instruct",
        include_str!("presets/instruct/llama3_instruct.json"),
    ),
    ("ChatML", include_str!("presets/instruct/chatml.json")),
    ("Phi", include_str!("presets/instruct/phi.json")),
    ("Alpaca", include_str!("presets/instruct/alpaca.json")),
];

const BUILTIN_REASONING: &[(&str, &str)] =
    &[("DeepSeek", include_str!("presets/reasoning/deepseek.json"))];

const DEFAULT_TEMPLATE_PRESET: &str = "Default";

const BUILTIN_TEMPLATE: &[(&str, &str)] =
    &[("Default", include_str!("presets/template/default.json"))];

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Complete,
    Continuation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopSequence {
    Single(String),
    Multiple(Vec<String>),
}

impl Default for StopSequence {
    fn default() -> Self {
        Self::Multiple(Vec::new())
    }
}

impl StopSequence {
    pub fn as_vec(&self) -> Vec<String> {
        match self {
            Self::Single(s) => {
                if s.is_empty() {
                    Vec::new()
                } else {
                    vec![s.clone()]
                }
            }
            Self::Multiple(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstructPreset {
    pub name: String,
    #[serde(default)]
    pub input_sequence: String,
    #[serde(default)]
    pub output_sequence: String,
    #[serde(default)]
    pub system_sequence: String,
    #[serde(default)]
    pub input_suffix: String,
    #[serde(default)]
    pub output_suffix: String,
    #[serde(default)]
    pub system_suffix: String,
    #[serde(default)]
    pub first_input_sequence: String,
    #[serde(default)]
    pub last_input_sequence: String,
    #[serde(default)]
    pub first_output_sequence: String,
    #[serde(default)]
    pub last_output_sequence: String,
    #[serde(default)]
    pub last_system_sequence: String,
    #[serde(default)]
    pub separator_sequence: String,
    #[serde(default)]
    pub stop_sequence: StopSequence,
    #[serde(default)]
    pub wrap: bool,
    #[serde(default)]
    pub system_same_as_user: bool,
    #[serde(default)]
    pub names_behavior: String,
    #[serde(default)]
    pub story_string_prefix: String,
    #[serde(default)]
    pub story_string_suffix: String,
    #[serde(default)]
    pub user_alignment_message: String,
    #[serde(default)]
    pub sequences_as_stop_strings: bool,
    #[serde(default)]
    pub activation_regex: String,
    #[serde(rename = "macro", default)]
    pub macro_enabled: bool,
    #[serde(default)]
    pub skip_examples: bool,
}

impl InstructPreset {
    pub fn raw() -> Self {
        Self {
            name: "Raw".to_owned(),
            input_suffix: "\n".to_owned(),
            output_suffix: "\n".to_owned(),
            system_suffix: "\n".to_owned(),
            ..Default::default()
        }
    }

    pub fn render(&self, messages: &[&Message], system_prompt: Option<&str>) -> String {
        self.render_with_mode(messages, system_prompt, RenderMode::Complete)
    }

    pub fn render_continuation(
        &self,
        messages: &[&Message],
        system_prompt: Option<&str>,
    ) -> String {
        self.render_with_mode(messages, system_prompt, RenderMode::Continuation)
    }

    fn render_with_mode(
        &self,
        messages: &[&Message],
        system_prompt: Option<&str>,
        mode: RenderMode,
    ) -> String {
        let mut prompt = String::new();
        let msg_count = messages.len();
        let mut system_emitted = self.append_prefixed_system_prompt(&mut prompt, system_prompt);

        let mut is_first_user = true;
        let mut is_first_assistant = true;

        for (i, msg) in messages.iter().enumerate() {
            let is_last = i == msg_count - 1;

            if !self.separator_sequence.is_empty() && i > 0 {
                prompt.push_str(&self.separator_sequence);
            }

            match msg.role {
                Role::User => {
                    let seq = self.select_input_sequence(is_first_user, is_last);
                    prompt.push_str(seq);

                    if is_first_user && !system_emitted {
                        self.append_inline_system_prompt(&mut prompt, system_prompt);
                        system_emitted = true;
                    }

                    prompt.push_str(&msg.content);
                    prompt.push_str(&self.input_suffix);
                    is_first_user = false;
                }
                Role::Assistant => {
                    let seq = self.select_output_sequence(is_first_assistant, is_last);
                    prompt.push_str(seq);
                    prompt.push_str(&msg.content);
                    if mode == RenderMode::Complete || !is_last {
                        prompt.push_str(&self.output_suffix);
                    }
                    is_first_assistant = false;
                }
                Role::System => {
                    let seq = self.effective_system_sequence();
                    let suffix = self.effective_system_suffix();
                    prompt.push_str(seq);
                    prompt.push_str(&msg.content);
                    prompt.push_str(suffix);
                }
            }
        }

        if messages
            .last()
            .is_some_and(|m| m.role == Role::User || m.role == Role::System)
        {
            let seq = self.select_output_sequence(is_first_assistant, false);
            prompt.push_str(seq);
        }

        prompt
    }

    fn append_prefixed_system_prompt(
        &self,
        prompt: &mut String,
        system_prompt: Option<&str>,
    ) -> bool {
        if let Some(sys) = system_prompt
            && !self.system_same_as_user
        {
            prompt.push_str(self.effective_system_sequence());
            prompt.push_str(sys);
            prompt.push_str(self.effective_system_suffix());
            return true;
        }

        false
    }

    fn append_inline_system_prompt(&self, prompt: &mut String, system_prompt: Option<&str>) {
        if let Some(sys) = system_prompt {
            prompt.push_str(sys);
            prompt.push_str("\n\n");
        }
    }

    pub fn stop_tokens(&self) -> Vec<String> {
        let mut tokens = self.stop_sequence.as_vec();
        if self.sequences_as_stop_strings {
            let trimmed = self.input_sequence.trim();
            if !trimmed.is_empty() && !tokens.iter().any(|t| t == trimmed) {
                tokens.push(trimmed.to_owned());
            }
        }
        tokens
    }

    fn effective_system_sequence(&self) -> &str {
        if self.system_same_as_user || self.system_sequence.is_empty() {
            &self.input_sequence
        } else {
            &self.system_sequence
        }
    }

    fn effective_system_suffix(&self) -> &str {
        if self.system_same_as_user || self.system_suffix.is_empty() {
            &self.input_suffix
        } else {
            &self.system_suffix
        }
    }

    fn select_input_sequence(&self, is_first: bool, is_last: bool) -> &str {
        if is_first && !self.first_input_sequence.is_empty() {
            &self.first_input_sequence
        } else if is_last && !self.last_input_sequence.is_empty() {
            &self.last_input_sequence
        } else {
            &self.input_sequence
        }
    }

    fn select_output_sequence(&self, is_first: bool, is_last: bool) -> &str {
        if is_first && !self.first_output_sequence.is_empty() {
            &self.first_output_sequence
        } else if is_last && !self.last_output_sequence.is_empty() {
            &self.last_output_sequence
        } else {
            &self.output_sequence
        }
    }
}

pub fn instruct_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("instruct")
}

pub fn reasoning_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("reasoning")
}

pub fn template_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("template")
}

pub fn ensure_default_presets() {
    write_defaults_if_dir_missing(&instruct_presets_dir(), BUILTIN_INSTRUCT);
    write_defaults_if_dir_missing(&reasoning_presets_dir(), BUILTIN_REASONING);
    write_defaults_if_dir_missing(&template_presets_dir(), BUILTIN_TEMPLATE);
}

fn write_defaults_if_dir_missing(dir: &Path, builtins: &[(&str, &str)]) {
    let already_existed = dir.exists();
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if already_existed {
        return;
    }
    for (name, json) in builtins {
        let path = dir.join(format!("{name}.json"));
        let _ = std::fs::write(&path, json);
    }
}

fn backward_compat_alias(name: &str) -> Option<&'static str> {
    match name {
        "llama2" | "mistral" => Some("Mistral V3-Tekken"),
        "chatml" => Some("ChatML"),
        "phi" => Some("Phi"),
        "alpaca" => Some("Alpaca"),
        _ => None,
    }
}

fn load_json_from_dir<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> Option<T> {
    let entries = std::fs::read_dir(dir).ok()?;
    let name_lower = name.to_lowercase();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if stem == name_lower
                && let Ok(contents) = std::fs::read_to_string(&path)
            {
                return serde_json::from_str(&contents).ok();
            }
        }
    }

    None
}

fn list_json_names_in_dir(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && let Some(stem) = path.file_stem()
        {
            names.push(stem.to_string_lossy().to_string());
        }
    }
    names.sort();
    names
}

fn load_builtin_instruct(name: &str) -> Option<InstructPreset> {
    let name_lower = name.to_lowercase();
    for (builtin_name, json) in BUILTIN_INSTRUCT {
        if builtin_name.to_lowercase() == name_lower {
            return serde_json::from_str(json).ok();
        }
    }
    None
}

pub fn resolve_instruct_preset(name: &str) -> InstructPreset {
    if name.eq_ignore_ascii_case("raw") {
        return InstructPreset::raw();
    }

    let resolved_name = backward_compat_alias(name).unwrap_or(name);

    if let Some(preset) = load_json_from_dir(&instruct_presets_dir(), resolved_name) {
        return preset;
    }

    if let Some(preset) = load_builtin_instruct(resolved_name) {
        return preset;
    }

    load_builtin_instruct(DEFAULT_INSTRUCT_PRESET).unwrap()
}

pub fn list_instruct_preset_names() -> Vec<String> {
    let mut names = list_json_names_in_dir(&instruct_presets_dir());
    let mut seen: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();

    for (builtin_name, _) in BUILTIN_INSTRUCT {
        if seen.insert(builtin_name.to_lowercase()) {
            names.push((*builtin_name).to_owned());
        }
    }

    if seen.insert("raw".to_owned()) {
        names.push("Raw".to_owned());
    }

    names
}

pub fn list_reasoning_preset_names() -> Vec<String> {
    let mut names = vec![REASONING_OFF.to_owned()];
    let mut seen: HashSet<String> = HashSet::from([REASONING_OFF.to_lowercase()]);

    for dir_name in list_json_names_in_dir(&reasoning_presets_dir()) {
        if seen.insert(dir_name.to_lowercase()) {
            names.push(dir_name);
        }
    }

    for (builtin_name, _) in BUILTIN_REASONING {
        if seen.insert(builtin_name.to_lowercase()) {
            names.push((*builtin_name).to_owned());
        }
    }

    names
}

fn load_builtin_reasoning(name: &str) -> Option<ReasoningPreset> {
    let name_lower = name.to_lowercase();
    for (builtin_name, json) in BUILTIN_REASONING {
        if builtin_name.to_lowercase() == name_lower {
            return serde_json::from_str(json).ok();
        }
    }
    None
}

pub fn resolve_reasoning_preset(name: &str) -> Option<ReasoningPreset> {
    if name.eq_ignore_ascii_case(REASONING_OFF) || name.is_empty() {
        return None;
    }
    if let Some(preset) = load_json_from_dir(&reasoning_presets_dir(), name) {
        return Some(preset);
    }
    load_builtin_reasoning(name)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningPreset {
    pub name: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub separator: String,
}

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

pub struct ContextVars {
    pub system: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub persona: String,
    pub wi_before: String,
    pub wi_after: String,
    pub mes_examples: String,
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

pub fn resolve_template_preset(name: &str) -> ContextPreset {
    if let Some(preset) = load_json_from_dir(&template_presets_dir(), name) {
        return preset;
    }
    if let Some(preset) = load_builtin_template(name) {
        return preset;
    }
    load_builtin_template(DEFAULT_TEMPLATE_PRESET).unwrap()
}

pub fn list_template_preset_names() -> Vec<String> {
    let mut names = list_json_names_in_dir(&template_presets_dir());
    let mut seen: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();

    for (builtin_name, _) in BUILTIN_TEMPLATE {
        if seen.insert(builtin_name.to_lowercase()) {
            names.push((*builtin_name).to_owned());
        }
    }

    names
}
