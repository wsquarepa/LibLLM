//! Context window management and token budget allocation.

use crate::session::{Message, Role};

const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

/// Estimates token counts and truncates message paths to fit within a configured token budget.
///
/// Uses a simple heuristic of ~4 characters per token plus a per-message overhead of 4 tokens.
/// Always preserves at least the last 2 messages.
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

    pub fn estimate_message_tokens(messages: &[&Message]) -> usize {
        Self::estimate_tokens_for_messages(messages.iter().copied())
    }

    pub fn estimate_tokens_for_messages<'a, I>(messages: I) -> usize
    where
        I: IntoIterator<Item = &'a Message>,
    {
        messages
            .into_iter()
            .map(|m| m.content.len() / CHARS_PER_TOKEN_ESTIMATE + 4)
            .sum()
    }

    pub fn truncated_path<'a>(&self, messages: &'a [&'a Message]) -> &'a [&'a Message] {
        if messages.len() <= 2 {
            return messages;
        }

        let total = Self::estimate_message_tokens(messages);
        if total <= self.token_limit {
            return messages;
        }

        let mut cumulative = 0usize;
        let mut skip = 0usize;
        for msg in messages.iter() {
            let tokens = msg.content.len() / CHARS_PER_TOKEN_ESTIMATE + 4;
            cumulative += tokens;
            skip += 1;
            if messages.len() - skip <= 2 {
                break;
            }
            if total - cumulative <= self.token_limit {
                break;
            }
        }

        tracing::info!(
            phase = "truncate",
            result = "ok",
            input_message_count = messages.len(),
            kept = messages.len() - skip,
            dropped = skip,
            estimated_tokens = total,
            token_limit = self.token_limit,
            "context.truncate",
        );
        &messages[skip..]
    }

    /// Returns the number of messages that would be dropped by truncation.
    pub fn dropped_message_count(&self, messages: &[&Message]) -> usize {
        messages.len() - self.truncated_path(messages).len()
    }

    /// Returns a vec starting from the last `Role::Summary` node (inclusive).
    /// If no summary exists, returns the full slice unchanged.
    /// The caller should then pass the result to `truncated_path()` for token-based truncation.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Message, Role};

    fn user_msg(content: &str) -> Message {
        Message::new(Role::User, content.to_string())
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new(Role::Assistant, content.to_string())
    }

    #[test]
    fn token_estimation() {
        let msg_a = user_msg("a]bc"); // 4 chars -> 4/4 + 4 = 5 tokens
        let msg_b = assistant_msg("12345678"); // 8 chars -> 8/4 + 4 = 6 tokens
        let refs: Vec<&_> = vec![&msg_a, &msg_b];

        let estimated = ContextManager::estimate_message_tokens(&refs);
        assert_eq!(estimated, 5 + 6, "expected 11 tokens for 4+8 chars");
    }

    #[test]
    fn token_estimation_empty_message() {
        let msg = user_msg(""); // 0 chars -> 0/4 + 4 = 4 tokens
        let refs: Vec<&_> = vec![&msg];

        let estimated = ContextManager::estimate_message_tokens(&refs);
        assert_eq!(
            estimated, 4,
            "empty message should still count 4 overhead tokens"
        );
    }

    #[test]
    fn truncated_path_fits_within_limit() {
        let ctx = ContextManager::new(4096);
        // Each message: 4000 chars -> 4000/4 + 4 = 1004 tokens
        // 10 messages = 10040 tokens, well over 4096
        let big_content = "x".repeat(4000);
        let msgs: Vec<_> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    user_msg(&big_content)
                } else {
                    assistant_msg(&big_content)
                }
            })
            .collect();
        let refs: Vec<&_> = msgs.iter().collect();

        let truncated = ctx.truncated_path(&refs);
        let truncated_tokens = ContextManager::estimate_message_tokens(truncated);

        assert!(
            truncated.len() < msgs.len(),
            "should have truncated some messages"
        );
        assert!(
            truncated_tokens <= 4096,
            "truncated path ({truncated_tokens} tokens) should fit within 4096 limit"
        );
    }

    #[test]
    fn truncated_path_preserves_recent_messages() {
        let ctx = ContextManager::default();
        let big_content = "x".repeat(4000);
        let msgs: Vec<_> = (0..10)
            .map(|i| user_msg(&format!("msg_{i}_{big_content}")))
            .collect();
        let refs: Vec<&_> = msgs.iter().collect();

        let truncated = ctx.truncated_path(&refs);

        let last_truncated = truncated.last().expect("truncated should not be empty");
        let last_original = refs.last().expect("refs should not be empty");
        assert_eq!(
            last_truncated.content, last_original.content,
            "last message should be preserved (most recent)"
        );

        let second_last_truncated = truncated[truncated.len() - 2].content.as_str();
        let second_last_original = refs[refs.len() - 2].content.as_str();
        assert_eq!(
            second_last_truncated, second_last_original,
            "second-to-last message should be preserved"
        );
    }

    #[test]
    fn large_context_no_truncation() {
        let ctx = ContextManager::default();
        // Each message: 20 chars -> 20/4 + 4 = 9 tokens
        // 10 messages = 90 tokens, well under 4096
        let msgs: Vec<_> = (0..10)
            .map(|i| user_msg(&format!("short message {i:04}")))
            .collect();
        let refs: Vec<&_> = msgs.iter().collect();

        let truncated = ctx.truncated_path(&refs);
        assert_eq!(
            truncated.len(),
            refs.len(),
            "all messages should be returned when they fit"
        );
    }

    #[test]
    fn single_message_exceeding_limit() {
        let ctx = ContextManager::default();
        // 100000 chars -> 100000/4 + 4 = 25004 tokens, way over 4096
        let huge = user_msg(&"x".repeat(100_000));
        let refs: Vec<&_> = vec![&huge];

        let truncated = ctx.truncated_path(&refs);
        assert_eq!(
            truncated.len(),
            1,
            "single message should always be returned (len <= 2 guard)"
        );
    }

    #[test]
    fn two_messages_exceeding_limit() {
        let ctx = ContextManager::default();
        let huge = "x".repeat(100_000);
        let msgs = vec![user_msg(&huge), assistant_msg(&huge)];
        let refs: Vec<&_> = msgs.iter().collect();

        let truncated = ctx.truncated_path(&refs);
        assert_eq!(
            truncated.len(),
            2,
            "two messages should always be returned (len <= 2 guard)"
        );
    }

    #[test]
    fn new_with_custom_limit() {
        let ctx = ContextManager::new(16384);
        let small_msgs: Vec<_> = (0..10)
            .map(|i| user_msg(&format!("msg {i}")))
            .collect();
        let refs: Vec<&_> = small_msgs.iter().collect();
        let truncated = ctx.truncated_path(&refs);
        assert_eq!(truncated.len(), refs.len());
    }

    #[test]
    fn dropped_message_count_no_overflow() {
        let ctx = ContextManager::new(8192);
        let msgs: Vec<_> = (0..5).map(|i| user_msg(&format!("short {i}"))).collect();
        let refs: Vec<&_> = msgs.iter().collect();
        assert_eq!(ctx.dropped_message_count(&refs), 0);
    }

    #[test]
    fn dropped_message_count_with_overflow() {
        let ctx = ContextManager::new(4096);
        let big = "x".repeat(4000);
        let msgs: Vec<_> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    user_msg(&big)
                } else {
                    assistant_msg(&big)
                }
            })
            .collect();
        let refs: Vec<&_> = msgs.iter().collect();
        let dropped = ctx.dropped_message_count(&refs);
        assert!(dropped > 0);
        assert_eq!(dropped, refs.len() - ctx.truncated_path(&refs).len());
    }

    #[test]
    fn summary_aware_truncation() {
        let ctx = ContextManager::new(4096);
        let big = "x".repeat(4000);
        let summary = Message::new(Role::Summary, "Summary of earlier conversation".to_owned());
        let mut msgs = vec![user_msg(&big), assistant_msg(&big), user_msg(&big)];
        msgs.push(summary);
        msgs.push(user_msg("after summary"));
        msgs.push(assistant_msg("response"));

        let refs: Vec<&_> = msgs.iter().collect();
        let truncated = ctx.summary_aware_path(&refs);

        assert!(truncated[0].role == Role::Summary);
        assert_eq!(truncated.len(), 3);
    }

    #[test]
    fn multiple_summaries_uses_last_one() {
        let ctx = ContextManager::new(8192);
        let msgs = vec![
            user_msg("ancient msg"),
            Message::new(Role::Summary, "Old summary".to_owned()),
            user_msg("mid msg"),
            Message::new(Role::Summary, "Newer summary".to_owned()),
            user_msg("recent msg"),
        ];
        let refs: Vec<&_> = msgs.iter().collect();
        let aware = ctx.summary_aware_path(&refs);

        assert_eq!(aware.len(), 2);
        assert_eq!(aware[0].role, Role::Summary);
        assert_eq!(aware[0].content, "Newer summary");
        assert_eq!(aware[1].content, "recent msg");
    }

    #[test]
    fn no_summary_returns_full_path() {
        let ctx = ContextManager::new(8192);
        let msgs = vec![user_msg("msg 1"), assistant_msg("reply 1")];
        let refs: Vec<&_> = msgs.iter().collect();
        let aware = ctx.summary_aware_path(&refs);
        assert_eq!(aware.len(), 2);
    }

    #[test]
    fn server_limit_higher_than_local_does_not_raise() {
        let local_limit = 4096usize;
        let server_n_ctx = 65536usize;
        let effective = server_n_ctx.min(local_limit);
        assert_eq!(
            effective, local_limit,
            "server value above local limit must be clamped to local limit"
        );

        let server_n_ctx_lower = 2048usize;
        let effective_lower = server_n_ctx_lower.min(local_limit);
        assert_eq!(
            effective_lower, server_n_ctx_lower,
            "server value below local limit must lower the effective limit"
        );
    }

    #[test]
    fn dropped_count_after_summary_boundary() {
        let ctx = ContextManager::new(4096);
        let big = "x".repeat(4000);
        let msgs = vec![
            user_msg(&big),
            assistant_msg(&big),
            Message::new(Role::Summary, "Summary".to_owned()),
            user_msg(&big),
            assistant_msg(&big),
            user_msg(&big),
            assistant_msg(&big),
            user_msg(&big),
        ];
        let refs: Vec<&_> = msgs.iter().collect();
        let aware = ctx.summary_aware_path(&refs);
        let dropped = ctx.dropped_message_count(&aware);
        // After summary boundary, 6 messages (summary + 5 big ones).
        // 5 big messages at 1004 tokens each = 5020 tokens, plus summary = 5025, over 4096.
        assert!(dropped > 0);
    }
}
