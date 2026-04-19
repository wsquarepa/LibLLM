//! Context window budget and summary-boundary routing for message paths.

use crate::session::{Message, Role};

/// Holds the token-budget limit for truncation decisions. Token counting itself
/// lives in the `tokenizer` module; this type owns the limit and the summary
/// boundary, not the math.
pub struct ContextManager {
    token_limit: usize,
}

impl ContextManager {
    pub fn new(token_limit: usize) -> Self {
        Self { token_limit }
    }

    pub fn token_limit(&self) -> usize {
        self.token_limit
    }

    pub fn set_token_limit(&mut self, limit: usize) {
        let previous = self.token_limit;
        self.token_limit = limit;
        tracing::info!(
            phase = "set",
            previous = previous,
            token_limit = limit,
            "context.limit",
        );
    }

    /// Returns a vec starting from the last `Role::Summary` node (inclusive).
    /// If no summary exists, returns the full slice unchanged.
    pub fn summary_aware_path<'a>(&self, messages: &'a [&'a Message]) -> Vec<&'a Message> {
        let last_summary_idx = messages.iter().rposition(|m| m.role == Role::Summary);
        let result = match last_summary_idx {
            Some(idx) => messages[idx..].to_vec(),
            None => messages.to_vec(),
        };
        tracing::info!(
            summary_found = last_summary_idx.is_some(),
            input_message_count = messages.len(),
            kept_after_boundary = result.len(),
            "context.summary_boundary",
        );
        result
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self { token_limit: 8192 }
    }
}

/// Returns a copy of `messages` with the `k` oldest non-`Summary` messages removed.
/// `Role::Summary` rows are never droppable.
///
/// Used by `streaming::start` to build the candidate prompt for the k-th shrink step,
/// and by `Summarizer::shed_to_fit` to trim the summarize-prompt input.
pub fn drop_oldest_non_summary<'a>(messages: &[&'a Message], k: usize) -> Vec<&'a Message> {
    if k == 0 {
        return messages.to_vec();
    }
    let mut dropped = 0usize;
    messages
        .iter()
        .filter(|m| {
            if m.role == Role::Summary || dropped >= k {
                true
            } else {
                dropped += 1;
                false
            }
        })
        .copied()
        .collect()
}

/// Counts droppable (non-Summary) messages in a path. Needed by the binary-search
/// upper bound in `streaming::start`.
pub fn droppable_count(messages: &[&Message]) -> usize {
    messages.iter().filter(|m| m.role != Role::Summary).count()
}

/// Returns the exclusive upper bound `i` in `messages` such that exactly `k` non-Summary
/// messages appear in `messages[..i]`. Returns `messages.len()` when fewer than `k`
/// non-Summary messages exist. Used by the summarization trigger to locate the boundary
/// between dropped and kept messages without re-deriving it from `drop_oldest_non_summary`.
pub fn drop_split_index(messages: &[&Message], k: usize) -> usize {
    if k == 0 {
        return 0;
    }
    let mut seen = 0usize;
    for (i, m) in messages.iter().enumerate() {
        if m.role != Role::Summary {
            seen += 1;
            if seen == k {
                return i + 1;
            }
        }
    }
    messages.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> Message {
        Message::new(Role::User, content.to_owned())
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new(Role::Assistant, content.to_owned())
    }

    #[test]
    fn drop_oldest_non_summary_drops_k_from_front() {
        let a = user_msg("one");
        let b = assistant_msg("two");
        let c = user_msg("three");
        let refs: Vec<&_> = vec![&a, &b, &c];

        let result = drop_oldest_non_summary(&refs, 1);
        assert_eq!(result.len(), 2);
        assert!(std::ptr::eq(result[0], &b));
        assert!(std::ptr::eq(result[1], &c));
    }

    #[test]
    fn drop_oldest_non_summary_preserves_summary_rows() {
        let summary = Message::new(Role::Summary, "prior summary".to_owned());
        let a = user_msg("one");
        let b = assistant_msg("two");
        let refs: Vec<&_> = vec![&summary, &a, &b];

        let result = drop_oldest_non_summary(&refs, 2);
        assert_eq!(result.len(), 1);
        assert!(std::ptr::eq(result[0], &summary));
    }

    #[test]
    fn drop_oldest_non_summary_saturates_at_message_count() {
        let a = user_msg("one");
        let refs: Vec<&_> = vec![&a];
        let result = drop_oldest_non_summary(&refs, 5);
        assert!(result.is_empty());
    }

    #[test]
    fn summary_aware_path_starts_from_last_summary() {
        let a = user_msg("pre1");
        let b = user_msg("pre2");
        let summary = Message::new(Role::Summary, "mid".to_owned());
        let c = user_msg("post");
        let refs: Vec<&_> = vec![&a, &b, &summary, &c];
        let mgr = ContextManager::new(1024);
        let result = mgr.summary_aware_path(&refs);
        assert_eq!(result.len(), 2);
        assert!(std::ptr::eq(result[0], &summary));
    }

    #[test]
    fn droppable_count_skips_summary() {
        let a = user_msg("pre1");
        let summary = Message::new(Role::Summary, "mid".to_owned());
        let c = user_msg("post");
        let refs: Vec<&_> = vec![&a, &summary, &c];
        assert_eq!(droppable_count(&refs), 2);
    }

    #[test]
    fn drop_split_index_counts_non_summary_only() {
        let a = user_msg("m1");
        let b = assistant_msg("m2");
        let summary = Message::new(Role::Summary, "s".to_owned());
        let c = user_msg("m3");
        let d = assistant_msg("m4");
        let refs: Vec<&_> = vec![&a, &b, &summary, &c, &d];

        assert_eq!(drop_split_index(&refs, 0), 0);
        assert_eq!(drop_split_index(&refs, 1), 1);
        assert_eq!(drop_split_index(&refs, 2), 2);
        assert_eq!(drop_split_index(&refs, 3), 4);
        assert_eq!(drop_split_index(&refs, 4), 5);
        assert_eq!(drop_split_index(&refs, 99), 5);
    }

    #[test]
    fn drop_split_index_with_leading_summary() {
        let summary = Message::new(Role::Summary, "prior".to_owned());
        let a = user_msg("m1");
        let b = assistant_msg("m2");
        let refs: Vec<&_> = vec![&summary, &a, &b];

        assert_eq!(drop_split_index(&refs, 1), 2);
        assert_eq!(drop_split_index(&refs, 2), 3);
    }
}
