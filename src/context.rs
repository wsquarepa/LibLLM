use crate::session::Message;

const DEFAULT_CONTEXT_LIMIT: usize = 4096;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const WARNING_THRESHOLD: f64 = 0.85;

pub struct ContextManager {
    token_limit: usize,
}

impl ContextManager {
    pub fn new(token_limit: usize) -> Self {
        Self { token_limit }
    }

    pub fn estimate_tokens(text: &str) -> usize {
        text.len() / CHARS_PER_TOKEN_ESTIMATE
    }

    pub fn estimate_message_tokens(messages: &[Message]) -> usize {
        messages.iter().map(|m| Self::estimate_tokens(&m.content) + 4).sum()
    }

    pub fn check_and_truncate(&self, messages: &mut Vec<Message>) -> ContextStatus {
        let total = Self::estimate_message_tokens(messages);

        if total > self.token_limit {
            self.truncate_oldest(messages);
            return ContextStatus::Truncated {
                removed_count: total - Self::estimate_message_tokens(messages),
            };
        }

        let ratio = total as f64 / self.token_limit as f64;
        if ratio >= WARNING_THRESHOLD {
            return ContextStatus::Warning {
                used: total,
                limit: self.token_limit,
            };
        }

        ContextStatus::Ok
    }

    fn truncate_oldest(&self, messages: &mut Vec<Message>) {
        while Self::estimate_message_tokens(messages) > self.token_limit && messages.len() > 2 {
            messages.remove(0);
        }
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(DEFAULT_CONTEXT_LIMIT)
    }
}

pub enum ContextStatus {
    Ok,
    Warning { used: usize, limit: usize },
    Truncated { removed_count: usize },
}
