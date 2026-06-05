//! Background conversation summarization engine.

use std::time::Instant;

use anyhow::Result;

use crate::client::ApiClient;
use crate::tokenizer::TokenCounter;
use libllm_core::files::{FileError, ResolvedFile};
use libllm_core::sampling::SamplingParams;
use libllm_core::session::{Message, Role};

/// Matches `run_summary_task`'s `SamplingParams.max_tokens` — the reserved space for
/// the response in the file-summary completion call.
pub const MAX_SUMMARY_RESPONSE_TOKENS: usize = 512;

/// Extra headroom to absorb per-flavor tokenizer quirks (BOS/EOS, chat-template wrapping).
pub const SAFETY_PAD: usize = 32;

/// Returns `Ok(())` if the resolved file can be summarized under `context_size` tokens.
/// Returns `Err(FileError::TooLargeForSummary { .. })` if the file is too large to fit,
/// or `Err(FileError::SummaryTokenize { .. })` if the tokenize call itself fails.
///
/// Tokenizes the exact prompt `run_summary_task` would send, so the check is
/// self-consistent with the real call and cache-warms the counter.
pub async fn check_file_fits(
    counter: &TokenCounter,
    file: &ResolvedFile,
    instruction: &str,
    context_size: usize,
) -> Result<(), FileError> {
    let prompt = format!(
        "--- FILE ---\n{}\n--- END FILE ---\n\n{}\n\nSummary:",
        file.body, instruction,
    );
    let prompt_tokens = counter
        .count_authoritative(&prompt)
        .await
        .map_err(|source| FileError::SummaryTokenize {
            path: file.canonical_path.clone(),
            source,
        })?;
    let limit = context_size.saturating_sub(MAX_SUMMARY_RESPONSE_TOKENS + SAFETY_PAD);
    if prompt_tokens > limit {
        return Err(FileError::TooLargeForSummary {
            path: file.canonical_path.clone(),
            tokens: prompt_tokens,
            limit,
        });
    }
    Ok(())
}

