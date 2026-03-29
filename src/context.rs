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
            if total - cumulative - tokens <= self.token_limit {
                break;
            }
            cumulative += tokens;
            skip += 1;
            if messages.len() - skip <= 2 {
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
