//! Background conversation summarization engine.

use std::time::Instant;

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

    /// Shrinks `messages` until `format_prompt(instruction, trimmed)` is ≤ `token_budget`
    /// according to `counter`. Binary-searches the smallest `k` such that dropping the `k`
    /// oldest non-Summary messages puts the rendered prompt under budget. Always keeps at
    /// least one message if the input is non-empty.
    pub async fn shed_to_fit<'a>(
        instruction: &str,
        messages: &[&'a Message],
        token_budget: usize,
        counter: &crate::tokenizer::TokenCounter,
    ) -> Result<Vec<&'a Message>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let droppable = crate::context::droppable_count(messages);
        let max_drop = droppable.saturating_sub(1);

        let render_at = |k: usize| -> String {
            let subset = crate::context::drop_oldest_non_summary(messages, k);
            let refs: Vec<&Message> = subset.iter().copied().collect();
            Self::format_prompt(instruction, &refs)
        };

        // Fast path: no shedding needed.
        let full = render_at(0);
        let full_count = counter.count_authoritative(&full).await?;
        if full_count <= token_budget {
            return Ok(messages.to_vec());
        }

        // Binary-search the smallest k in [1, max_drop] with rendered count ≤ budget.
        let (mut lo, mut hi) = (1usize, max_drop);
        let mut best = max_drop;
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let rendered = render_at(mid);
            let count = counter.count_authoritative(&rendered).await?;
            if count <= token_budget {
                best = mid;
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }

        Ok(crate::context::drop_oldest_non_summary(messages, best))
    }

    pub async fn summarize(
        &self,
        messages: &[&Message],
        token_budget: usize,
        counter: &crate::tokenizer::TokenCounter,
    ) -> Result<String> {
        let start = Instant::now();
        tracing::info!(
            phase = "start",
            input_message_count = messages.len(),
            token_budget = token_budget,
            "summarize.run"
        );

        let trimmed =
            Self::shed_to_fit(&self.prompt_instruction, messages, token_budget, counter).await?;
        let dropped = messages.len() - trimmed.len();
        tracing::info!(
            phase = "trim",
            input_message_count = messages.len(),
            trimmed_message_count = trimmed.len(),
            dropped = dropped,
            "summarize.run"
        );

        let refs: Vec<&Message> = trimmed.iter().copied().collect();
        let prompt = Self::format_prompt(&self.prompt_instruction, &refs);
        tracing::info!(
            phase = "prompt",
            prompt_bytes = prompt.len(),
            "summarize.run"
        );

        let sampling = SamplingParams {
            temperature: 0.3,
            max_tokens: 1024,
            ..SamplingParams::default()
        };

        let result = self.client.complete(&prompt, &[], &sampling).await;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        match &result {
            Ok(summary) => tracing::info!(
                phase = "done",
                result = "ok",
                elapsed_ms = elapsed_ms,
                summary_bytes = summary.len(),
                "summarize.run"
            ),
            Err(err) => tracing::error!(
                phase = "done",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "summarize.run"
            ),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_prompt_basic() {
        let msgs = [
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
        let msgs = [
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
        let msgs = [
            Message::new(Role::System, "You are helpful.".to_owned()),
            Message::new(Role::User, "Hi".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt("Summarize.", &refs);
        assert!(prompt.contains("System: You are helpful."));
    }

    #[tokio::test]
    async fn shed_to_fit_trims_until_prompt_under_budget() {
        use crate::tokenizer::{HeuristicTokenizer, TokenCounter, TokenizerBackend};
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );

        let m1 = Message::new(Role::User, "a".repeat(400));
        let m2 = Message::new(Role::Assistant, "b".repeat(400));
        let m3 = Message::new(Role::User, "c".repeat(400));
        let refs: Vec<&Message> = vec![&m1, &m2, &m3];

        let instruction = "Summarize.";
        let trimmed = Summarizer::shed_to_fit(instruction, &refs, 150, &counter)
            .await
            .expect("shed_to_fit");

        // Under budget once re-rendered with the trimmed set.
        let rendered = Summarizer::format_prompt(instruction, &trimmed);
        let final_count = counter.count_authoritative(&rendered).await.unwrap();
        assert!(
            final_count <= 150,
            "expected rendered prompt ≤ 150 tokens, got {final_count}"
        );

        // At least one message must always remain.
        assert!(!trimmed.is_empty());
    }
}
