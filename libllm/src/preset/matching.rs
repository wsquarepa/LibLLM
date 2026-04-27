//! Match a server-supplied Jinja `chat_template` against the user's instruct presets
//! by rendering both sides over a fixed canonical conversation, normalizing the result,
//! and scoring with normalized Levenshtein distance.

use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};
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
    let value = Value::from_serialize(serde_json::json!({
        "messages": ctx.messages,
        "bos_token": ctx.bos_token,
        "eos_token": ctx.eos_token,
        "add_generation_prompt": ctx.add_generation_prompt,
    }));
    tmpl.render(value)
        .context("failed to render server chat_template")
}

/// Render a preset over the scoring conversation (single user turn, no system).
///
/// Because the last message is a user turn, `render_continuation` appends the output
/// sequence, mirroring what the Jinja side produces via `add_generation_prompt: true`.
pub(crate) fn render_preset(preset: &InstructPreset) -> String {
    let messages = [Message::new(Role::User, CANONICAL_USER.to_owned())];
    let refs: Vec<&Message> = messages.iter().collect();
    preset.render_continuation(&refs, None)
}

/// 0.0 (totally different) to 1.0 (identical) similarity score using normalized Levenshtein.
pub fn score(server_normalized: &str, preset_normalized: &str) -> f64 {
    strsim::normalized_levenshtein(server_normalized, preset_normalized)
}

pub const CONFIDENT_THRESHOLD: f64 = 0.98;
pub const BEST_GUESS_THRESHOLD: f64 = 0.85;

#[derive(Debug, Clone, PartialEq)]
pub enum MatchOutcome {
    Confident { preset: String, score: f64 },
    BestGuess { preset: String, score: f64 },
    NoMatch { best_score: f64 },
}

/// Render `server_template` and each preset against the canonical context, normalize,
/// score, and return the best outcome. Skips presets that fail to render. Returns
/// `NoMatch { best_score: 0.0 }` if every preset fails or `presets` is empty.
///
/// `current_preset_name` is the name of the preset currently active in the caller. When it
/// appears in the tie set, spec edge case 3 applies: the current preset wins so the caller's
/// "already-current" check suppresses the popup without any user-visible disruption.
///
/// Tiebreak (when multiple presets share the top score and the current preset is not among them):
/// 1. preset with the smallest sum of byte lengths across distinctive sequence fields
/// 2. alphabetical by `name` (deterministic fallback)
pub fn pick_best_match(
    server_template: &str,
    presets: &[InstructPreset],
    current_preset_name: &str,
) -> MatchOutcome {
    // Single user turn, no system message. Omitting system avoids false mismatches on templates
    // that silently drop system messages (e.g., Mistral V3). Both the Jinja side (via
    // add_generation_prompt) and the preset side (last message is a user turn) append the
    // assistant prompt, keeping both sides structurally aligned.
    let ctx = CanonicalContext {
        messages: vec![CanonicalMessage { role: "user", content: CANONICAL_USER }],
        bos_token: "",
        eos_token: "",
        add_generation_prompt: true,
    };
    let server_rendered = match render_jinja(server_template, &ctx) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(error = %err, "preset.matching.jinja_render_failed");
            return MatchOutcome::NoMatch { best_score: 0.0 };
        }
    };
    let server_norm = normalize(&server_rendered);

    let scored: Vec<(f64, &InstructPreset)> = presets
        .iter()
        .map(|p| {
            let rendered = render_preset(p);
            let norm = normalize(&rendered);
            (score(&server_norm, &norm), p)
        })
        .collect();

    if scored.is_empty() {
        return MatchOutcome::NoMatch { best_score: 0.0 };
    }

    let top_score = scored
        .iter()
        .map(|(s, _)| *s)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut top: Vec<&InstructPreset> = scored
        .iter()
        .filter(|(s, _)| (*s - top_score).abs() < f64::EPSILON)
        .map(|(_, p)| *p)
        .collect();

    let winner = if let Some(current) = top.iter().find(|p| p.name == current_preset_name) {
        *current
    } else {
        top.sort_by(|a, b| {
            sequence_length_sum(a)
                .cmp(&sequence_length_sum(b))
                .then_with(|| a.name.cmp(&b.name))
        });
        top[0]
    };

    if top_score >= CONFIDENT_THRESHOLD {
        MatchOutcome::Confident { preset: winner.name.clone(), score: top_score }
    } else if top_score >= BEST_GUESS_THRESHOLD {
        MatchOutcome::BestGuess { preset: winner.name.clone(), score: top_score }
    } else {
        MatchOutcome::NoMatch { best_score: top_score }
    }
}

