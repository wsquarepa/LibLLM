use crate::session::Message;

const DEFAULT_CONTEXT_LIMIT: usize = 4096;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const WARNING_THRESHOLD: f64 = 0.85;

pub struct ContextManager {
    token_limit: usize,
}

impl ContextManager {
    pub fn estimate_message_tokens(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| m.content.len() / CHARS_PER_TOKEN_ESTIMATE + 4)
            .sum()
    }

    pub fn check(&self, messages: &[Message]) -> ContextStatus {
        let total = Self::estimate_message_tokens(messages);
        let ratio = total as f64 / self.token_limit as f64;

        if total > self.token_limit {
            ContextStatus::OverLimit
        } else if ratio >= WARNING_THRESHOLD {
            ContextStatus::Warning {
                used: total,
                limit: self.token_limit,
            }
        } else {
            ContextStatus::Ok
        }
    }

    pub fn truncate(&self, messages: &mut Vec<Message>) -> usize {
        let before = Self::estimate_message_tokens(messages);
        if before <= self.token_limit || messages.len() <= 2 {
            return 0;
        }

        let mut cumulative = 0usize;
        let mut remove_count = 0usize;
        for msg in messages.iter() {
            let tokens = msg.content.len() / CHARS_PER_TOKEN_ESTIMATE + 4;
            if before - cumulative - tokens <= self.token_limit {
                break;
            }
            cumulative += tokens;
            remove_count += 1;
            if messages.len() - remove_count <= 2 {
                break;
            }
        }

        if remove_count > 0 {
            messages.drain(..remove_count);
        }
        cumulative
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self {
            token_limit: DEFAULT_CONTEXT_LIMIT,
        }
    }
}

pub enum ContextStatus {
    Ok,
    Warning { used: usize, limit: usize },
    OverLimit,
}
