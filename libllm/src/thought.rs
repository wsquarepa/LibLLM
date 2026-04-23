//! Parsing of assistant thinking blocks, driven by the active reasoning preset.
//!
//! The reasoning preset's `prefix` defines the opener (e.g. `<think>\n`) and
//! `suffix` defines the closer (e.g. `\n</think>`). Both are compared against
//! the response after trimming whitespace, which tolerates models that emit
//! the marker without matching the configured whitespace (e.g. `</think>`
//! instead of `\n</think>`). Without an active preset, no thought block can be
//! detected.

use std::time::Instant;

use crate::preset::ReasoningPreset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinkSplit<'a> {
    pub thought: Option<&'a str>,
    pub after: &'a str,
    pub closed: bool,
}

/// A thinking block is only recognized when it sits at the very start of the
/// response — either as a literal `preset.prefix.trim()` prefix, or, when
/// `implicit_open_from_start` is set, as the initial body of the response up
/// to the first `preset.suffix.trim()` occurrence. Any later marker tokens are
/// treated as literal content.
pub fn split_first_think_block<'a>(
    content: &'a str,
    preset: Option<&ReasoningPreset>,
    implicit_open_from_start: bool,
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

    if let Some(after_open) = content.strip_prefix(open) {
        if let Some(close_rel) = after_open.find(close) {
            let after_idx = close_rel + close.len();
            return ThinkSplit {
                thought: Some(&after_open[..close_rel]),
                after: &after_open[after_idx..],
                closed: true,
            };
        }
        return ThinkSplit {
            thought: Some(after_open),
            after: "",
            closed: false,
        };
    }

    if implicit_open_from_start
        && let Some(close_idx) = content.find(close)
    {
        let after_idx = close_idx + close.len();
        return ThinkSplit {
            thought: Some(&content[..close_idx]),
            after: &content[after_idx..],
            closed: true,
        };
    }

    if implicit_open_from_start {
        return ThinkSplit {
            thought: Some(content),
            after: "",
            closed: false,
        };
    }

    ThinkSplit {
        thought: None,
        after: content,
        closed: false,
    }
}

pub fn contains_close_marker(content: &str, preset: Option<&ReasoningPreset>) -> bool {
    let Some(preset) = preset else {
        return false;
    };
    let close = preset.suffix.trim();
    !close.is_empty() && content.contains(close)
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
    implicit_open_from_start: bool,
) -> Option<u32> {
    if !split_first_think_block(content, preset, implicit_open_from_start).closed {
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
        let s = split_first_think_block("<think>a</think>post", None, true);
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "<think>a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_open_and_close_at_start() {
        let p = deepseek();
        let s = split_first_think_block("<think>a</think>post", Some(&p), false);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_tolerates_missing_whitespace_around_tags() {
        let p = deepseek();
        // Model emits `</think>` without the preset's leading `\n`.
        let s = split_first_think_block("<think>a</think>answer", Some(&p), false);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "answer");
        assert!(s.closed);
    }

    #[test]
    fn split_respects_custom_preset_tags() {
        let p = preset("<reasoning>", "</reasoning>");
        let s = split_first_think_block("<reasoning>thoughts</reasoning>answer", Some(&p), false);
        assert_eq!(s.thought, Some("thoughts"));
        assert_eq!(s.after, "answer");
        assert!(s.closed);
    }

    #[test]
    fn split_explicit_open_without_close() {
        let p = deepseek();
        let s = split_first_think_block("<think>a", Some(&p), false);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_open_not_at_start_is_literal() {
        let p = deepseek();
        let s = split_first_think_block("pre<think>a</think>post", Some(&p), false);
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "pre<think>a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_implicit_open_uses_start_when_close_present() {
        let p = deepseek();
        let s = split_first_think_block("a</think>post", Some(&p), true);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_implicit_open_without_close_is_unclosed_thought() {
        let p = deepseek();
        let s = split_first_think_block("still thinking", Some(&p), true);
        assert_eq!(s.thought, Some("still thinking"));
        assert_eq!(s.after, "");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_at_start_wins_over_implicit() {
        let p = deepseek();
        let s = split_first_think_block("<think>a</think>post", Some(&p), true);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_preserves_later_literal_tags_in_after() {
        let p = deepseek();
        let s = split_first_think_block(
            "<think>a</think>tail with <think>second</think> literal",
            Some(&p),
            false,
        );
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "tail with <think>second</think> literal");
        assert!(s.closed);
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
            resolve_thought_seconds("hello", Some(3), Some(2), Some(&p), false),
            None
        );
        assert_eq!(
            resolve_thought_seconds("<think>hello", Some(3), Some(2), Some(&p), false),
            None
        );
    }

    #[test]
    fn resolve_thought_seconds_prefers_existing_when_closed() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("hello</think>world", Some(9), Some(2), Some(&p), true),
            Some(9)
        );
    }

    #[test]
    fn resolve_thought_seconds_uses_measured_when_no_existing() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("hello</think>world", None, Some(2), Some(&p), true),
            Some(2)
        );
    }

    #[test]
    fn resolve_thought_seconds_allows_moment_fallback() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("hello</think>world", None, None, Some(&p), true),
            None
        );
    }

    #[test]
    fn resolve_thought_seconds_discards_continuation_timing_for_implicit_open() {
        let p = deepseek();
        // Original message had an unclosed implicit-open thought. Continuation
        // runs with implicit_open=false, so the combined content is not
        // recognized as a thought block and the misleading measurement is
        // correctly discarded.
        assert_eq!(
            resolve_thought_seconds(
                "early thoughts</think>answer",
                None,
                Some(3),
                Some(&p),
                false
            ),
            None
        );
    }

    #[test]
    fn resolve_thought_seconds_preserves_existing_across_edit_when_implicit() {
        let p = deepseek();
        assert_eq!(
            resolve_thought_seconds("musing</think>fixed answer", Some(11), None, Some(&p), true),
            Some(11)
        );
    }

    #[test]
    fn contains_close_marker_uses_preset_suffix() {
        let p = deepseek();
        assert!(contains_close_marker("hello</think>there", Some(&p)));
        assert!(!contains_close_marker("hello</done>there", Some(&p)));
        assert!(!contains_close_marker("hello</think>there", None));
    }
}