/// Sum of byte lengths of the distinctive sequence fields that differentiate presets.
///
/// Extracted because the `StopSequence` match adds enough branching to make the sort
/// closure unreadable if inlined.
fn sequence_length_sum(p: &InstructPreset) -> usize {
    p.input_sequence.len()
        + p.output_sequence.len()
        + p.system_sequence.len()
        + p.input_suffix.len()
        + p.output_suffix.len()
        + p.system_suffix.len()
        + match &p.stop_sequence {
            crate::preset::StopSequence::Single(s) => s.len(),
            crate::preset::StopSequence::Multiple(v) => v.iter().map(|s| s.len()).sum(),
        }
}

/// SHA-256 hex of the *normalized* server template. Stable across cosmetic
/// whitespace differences between two model files that ship the same logical template.
pub fn template_hash(server_template: &str) -> String {
    let normalized = normalize(server_template);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
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
        let out = render_preset(&preset);
        assert!(out.contains("<|im_start|>"));
        assert!(out.contains("<|im_end|>"));
        assert!(out.contains("user"));
    }

    #[test]
    fn render_preset_llama3_produces_header_id_tags() {
        let preset = crate::preset::resolve_instruct_preset("Llama 3 Instruct");
        let out = render_preset(&preset);
        assert!(out.contains("<|start_header_id|>"));
        assert!(out.contains("<|end_header_id|>"));
        assert!(out.contains("<|eot_id|>"));
    }

    #[test]
    fn score_identical_strings_is_one() {
        assert_eq!(score("hello world", "hello world"), 1.0);
    }

    #[test]
    fn score_disjoint_strings_is_low() {
        let s = score("AAAAAAAA", "ZZZZZZZZ");
        assert!(s < 0.1, "expected near-zero, got {s}");
    }

    #[test]
    fn score_similar_strings_is_high() {
        let s = score("hello world", "hello worle");
        assert!(s > 0.9, "expected near-1.0, got {s}");
    }

    #[test]
    fn score_empty_pair_is_one() {
        assert_eq!(score("", ""), 1.0);
    }

    const LLAMA3_JINJA: &str = include_str!("matching_fixtures/llama3.jinja");
    const CHATML_JINJA: &str = include_str!("matching_fixtures/chatml.jinja");
    const MISTRAL_V3_JINJA: &str = include_str!("matching_fixtures/mistral_v3.jinja");
    const NONSENSE_JINJA: &str = include_str!("matching_fixtures/nonsense.jinja");


    fn all_builtin_presets() -> Vec<InstructPreset> {
        crate::preset::BUILTIN_INSTRUCT
            .iter()
            .filter_map(|(_, json)| serde_json::from_str(json).ok())
            .collect()
    }


    #[test]
    fn pick_best_match_llama3_template_picks_llama3_preset() {
        let presets = all_builtin_presets();
        let outcome = pick_best_match(LLAMA3_JINJA, &presets, "");
        match outcome {
            MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
                assert_eq!(preset, "Llama 3 Instruct");
            }
            MatchOutcome::NoMatch { best_score } => {
                panic!("expected match for Llama-3 template, got NoMatch (best_score={best_score})");
            }
        }
    }

    #[test]
    fn pick_best_match_chatml_template_picks_chatml_preset() {
        let presets = all_builtin_presets();
        let outcome = pick_best_match(CHATML_JINJA, &presets, "");
        match outcome {
            MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
                assert_eq!(preset, "ChatML");
            }
            MatchOutcome::NoMatch { best_score } => {
                panic!("expected match for ChatML template, got NoMatch (best_score={best_score})");
            }
        }
    }

    #[test]
    fn pick_best_match_mistral_template_picks_mistral_preset() {
        let presets = all_builtin_presets();
        let outcome = pick_best_match(MISTRAL_V3_JINJA, &presets, "");
        match outcome {
            MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
                assert_eq!(preset, "Mistral V3-Tekken");
            }
            MatchOutcome::NoMatch { best_score } => {
                panic!("expected match for Mistral V3 template, got NoMatch (best_score={best_score})");
            }
        }
    }

    #[test]
    fn pick_best_match_nonsense_template_returns_no_match() {
        let presets = all_builtin_presets();
        let outcome = pick_best_match(NONSENSE_JINJA, &presets, "");
        match outcome {
            MatchOutcome::NoMatch { best_score } => {
                assert!(best_score < BEST_GUESS_THRESHOLD, "score={best_score}");
            }
            other => panic!("expected NoMatch, got {other:?}"),
        }
    }

    #[test]
    fn pick_best_match_empty_preset_list_returns_no_match() {
        let outcome = pick_best_match(LLAMA3_JINJA, &[], "");
        assert_eq!(outcome, MatchOutcome::NoMatch { best_score: 0.0 });
    }

    #[test]
    fn pick_best_match_invalid_jinja_returns_no_match() {
        let presets = all_builtin_presets();
        let outcome = pick_best_match("{% for m in messages %}{{ m.role }}", &presets, "");
        assert_eq!(outcome, MatchOutcome::NoMatch { best_score: 0.0 });
    }

    #[test]
    fn pick_best_match_tiebreak_prefers_shorter_sequence_sum() {
        // Two synthetic presets with identical rendering but different total sequence lengths.
        // stop_sequence is included in sequence_length_sum but never appears in the rendered
        // prompt, so padding it creates a length difference without changing rendered output.
        let short_preset: InstructPreset = crate::preset::BUILTIN_INSTRUCT
            .iter()
            .find(|(name, _)| *name == "ChatML")
            .and_then(|(_, json)| serde_json::from_str(json).ok())
            .expect("ChatML builtin must exist");
        let mut long_preset = short_preset.clone();
        long_preset.name = "ChatML-padded".to_owned();
        long_preset.stop_sequence = crate::preset::StopSequence::Multiple(vec![
            "<|im_end|>".to_owned(),
            "[unused-padding-string-that-does-not-render]".to_owned(),
        ]);
        let presets = vec![long_preset, short_preset];
        let outcome = pick_best_match(CHATML_JINJA, &presets, "");
        match outcome {
            MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
                assert_eq!(preset, "ChatML", "expected shorter preset to win tiebreak");
            }
            other => panic!("expected match, got {other:?}"),
        }
    }

    #[test]
    fn pick_best_match_skips_tiebreak_when_current_is_in_tie_set() {
        // Two presets that render identically (ChatML and ChatML-padded from the tiebreak test).
        // Normally ChatML wins (shorter sequence sum). Here we set current to ChatML-padded,
        // which would lose the tiebreak — but the current-in-tie-set rule must return it instead
        // so the caller's "already-current" check suppresses any popup.
        let short_preset: InstructPreset = crate::preset::BUILTIN_INSTRUCT
            .iter()
            .find(|(name, _)| *name == "ChatML")
            .and_then(|(_, json)| serde_json::from_str(json).ok())
            .expect("ChatML builtin must exist");
        let mut long_preset = short_preset.clone();
        long_preset.name = "ChatML-padded".to_owned();
        long_preset.stop_sequence = crate::preset::StopSequence::Multiple(vec![
            "<|im_end|>".to_owned(),
            "[unused-padding-string-that-does-not-render]".to_owned(),
        ]);
        let presets = vec![long_preset, short_preset];
        let outcome = pick_best_match(CHATML_JINJA, &presets, "ChatML-padded");
        match outcome {
            MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
                assert_eq!(
                    preset, "ChatML-padded",
                    "current-in-tie-set should win over tiebreak winner"
                );
            }
            other => panic!("expected match, got {other:?}"),
        }
    }

    #[test]
    fn template_hash_is_64_char_hex() {
        let h = template_hash("hello");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn template_hash_is_stable_across_whitespace_drift() {
        let a = "hello   world\n\n\n";
        let b = "  hello world\n\n";
        assert_eq!(template_hash(a), template_hash(b));
    }

    #[test]
    fn template_hash_differs_for_meaningful_changes() {
        assert_ne!(template_hash("hello world"), template_hash("hello earth"));
    }

    #[test]
    fn template_hash_strips_trailing_nul() {
        assert_eq!(template_hash("hello"), template_hash("hello\0"));
    }
}
