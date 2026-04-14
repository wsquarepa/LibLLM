//! Background conversation summarization engine.

use crate::client::ApiClient;
use crate::sampling::SamplingParams;
use crate::session::{Message, Role};
use anyhow::Result;

/// Formats messages into a summarization prompt and calls the LLM for a non-streaming summary.
pub struct Summarizer {
    client: ApiClient,
    prompt_instruction: String,
}

impl Summarizer {
    pub fn new(client: ApiClient, prompt_instruction: String) -> Self {
        Self {
            client,
            prompt_instruction,
        }
    }

    /// Formats messages into a summarization prompt.
    /// Excludes any `Role::Summary` messages from the input (no recursive summarization).
    pub fn format_prompt(instruction: &str, messages: &[&Message]) -> String {
        let mut prompt = String::from(instruction);
        prompt.push_str("\n\n");
        for msg in messages {
            if msg.role == Role::Summary {
                continue;
            }
            let label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Summary => continue,
            };
            prompt.push_str(label);
            prompt.push_str(": ");
            prompt.push_str(&msg.content);
            prompt.push('\n');
        }
        prompt
    }

    /// Sheds the oldest messages until the formatted prompt fits within the token budget.
    /// Uses the same 4-chars-per-token heuristic as `ContextManager`.
    /// Always keeps at least the last message.
    pub fn shed_to_fit<'a>(messages: &[&'a Message], token_budget: usize) -> Vec<&'a Message> {
        let estimate_tokens = |msgs: &[&Message]| -> usize {
            msgs.iter()
                .map(|m| m.content.len() / 4 + 4)
                .sum::<usize>()
                + 50
        };

        let mut start = 0;
        while start < messages.len().saturating_sub(1)
            && estimate_tokens(&messages[start..]) > token_budget
        {
            start += 1;
        }
        messages[start..].to_vec()
    }

    /// Summarizes the given messages by calling the LLM.
    /// Sheds oldest messages if they exceed the token budget.
    pub async fn summarize(
        &self,
        messages: &[&Message],
        token_budget: usize,
    ) -> Result<String> {
        let trimmed = Self::shed_to_fit(messages, token_budget);
        let prompt = Self::format_prompt(&self.prompt_instruction, &trimmed);

        let sampling = SamplingParams {
            temperature: 0.3,
            max_tokens: 1024,
            ..SamplingParams::default()
        };

        self.client
            .complete(&prompt, &[], &sampling)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_prompt_basic() {
        let msgs = vec![
            Message::new(Role::User, "Hello".to_owned()),
            Message::new(Role::Assistant, "Hi there!".to_owned()),
            Message::new(Role::User, "How are you?".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt("Summarize this.", &refs);
        assert!(prompt.contains("Summarize this."));
        assert!(prompt.contains("User: Hello"));
        assert!(prompt.contains("Assistant: Hi there!"));
        assert!(prompt.contains("User: How are you?"));
    }

    #[test]
    fn format_prompt_excludes_summary_role() {
        let msgs = vec![
            Message::new(Role::Summary, "Old summary".to_owned()),
            Message::new(Role::User, "New message".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt("Summarize.", &refs);
        assert!(!prompt.contains("Old summary"));
        assert!(prompt.contains("User: New message"));
    }

    #[test]
    fn format_prompt_handles_system_messages() {
        let msgs = vec![
            Message::new(Role::System, "You are helpful.".to_owned()),
            Message::new(Role::User, "Hi".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt("Summarize.", &refs);
        assert!(prompt.contains("System: You are helpful."));
    }

    #[test]
    fn shed_oldest_messages_when_over_budget() {
        let big = "x".repeat(40000);
        let msgs: Vec<_> = (0..20)
            .map(|_| Message::new(Role::User, big.clone()))
            .collect();
        let refs: Vec<&Message> = msgs.iter().collect();
        let trimmed = Summarizer::shed_to_fit(&refs, 8192);
        assert!(trimmed.len() < refs.len());
        assert_eq!(trimmed.last().unwrap().content, refs.last().unwrap().content);
    }
}
