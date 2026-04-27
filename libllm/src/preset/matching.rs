//! Match a server-supplied Jinja `chat_template` against the user's instruct presets
//! by rendering both sides over a fixed canonical conversation, normalizing the result,
//! and scoring with normalized Levenshtein distance.

use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior, Value};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::preset::InstructPreset;
use crate::session::{Message, Role};

const BOS_PREFIXES: &[&str] = &[
    "<|begin_of_text|>",
    "<|begin▁of▁sentence|>",
    "[BOS]",
    "<s>",
];

/// Strip leading BOS-like prefix, NFC normalize, trim, collapse whitespace runs.
pub fn normalize(s: &str) -> String {
    let stripped_nul = s.trim_end_matches('\0');
    let mut after_bos = stripped_nul;
    for prefix in BOS_PREFIXES {
        if let Some(rest) = after_bos.strip_prefix(prefix) {
            after_bos = rest;
            break;
        }
    }
    let nfc: String = after_bos.nfc().collect();
    let trimmed = nfc.trim();
    collapse_whitespace(trimmed)
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_run = 0_usize;
    let mut space_run = 0_usize;
    for ch in s.chars() {
        match ch {
            '\n' => {
                space_run = 0;
                newline_run += 1;
                if newline_run <= 2 {
                    out.push('\n');
                }
            }
            ' ' | '\t' => {
                newline_run = 0;
                space_run += 1;
                if space_run <= 1 {
                    out.push(' ');
                }
            }
            other => {
                newline_run = 0;
                space_run = 0;
                out.push(other);
            }
        }
    }
    out
}

/// The fixed conversation we feed both sides during matching. Constants are public
/// so tests can construct identical inputs.
pub const CANONICAL_SYSTEM: &str = "S";
pub const CANONICAL_USER: &str = "U";
pub const CANONICAL_ASSISTANT: &str = "A";

#[derive(Debug, Clone, Serialize)]
pub struct CanonicalMessage {
    pub role: &'static str,
    pub content: &'static str,
}

#[derive(Debug, Clone)]
pub struct CanonicalContext {
    pub messages: Vec<CanonicalMessage>,
    pub bos_token: &'static str,
    pub eos_token: &'static str,
    pub add_generation_prompt: bool,
}

impl CanonicalContext {
    pub fn fixed() -> Self {
        Self {
            messages: vec![
                CanonicalMessage { role: "system", content: CANONICAL_SYSTEM },
                CanonicalMessage { role: "user", content: CANONICAL_USER },
                CanonicalMessage { role: "assistant", content: CANONICAL_ASSISTANT },
            ],
            bos_token: "",
            eos_token: "",
            add_generation_prompt: true,
        }
    }
}

/// Render a server-supplied Jinja `chat_template` string against the canonical context.
/// Uses `UndefinedBehavior::Lenient` so missing variables (`tools`, `documents`, etc.)
/// render as empty rather than raising.
pub fn render_jinja(template: &str, ctx: &CanonicalContext) -> Result<String> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Lenient);
    env.add_template("chat", template)
        .context("failed to parse server chat_template")?;
    let tmpl = env.get_template("chat").expect("just added");
    let value = Value::from_serialize(&serde_json::json!({
        "messages": ctx.messages,
        "bos_token": ctx.bos_token,
        "eos_token": ctx.eos_token,
        "add_generation_prompt": ctx.add_generation_prompt,
    }));
    tmpl.render(value)
        .context("failed to render server chat_template")
}

/// Render an `InstructPreset` against the same canonical conversation `render_jinja` uses,
/// reusing the existing production renderer (`InstructPreset::render_continuation`) so
/// matching reflects what we actually send to the API.
pub fn render_preset(preset: &InstructPreset, _ctx: &CanonicalContext) -> String {
    let messages = vec![
        Message::new(Role::User, CANONICAL_USER.to_owned()),
        Message::new(Role::Assistant, CANONICAL_ASSISTANT.to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();
    preset.render_continuation(&refs, Some(CANONICAL_SYSTEM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_nul() {
        assert_eq!(normalize("hello\0\0\0"), "hello");
    }

    #[test]
    fn strips_known_bos_prefixes() {
        assert_eq!(normalize("<|begin_of_text|>hello"), "hello");
        assert_eq!(normalize("<s>hello"), "hello");
        assert_eq!(normalize("[BOS]hello"), "hello");
    }

    #[test]
    fn trims_leading_and_trailing_whitespace() {
        assert_eq!(normalize("   hello   "), "hello");
    }

    #[test]
    fn collapses_repeated_spaces() {
        assert_eq!(normalize("a    b"), "a b");
    }

    #[test]
    fn collapses_three_or_more_newlines_to_two() {
        assert_eq!(normalize("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn preserves_two_newlines_unchanged() {
        assert_eq!(normalize("a\n\nb"), "a\n\nb");
    }

    #[test]
    fn does_not_lowercase() {
        assert_eq!(normalize("System User"), "System User");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn nfc_normalizes_decomposed_chars() {
        // "é" composed vs decomposed
        let composed = "\u{00E9}";       // é
        let decomposed = "e\u{0301}";    // e + combining acute
        assert_eq!(normalize(decomposed), composed);
    }

    #[test]
    fn render_jinja_basic_chatml_template() {
        let template = "{% for m in messages %}<|im_start|>{{ m.role }}\n{{ m.content }}<|im_end|>\n{% endfor %}{% if add_generation_prompt %}<|im_start|>assistant\n{% endif %}";
        let out = render_jinja(template, &CanonicalContext::fixed()).unwrap();
        assert!(out.contains("<|im_start|>system\nS<|im_end|>"));
        assert!(out.contains("<|im_start|>user\nU<|im_end|>"));
        assert!(out.contains("<|im_start|>assistant\nA<|im_end|>"));
        assert!(out.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn render_jinja_lenient_on_unknown_variable() {
        // References `tools` which we don't supply.
        let template = "{% if tools %}HAS_TOOLS{% else %}NO_TOOLS{% endif %}";
        let out = render_jinja(template, &CanonicalContext::fixed()).unwrap();
        assert_eq!(out, "NO_TOOLS");
    }

    #[test]
    fn render_jinja_parse_error_returns_err() {
        let bad = "{% for m in messages %}{{ m.role }}";
        assert!(render_jinja(bad, &CanonicalContext::fixed()).is_err());
    }

    #[test]
    fn render_preset_chatml_produces_im_start_tags() {
        let preset = crate::preset::resolve_instruct_preset("ChatML");
        let out = render_preset(&preset, &CanonicalContext::fixed());
        assert!(out.contains("<|im_start|>"));
        assert!(out.contains("<|im_end|>"));
        assert!(out.contains("system"));
        assert!(out.contains("user"));
        assert!(out.contains("assistant"));
    }

    #[test]
    fn render_preset_llama3_produces_header_id_tags() {
        let preset = crate::preset::resolve_instruct_preset("Llama 3 Instruct");
        let out = render_preset(&preset, &CanonicalContext::fixed());
        assert!(out.contains("<|start_header_id|>"));
        assert!(out.contains("<|end_header_id|>"));
        assert!(out.contains("<|eot_id|>"));
    }
}
