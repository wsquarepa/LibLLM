use std::time::Instant;

const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThinkSplit<'a> {
    pub(crate) thought: Option<&'a str>,
    pub(crate) after: &'a str,
    pub(crate) closed: bool,
}

/// A thinking block is only recognized when it sits at the very start of the
/// response -- either as a literal `<think>` prefix or, when
/// `implicit_open_from_start` is set, as the initial body of the response up to
/// the first `</think>`. Any later `<think>` / `</think>` tokens are treated as
/// literal content.
pub(crate) fn split_first_think_block(content: &str, implicit_open_from_start: bool) -> ThinkSplit<'_> {
    if let Some(after_open) = content.strip_prefix(THINK_OPEN_TAG) {
        if let Some(close_rel) = after_open.find(THINK_CLOSE_TAG) {
            let after_idx = close_rel + THINK_CLOSE_TAG.len();
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

    if implicit_open_from_start {
        if let Some(close_idx) = content.find(THINK_CLOSE_TAG) {
            let after_idx = close_idx + THINK_CLOSE_TAG.len();
            return ThinkSplit {
                thought: Some(&content[..close_idx]),
                after: &content[after_idx..],
                closed: true,
            };
        }
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

pub(crate) fn measured_thought_seconds(
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

pub(crate) fn resolve_thought_seconds(
    content: &str,
    existing: Option<u32>,
    measured: Option<u32>,
    implicit_open_from_start: bool,
) -> Option<u32> {
    if !split_first_think_block(content, implicit_open_from_start).closed {
        return None;
    }
    existing.or(measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_explicit_open_and_close_at_start() {
        let s = split_first_think_block("<think>a</think>post", false);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_explicit_open_without_close() {
        let s = split_first_think_block("<think>a", false);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_open_not_at_start_is_literal() {
        let s = split_first_think_block("pre<think>a</think>post", false);
        assert_eq!(s.thought, None);
        assert_eq!(s.after, "pre<think>a</think>post");
        assert!(!s.closed);
    }

    #[test]
    fn split_implicit_open_uses_start_when_close_present() {
        let s = split_first_think_block("a</think>post", true);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_implicit_open_without_close_is_unclosed_thought() {
        let s = split_first_think_block("still thinking", true);
        assert_eq!(s.thought, Some("still thinking"));
        assert_eq!(s.after, "");
        assert!(!s.closed);
    }

    #[test]
    fn split_explicit_at_start_wins_over_implicit() {
        let s = split_first_think_block("<think>a</think>post", true);
        assert_eq!(s.thought, Some("a"));
        assert_eq!(s.after, "post");
        assert!(s.closed);
    }

    #[test]
    fn split_preserves_later_literal_tags_in_after() {
        let s = split_first_think_block("<think>a</think>tail with <think>second</think> literal", false);
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
        assert_eq!(resolve_thought_seconds("hello", Some(3), Some(2), false), None);
        assert_eq!(resolve_thought_seconds("<think>hello", Some(3), Some(2), false), None);
    }

    #[test]
    fn resolve_thought_seconds_prefers_existing_when_closed() {
        assert_eq!(
            resolve_thought_seconds("hello</think>world", Some(9), Some(2), true),
            Some(9)
        );
    }

    #[test]
    fn resolve_thought_seconds_uses_measured_when_no_existing() {
        assert_eq!(
            resolve_thought_seconds("hello</think>world", None, Some(2), true),
            Some(2)
        );
    }

    #[test]
    fn resolve_thought_seconds_allows_moment_fallback() {
        assert_eq!(
            resolve_thought_seconds("hello</think>world", None, None, true),
            None
        );
    }

    /// Continuation path: original message had an unclosed implicit-open thought
    /// (no literal `<think>` marker). The continuation stream starts its clock
    /// at its own first token, so `measured` only reflects the continuation
    /// fragment, not the full thought. The continuation runs with
    /// `implicit_open_from_start=false`, so this combined content is not
    /// recognized as a thought block and the misleading measurement is
    /// correctly discarded.
    #[test]
    fn resolve_thought_seconds_discards_continuation_timing_for_implicit_open() {
        assert_eq!(
            resolve_thought_seconds("early thoughts</think>answer", None, Some(3), false),
            None
        );
    }

    #[test]
    fn resolve_thought_seconds_preserves_existing_across_edit_when_implicit() {
        assert_eq!(
            resolve_thought_seconds("musing</think>fixed answer", Some(11), None, true),
            Some(11)
        );
    }
}