/// Stop tokens that terminate summary generation when the model starts hallucinating
/// additional roleplay turns that mimic the conversation block's `User:`/`Assistant:` labels.
const SUMMARY_STOP_TOKENS: &[&str] = &["\nUser:", "\nAssistant:", "\nSystem:"];

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
    ///
    /// The conversation is rendered first inside a delimited block, then the instruction,
    /// ending with a `Summary:` cue. This ordering is load-bearing: if the instruction
    /// precedes the conversation, the model treats the trailing `Assistant:` line as an
    /// ongoing turn and continues the roleplay instead of summarizing.
    ///
    /// `Role::Summary` messages are rendered as `Previous summary:` blocks so a rolling
    /// summary can build on the prior one instead of starting from scratch each time.
    pub fn format_prompt(
        scenario: Option<&str>,
        instruction: &str,
        messages: &[&Message],
        file_summaries: &dyn libllm_core::files::FileSummaryLookup,
    ) -> String {
        let mut prompt = String::new();
        if let Some(s) = scenario
            && !s.trim().is_empty()
        {
            prompt.push_str("Scenario:\n");
            prompt.push_str(s.trim());
            prompt.push_str("\n\n");
        }
        prompt.push_str("--- CONVERSATION ---\n");
        for msg in messages {
            let label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Summary => "Previous summary",
            };
            prompt.push_str(label);
            prompt.push_str(": ");
            prompt.push_str(&render_message_content(msg, file_summaries));
            prompt.push('\n');
        }
        prompt.push_str("--- END CONVERSATION ---\n\n");
        prompt.push_str(instruction);
        prompt.push_str("\n\nSummary:");
        prompt
    }

    /// Shrinks `messages` until `format_prompt(scenario, instruction, trimmed, file_summaries)` is ≤ `token_budget`
    /// according to `counter`. Binary-searches the smallest `k` such that dropping the `k`
    /// oldest non-Summary messages puts the rendered prompt under budget. Always keeps at
    /// least one message if the input is non-empty.
    pub async fn shed_to_fit<'a>(
        scenario: Option<&str>,
        instruction: &str,
        messages: &[&'a Message],
        token_budget: usize,
        counter: &crate::tokenizer::TokenCounter,
        file_summaries: &dyn libllm_core::files::FileSummaryLookup,
    ) -> Result<Vec<&'a Message>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let droppable = libllm_core::context::droppable_count(messages);
        let max_drop = droppable.saturating_sub(1);

        let render_at = |k: usize| -> String {
            let subset = libllm_core::context::drop_oldest_non_summary(messages, k);
            Self::format_prompt(scenario, instruction, &subset, file_summaries)
        };

        let full = render_at(0);
        let full_count = counter.count_authoritative(&full).await?;
        if full_count <= token_budget {
            return Ok(messages.to_vec());
        }

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

        Ok(libllm_core::context::drop_oldest_non_summary(
            messages, best,
        ))
    }

    pub async fn summarize(
        &self,
        scenario: Option<&str>,
        messages: &[&Message],
        token_budget: usize,
        counter: &crate::tokenizer::TokenCounter,
        file_summaries: &dyn libllm_core::files::FileSummaryLookup,
    ) -> Result<String> {
        let start = Instant::now();
        tracing::info!(
            phase = "start",
            input_message_count = messages.len(),
            token_budget = token_budget,
            "summarize.run"
        );

        let trimmed = Self::shed_to_fit(
            scenario,
            &self.prompt_instruction,
            messages,
            token_budget,
            counter,
            file_summaries,
        )
        .await?;
        let dropped = messages.len() - trimmed.len();
        tracing::info!(
            phase = "trim",
            input_message_count = messages.len(),
            trimmed_message_count = trimmed.len(),
            dropped = dropped,
            "summarize.run"
        );

        let prompt =
            Self::format_prompt(scenario, &self.prompt_instruction, &trimmed, file_summaries);
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

        let result = self
            .client
            .complete(&prompt, SUMMARY_STOP_TOKENS, &sampling)
            .await;
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

fn render_message_content(
    msg: &Message,
    lookup: &dyn libllm_core::files::FileSummaryLookup,
) -> String {
    if msg.role != Role::System {
        return msg.content.clone();
    }
    let Some(basename) = libllm_core::files::snapshot_basename(&msg.content) else {
        return msg.content.clone();
    };
    let inner = libllm_core::files::snapshot_inner_text(&msg.content);
    let hash = libllm_core::files::content_hash_hex(inner.as_bytes());
    let lookup_result = lookup.lookup(&hash);
    let branch = match &lookup_result {
        Some(s)
            if s.status == libllm_core::files::FileSummaryStatus::Done && !s.summary.is_empty() =>
        {
            "substituted"
        }
        Some(s) if s.status == libllm_core::files::FileSummaryStatus::Done => "placeholder_empty",
        Some(s) if s.status == libllm_core::files::FileSummaryStatus::Failed => {
            "placeholder_failed"
        }
        Some(_) => "placeholder_pending",
        None => "placeholder_missing",
    };
    tracing::debug!(
        basename = %basename,
        content_hash = %hash,
        branch,
        "summarize.substitute"
    );
    match lookup_result {
        Some(summary)
            if summary.status == libllm_core::files::FileSummaryStatus::Done
                && !summary.summary.is_empty() =>
        {
            format!(
                "The user attached a file \"{basename}\". Summary of its contents:\n{}",
                summary.summary
            )
        }
        _ => format!("The user attached a file \"{basename}\"; summary unavailable."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libllm_core::files::NullFileSummaryLookup;

    #[test]
    fn format_prompt_basic() {
        let msgs = [
            Message::new(Role::User, "Hello".to_owned()),
            Message::new(Role::Assistant, "Hi there!".to_owned()),
            Message::new(Role::User, "How are you?".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt =
            Summarizer::format_prompt(None, "Summarize this.", &refs, &NullFileSummaryLookup);
        assert!(prompt.contains("Summarize this."));
        assert!(prompt.contains("User: Hello"));
        assert!(prompt.contains("Assistant: Hi there!"));
        assert!(prompt.contains("User: How are you?"));
        assert!(prompt.ends_with("\n\nSummary:"));
    }

    #[test]
    fn format_prompt_places_conversation_before_instruction() {
        let msgs = [Message::new(Role::User, "Hi".to_owned())];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt(None, "DO_SUMMARIZE", &refs, &NullFileSummaryLookup);
        let conversation_idx = prompt.find("User: Hi").expect("conversation body");
        let instruction_idx = prompt.find("DO_SUMMARIZE").expect("instruction");
        assert!(
            conversation_idx < instruction_idx,
            "conversation must precede instruction, got prompt: {prompt:?}"
        );
    }

    #[test]
    fn format_prompt_includes_prior_summary_as_context() {
        let msgs = [
            Message::new(Role::Summary, "Old summary".to_owned()),
            Message::new(Role::User, "New message".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt(None, "Summarize.", &refs, &NullFileSummaryLookup);
        assert!(prompt.contains("Previous summary: Old summary"));
        assert!(prompt.contains("User: New message"));
    }

    #[test]
    fn format_prompt_handles_system_messages() {
        let msgs = [
            Message::new(Role::System, "You are helpful.".to_owned()),
            Message::new(Role::User, "Hi".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt(None, "Summarize.", &refs, &NullFileSummaryLookup);
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
        let trimmed = Summarizer::shed_to_fit(
            None,
            instruction,
            &refs,
            150,
            &counter,
            &NullFileSummaryLookup,
        )
        .await
        .expect("shed_to_fit");

        let rendered =
            Summarizer::format_prompt(None, instruction, &trimmed, &NullFileSummaryLookup);
        let final_count = counter.count_authoritative(&rendered).await.unwrap();
        assert!(
            final_count <= 150,
            "expected rendered prompt ≤ 150 tokens, got {final_count}"
        );

        assert!(!trimmed.is_empty());
    }

    use libllm_core::files::{
        FileSummary, FileSummaryLookup, FileSummaryStatus, build_snapshot_body,
    };
    use std::collections::HashMap;

    struct MapLookup(HashMap<String, FileSummary>);
    impl FileSummaryLookup for MapLookup {
        fn lookup(&self, content_hash: &str) -> Option<FileSummary> {
            self.0.get(content_hash).cloned()
        }
    }

    #[test]
    fn format_prompt_substitutes_done_snapshot() {
        let snapshot_body = build_snapshot_body("notes.md", "raw file content");
        let msgs = [
            Message::new(Role::User, "hi".to_owned()),
            Message::new(Role::System, snapshot_body.clone()),
            Message::new(Role::Assistant, "reply".to_owned()),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();

        let inner = libllm_core::files::snapshot_inner_text(&snapshot_body);
        let hash = libllm_core::files::content_hash_hex(inner.as_bytes());
        let mut map = HashMap::new();
        map.insert(
            hash,
            FileSummary {
                basename: "notes.md".to_owned(),
                summary: "Cached summary of notes.md.".to_owned(),
                status: FileSummaryStatus::Done,
            },
        );
        let lookup = MapLookup(map);

        let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &lookup);
        assert!(prompt.contains("Cached summary of notes.md."));
        assert!(!prompt.contains("raw file content"));
        assert!(prompt.contains("User: hi"));
        assert!(prompt.contains("Assistant: reply"));
    }

    #[test]
    fn format_prompt_placeholder_for_failed_or_missing() {
        let snapshot_body = build_snapshot_body("notes.md", "raw file content");
        let msgs = [Message::new(Role::System, snapshot_body)];
        let refs: Vec<&Message> = msgs.iter().collect();

        let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &NullFileSummaryLookup);
        assert!(prompt.contains("summary unavailable"));
        assert!(!prompt.contains("raw file content"));
    }

    #[test]
    fn format_prompt_leaves_non_snapshot_system_messages_alone() {
        let msgs = [Message::new(Role::System, "You are helpful.".to_owned())];
        let refs: Vec<&Message> = msgs.iter().collect();
        let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &NullFileSummaryLookup);
        assert!(prompt.contains("System: You are helpful."));
    }

    #[test]
    fn format_prompt_with_empty_done_summary_uses_placeholder() {
        let snapshot_body = build_snapshot_body("notes.md", "raw file content");
        let msgs = [Message::new(Role::System, snapshot_body.clone())];
        let refs: Vec<&Message> = msgs.iter().collect();

        let inner = libllm_core::files::snapshot_inner_text(&snapshot_body);
        let hash = libllm_core::files::content_hash_hex(inner.as_bytes());
        let mut map = HashMap::new();
        map.insert(
            hash,
            FileSummary {
                basename: "notes.md".to_owned(),
                summary: "".to_owned(),
                status: FileSummaryStatus::Done,
            },
        );
        let lookup = MapLookup(map);

        let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &lookup);
        assert!(prompt.contains("summary unavailable"));
    }

    #[test]
    fn format_prompt_emits_scenario_block_when_set() {
        let refs: Vec<&Message> = vec![];
        let prompt = Summarizer::format_prompt(
            Some("A medieval tavern at dusk."),
            "Summarize.",
            &refs,
            &NullFileSummaryLookup,
        );
        assert!(prompt.starts_with("Scenario:\nA medieval tavern at dusk.\n\n"));
    }

    #[test]
    fn format_prompt_omits_scenario_block_when_absent() {
        let refs: Vec<&Message> = vec![];
        let prompt = Summarizer::format_prompt(None, "Summarize.", &refs, &NullFileSummaryLookup);
        assert!(!prompt.contains("Scenario:"));
    }

    #[test]
    fn format_prompt_omits_scenario_block_when_whitespace_only() {
        let refs: Vec<&Message> = vec![];
        let prompt =
            Summarizer::format_prompt(Some("   \n  "), "Summarize.", &refs, &NullFileSummaryLookup);
        assert!(!prompt.contains("Scenario:"));
    }

    fn heuristic_token_counter() -> (
        crate::tokenizer::TokenCounter,
        tokio::sync::mpsc::Receiver<crate::tokenizer::TokenCountUpdate>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let counter = crate::tokenizer::TokenCounter::new_with_backend(
            crate::tokenizer::TokenizerBackend::Heuristic(
                crate::tokenizer::HeuristicTokenizer::standard(),
            ),
            tx,
        );
        (counter, rx)
    }

    fn small_file(body: &str) -> libllm_core::files::ResolvedFile {
        libllm_core::files::ResolvedFile {
            raw_token: "@notes.md".to_owned(),
            canonical_path: std::path::PathBuf::from("/tmp/notes.md"),
            basename: "notes.md".to_owned(),
            body: body.to_owned(),
            byte_size: body.len(),
        }
    }

    #[tokio::test]
    async fn check_file_fits_accepts_small_file() {
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file("hello world");
        let result = check_file_fits(&counter, &file, "Summarize this file.", 4096).await;
        assert!(result.is_ok(), "expected small file to fit, got {result:?}");
    }

    #[tokio::test]
    async fn check_file_fits_rejects_when_prompt_exceeds_limit() {
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file(&"a".repeat(100_000));
        let result = check_file_fits(&counter, &file, "Summarize this file.", 4096).await;
        let err = result.expect_err("expected TooLargeForSummary");
        match err {
            libllm_core::files::FileError::TooLargeForSummary { tokens, limit, .. } => {
                assert!(
                    tokens > limit,
                    "tokens {tokens} should exceed limit {limit}"
                );
            }
            other => panic!("expected TooLargeForSummary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_file_fits_uses_instruction_length_dynamically() {
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file(&"a".repeat(12_000));
        let short = check_file_fits(&counter, &file, "Summarize.", 4096).await;
        assert!(
            short.is_err(),
            "12k-char body must not fit in 4096-token context"
        );

        let ok = check_file_fits(&counter, &file, "Summarize.", 131_072).await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn check_file_fits_rejects_when_context_size_below_response_reserve() {
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file("tiny");
        let result = check_file_fits(&counter, &file, "Summarize.", 100).await;
        match result {
            Err(libllm_core::files::FileError::TooLargeForSummary { limit, .. }) => {
                assert_eq!(limit, 0)
            }
            other => panic!("expected TooLargeForSummary with limit=0, got {other:?}"),
        }
    }
}
