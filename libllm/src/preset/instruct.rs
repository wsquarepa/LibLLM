use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::session::{Message, Role};

use super::{
    BUILTIN_INSTRUCT, DEFAULT_INSTRUCT_PRESET, backward_compat_alias, instruct_presets_dir,
    list_json_names_in_dir, load_json_from_dir,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Complete,
    Continuation,
}

/// One or more stop sequences that terminate generation, accepting both single and array JSON forms.
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

/// An instruct-mode prompt template defining input/output/system sequences and stop tokens.
///
/// Controls how multi-turn messages are formatted into a single prompt string for the
/// completions API. Supports first/last sequence overrides and `system_same_as_user` mode.
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
                Role::System | Role::Summary => {
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
            .is_some_and(|m| m.role == Role::User || m.role == Role::System || m.role == Role::Summary)
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

fn load_builtin_instruct(name: &str) -> Option<InstructPreset> {
    let name_lower = name.to_lowercase();
    for (builtin_name, json) in BUILTIN_INSTRUCT {
        if builtin_name.to_lowercase() == name_lower {
            return serde_json::from_str(json).ok();
        }
    }
    None
}

/// Resolves an instruct preset by name, checking user files first, then builtins, then the default.
///
/// The special name "Raw" returns a minimal preset with newline-only suffixes. Legacy
/// aliases like "chatml" are mapped to their canonical names.
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

/// Returns all available instruct preset names, merging user files with builtins.
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

#[cfg(test)]
mod tests {
    use crate::session::{Message, Role};

    use super::*;

    fn setup_data_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::config::set_data_dir(dir.path().to_path_buf()).ok();
        dir
    }

    fn user_msg(content: &str) -> Message {
        Message::new(Role::User, content.to_string())
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new(Role::Assistant, content.to_string())
    }

    fn system_msg(content: &str) -> Message {
        Message::new(Role::System, content.to_string())
    }

    #[test]
    fn raw_preset_render() {
        let preset = InstructPreset::raw();
        let msgs = [user_msg("Hello"), assistant_msg("Hi there")];
        let refs: Vec<&_> = msgs.iter().collect();
        let output = preset.render(&refs, None);

        assert!(output.contains("Hello"), "user content missing");
        assert!(output.contains("Hi there"), "assistant content missing");
        assert!(
            !output.contains("<|"),
            "raw preset should not contain special tokens"
        );
    }

    #[test]
    fn chatml_render() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let msgs = [system_msg("You are helpful."),
            user_msg("Hi"),
            assistant_msg("Hello!")];
        let refs: Vec<&_> = msgs.iter().collect();
        let output = preset.render(&refs, None);

        assert!(output.contains("<|im_start|>system"), "missing system tag");
        assert!(output.contains("<|im_start|>user"), "missing user tag");
        assert!(
            output.contains("<|im_start|>assistant"),
            "missing assistant tag"
        );
        assert!(
            output.contains("You are helpful."),
            "system content missing"
        );
        assert!(output.contains("Hi"), "user content missing");
        assert!(output.contains("Hello!"), "assistant content missing");
        assert!(output.contains("<|im_end|>"), "missing im_end tag");
    }

    #[test]
    fn llama3_render() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("Llama 3 Instruct");
        let msgs = [user_msg("Hi"), assistant_msg("Hello!")];
        let refs: Vec<&_> = msgs.iter().collect();
        let output = preset.render(&refs, Some("System text"));

        assert!(
            output.contains("<|start_header_id|>system<|end_header_id|>"),
            "missing system header"
        );
        assert!(
            output.contains("<|start_header_id|>user<|end_header_id|>"),
            "missing user header"
        );
        assert!(
            output.contains("<|start_header_id|>assistant<|end_header_id|>"),
            "missing assistant header"
        );
        assert!(output.contains("System text"), "system prompt missing");
        assert!(output.contains("<|eot_id|>"), "missing eot_id");
    }

    #[test]
    fn system_prompt_injection() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let msgs = [user_msg("Hi")];
        let refs: Vec<&_> = msgs.iter().collect();

        let without = preset.render(&refs, None);
        let with_sys = preset.render(&refs, Some("Be helpful."));

        assert!(
            !without.contains("Be helpful."),
            "system prompt should be absent"
        );
        assert!(
            with_sys.contains("Be helpful."),
            "system prompt should be present"
        );
    }

    #[test]
    fn stop_tokens_chatml() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let tokens = preset.stop_tokens();

        assert!(
            tokens.iter().any(|t| t == "<|im_end|>"),
            "ChatML should have <|im_end|> stop token, got: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.contains("<|im_start|>user")),
            "ChatML with sequences_as_stop_strings should include input_sequence, got: {tokens:?}"
        );
    }

    #[test]
    fn stop_tokens_llama3() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("Llama 3 Instruct");
        let tokens = preset.stop_tokens();

        assert!(
            tokens.iter().any(|t| t == "<|eot_id|>"),
            "Llama 3 should have <|eot_id|> stop token, got: {tokens:?}"
        );
    }

    #[test]
    fn all_instruct_presets_load() {
        let _dir = setup_data_dir();
        let names = list_instruct_preset_names();
        assert!(!names.is_empty(), "should have at least one preset");
        for name in &names {
            let p = resolve_instruct_preset(name);
            assert!(!p.name.is_empty(), "preset {name} should have a name");
        }
    }

    #[test]
    fn empty_message_list() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let refs: Vec<&Message> = Vec::new();
        let output = preset.render(&refs, None);
        let _ = output;
    }

    #[test]
    fn empty_message_list_with_system_prompt() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let refs: Vec<&Message> = Vec::new();
        let output = preset.render(&refs, Some("System"));
        assert!(
            output.contains("System"),
            "system prompt should appear even with no messages"
        );
    }

    #[test]
    fn multi_turn_conversation() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let msgs = [user_msg("Turn 1"),
            assistant_msg("Reply 1"),
            user_msg("Turn 2"),
            assistant_msg("Reply 2"),
            user_msg("Turn 3"),
            assistant_msg("Reply 3")];
        let refs: Vec<&_> = msgs.iter().collect();
        let output = preset.render(&refs, None);

        for content in &["Turn 1", "Reply 1", "Turn 2", "Reply 2", "Turn 3", "Reply 3"] {
            assert!(output.contains(content), "missing content: {content}");
        }

        let user_count = output.matches("<|im_start|>user").count();
        let assistant_count = output.matches("<|im_start|>assistant").count();
        assert_eq!(user_count, 3, "expected 3 user tags, got {user_count}");
        assert_eq!(
            assistant_count, 3,
            "expected 3 assistant tags, got {assistant_count}"
        );
    }

    #[test]
    fn special_characters_in_messages() {
        let _dir = setup_data_dir();
        let preset = resolve_instruct_preset("ChatML");
        let msgs = [user_msg("line1\nline2\nline3"),
            assistant_msg("<tag>content</tag> | pipe | test")];
        let refs: Vec<&_> = msgs.iter().collect();
        let output = preset.render(&refs, None);

        assert!(
            output.contains("line1\nline2\nline3"),
            "newlines should be preserved"
        );
        assert!(
            output.contains("<tag>content</tag> | pipe | test"),
            "angle brackets and pipes should be preserved"
        );
    }

    #[test]
    fn render_continuation_omits_output_suffix() {
        let preset = InstructPreset {
            name: "test".to_owned(),
            output_suffix: "\n".to_owned(),
            input_suffix: "\n".to_owned(),
            ..Default::default()
        };
        let msgs = [user_msg("Hello"), assistant_msg("Hi")];
        let refs: Vec<&_> = msgs.iter().collect();

        let complete = preset.render(&refs, None);
        let continuation = preset.render_continuation(&refs, None);

        assert!(
            complete.ends_with("Hi\n"),
            "complete render should end with output_suffix, got: {complete:?}"
        );
        assert!(
            continuation.ends_with("Hi"),
            "continuation render should not append output_suffix on last assistant msg, got: {continuation:?}"
        );
    }

    #[test]
    fn render_continuation_vs_render_differ() {
        let preset = InstructPreset {
            name: "test".to_owned(),
            output_suffix: "</s>".to_owned(),
            input_suffix: "\n".to_owned(),
            ..Default::default()
        };
        let msgs = [user_msg("Q"), assistant_msg("A")];
        let refs: Vec<&_> = msgs.iter().collect();

        let complete = preset.render(&refs, None);
        let continuation = preset.render_continuation(&refs, None);

        assert_ne!(
            complete, continuation,
            "render and render_continuation must produce different output"
        );
        assert!(
            complete.contains("</s>"),
            "complete output should contain output_suffix"
        );
        assert!(
            !continuation.ends_with("</s>"),
            "continuation should not end with output_suffix"
        );
    }

    #[test]
    fn backward_compat_alias_chatml() {
        let result = super::super::backward_compat_alias("chatml");
        assert_eq!(
            result,
            Some("ChatML"),
            "\"chatml\" should alias to \"ChatML\""
        );
    }
}
