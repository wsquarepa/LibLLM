//! Parsing of assistant thinking blocks, driven by the active reasoning preset.
//!
//! The reasoning preset's `prefix` defines the opener (e.g. `<think>\n`) and
//! `suffix` defines the closer (e.g. `\n</think>`). Both are compared against
//! the response after trimming whitespace, which tolerates models that emit
//! the marker without matching the configured whitespace (e.g. `</think>`
//! instead of `\n</think>`). Without an active preset, no thought block can be
//! detected.
//!
//! Stored assistant messages always carry the opener at the start: the model's
//! raw response is normalized via [`normalize_assistant_content`] before it
//! lands in the tree. Messages that lack an explicit opener at position 0 are
//! treated as plain content.

use std::time::Instant;

use crate::preset::ReasoningPreset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinkSplit<'a> {
    pub thought: Option<&'a str>,
    pub after: &'a str,
    pub closed: bool,
}

/// Recognize a thinking block only when the configured opener sits at position
/// 0 of `content`. Any later occurrences of the opener or closer are treated
/// as literal content.
pub fn split_first_think_block<'a>(
    content: &'a str,
    preset: Option<&ReasoningPreset>,
) -> ThinkSplit<'a> {
    let Some(preset) = preset else {
        return ThinkSplit {
            thought: None,
            after: content,
            closed: false,
        };
    };
    let open = preset.prefix.trim();
    let close = preset.suffix.trim();
    if open.is_empty() || close.is_empty() {
        return ThinkSplit {
            thought: None,
            after: content,
            closed: false,
        };
    }
    let Some(after_open) = content.strip_prefix(open) else {
        return ThinkSplit {
            thought: None,
            after: content,
            closed: false,
        };
    };
    if let Some(close_rel) = after_open.find(close) {
        let after_idx = close_rel + close.len();
        return ThinkSplit {
            thought: Some(&after_open[..close_rel]),
            after: &after_open[after_idx..],
            closed: true,
        };
    }
    ThinkSplit {
        thought: Some(after_open),
        after: "",
        closed: false,
    }
}

/// Reasoning presets attach their prefix to the *prompt* via `apply_prefix`, so
/// the model's streamed response does not echo the opener back. Persist the
/// response with the opener restored whenever the model genuinely engaged with
/// thinking mode (the close marker is present), producing fully-tagged content
/// that round-trips through storage, edit, and export. Noop when the preset is
/// absent, the opener is already present, or no close marker exists.
pub fn normalize_assistant_content<'a>(
    content: &'a str,
    preset: Option<&ReasoningPreset>,
) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    let Some(preset) = preset else {
        return Cow::Borrowed(content);
    };
    let open = preset.prefix.trim();
    let close = preset.suffix.trim();
    if open.is_empty() || close.is_empty() {
        return Cow::Borrowed(content);
    }
    if content.starts_with(open) || !content.contains(close) {
        return Cow::Borrowed(content);
    }
    Cow::Owned(format!("{}{}", preset.prefix, content))
}

pub fn measured_thought_seconds(
    started_at: Option<Instant>,
    closed_at: Option<Instant>,
) -> Option<u32> {
    let (Some(started_at), Some(closed_at)) = (started_at, closed_at) else {
        return None;
    };
    let elapsed = closed_at.saturating_duration_since(started_at).as_secs();
    if elapsed == 0 {
        return None;
    }
    Some(elapsed.try_into().unwrap_or(u32::MAX))
}

pub fn resolve_thought_seconds(
    content: &str,
    existing: Option<u32>,
    measured: Option<u32>,
    preset: Option<&ReasoningPreset>,
) -> Option<u32> {
    if !split_first_think_block(content, preset).closed {
        return None;
    }
    existing.or(measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(prefix: &str, suffix: &str) -> ReasoningPreset {
        ReasoningPreset {
            name: "test".to_owned(),
            prefix: prefix.to_owned(),
            suffix: suffix.to_owned(),
            separator: String::new(),
        }
    }

    fn deepseek() -> ReasoningPreset {
        preset("<think>\n", "\n</think>")
    }

    #[test]
    fn split_without_preset_returns_no_thought() {
        let s = split_first_think_block("<think>a</think>post", None);
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "<think>a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_open_and_close_at_start() {
        let p = deepseek();
        let s = split_first_think_block("<think>a</think>post", Some(&p));
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_tolerates_missing_whitespace_around_tags() {
        let p = deepseek();
        let s = split_first_think_block("<think>a</think>answer", Some(&p));
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "answer");
        assert!(s.closed);
    }

    #[test]
    fn split_respects_custom_preset_tags() {
        let p = preset("<reasoning>", "</reasoning>");
        let s = split_first_think_block("<reasoning>thoughts</reasoning>answer", Some(&p));
        assert_eq!(s.thought, Some("thoughts"));
        assert_eq!(s.after, "answer");
        assert!(s.closed);
    }

    #[test]
    fn split_explicit_open_without_close() {
        let p = deepseek();
        let s = split_first_think_block("<think>a", Some(&p));
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_open_not_at_start_is_literal() {
        let p = deepseek();
        let s = split_first_think_block("pre<think>a</think>post", Some(&p));
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "pre<think>a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_content_without_opener_is_plain() {
        let p = deepseek();
        let s = split_first_think_block("a</think>post", Some(&p));
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_preserves_later_literal_tags_in_after() {
        let p = deepseek();
        let s = split_first_think_block(
            "<think>a</think>tail with <think>second</think> literal",
            Some(&p),
        );
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "tail with <think>second</think> literal");
        assert!(s.closed);
    }

    #[test]
    fn normalize_adds_opener_when_close_marker_present() {
        let p = deepseek();
        let out = normalize_assistant_content("thinking body</think>answer", Some(&p));
        assert_eq!(out, "<think>\nthinking body</think>answer");
    }

    #[test]
    fn normalize_is_noop_when_opener_present() {
        let p = deepseek();
        let out = normalize_assistant_content("<think>body</think>tail", Some(&p));
        assert_eq!(out, "<think>body</think>tail");
    }

    #[test]
    fn normalize_is_noop_without_close_marker() {
        let p = deepseek();
        let out = normalize_assistant_content("plain answer", Some(&p));
        assert_eq!(out, "plain answer");
    }

    #[test]
    fn normalize_is_noop_without_preset() {
        let out = normalize_assistant_content("body</think>answer", None);
        assert_eq!(out, "body</think>answer");
    }

    #[test]
    fn measured_thought_seconds_sub_second_returns_none() {
        let started = Instant::now();
        assert_eq!(measured_thought_seconds(Some(started), Some(started)), None);
    }

    #[test]
    fn resolve_thought_seconds_returns_none_without_closed_block() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("hello", Some(3), Some(2), Some(&p)),
            None
        );
        assert_eq!(
            resolve_thought_seconds("<think>hello", Some(3), Some(2), Some(&p)),
            None
        );
    }

    #[test]
    fn resolve_thought_seconds_prefers_existing_when_closed() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("<think>body</think>world", Some(9), Some(2), Some(&p)),
            Some(9)
        );
    }

    #[test]
    fn resolve_thought_seconds_uses_measured_when_no_existing() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("<think>body</think>world", None, Some(2), Some(&p)),
            Some(2)
        );
    }

    #[test]
    fn resolve_thought_seconds_allows_moment_fallback() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("<think>body</think>world", None, None, Some(&p)),
            None
        );
    }
}
