use crate::session::Message;

const DEFAULT_CONTEXT_LIMIT: usize = 4096;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

pub struct ContextManager {
    token_limit: usize,
}

impl ContextManager {
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

        &messages[skip..]
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self {
            token_limit: DEFAULT_CONTEXT_LIMIT,
        }
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
        let ctx = ContextManager::default(); // 4096 token limit
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
}
