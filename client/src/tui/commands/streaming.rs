//! Streaming completion request lifecycle: start, token handling, and worldbook loading.

use std::collections::HashMap;

use anyhow::Result;
use tokio::sync::mpsc;

use libllm::client::StreamToken;
use libllm::preset::InstructPreset;
use libllm::session::{Message, Role};

use crate::tui::business;
use crate::tui::types::{SaveTrigger, StatusLevel, WorldbookCache};

use super::App;

struct SnapshotFileSummaryLookup(HashMap<String, libllm::files::FileSummary>);

impl libllm::files::FileSummaryLookup for SnapshotFileSummaryLookup {
    fn lookup(&self, content_hash: &str) -> Option<libllm::files::FileSummary> {
        self.0.get(content_hash).cloned()
    }
}

pub(in crate::tui::commands) fn loaded_worldbooks(
    app: &mut App,
) -> Vec<libllm::worldinfo::RuntimeWorldBook> {
    let enabled_names = business::enabled_worldbook_names(app.session, &app.config);
    let cache_stale = app
        .worldbook_cache
        .as_ref()
        .is_none_or(|cache| cache.enabled_names != enabled_names);

    if cache_stale {
        let books = {
            let _span = tracing::debug_span!(
                "worldbook.runtime",
                phase = "load",
                cache = "miss",
                enabled_count = enabled_names.len()
            )
            .entered();
            business::load_runtime_worldbooks(&enabled_names, app.db.as_ref())
        };
        app.worldbook_cache = Some(WorldbookCache {
            enabled_names,
            books,
        });
    } else if let Some(cache) = app.worldbook_cache.as_ref() {
        tracing::debug!(
            phase = "load",
            cache = "hit",
            enabled_count = enabled_names.len(),
            book_count = cache.books.len(),
            "worldbook.runtime"
        );
    }

    app.worldbook_cache.as_ref().unwrap().books.clone()
}

fn build_rendered_prompt_common<F>(app: &crate::tui::App, dropped: usize, render: F) -> String
where
    F: FnOnce(&InstructPreset, &[&libllm::session::Message], Option<&str>) -> String,
{
    let worldbooks = cached_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let context_messages = app.context_mgr.summary_aware_path(&branch_path);
    let trimmed = libllm::context::drop_oldest_non_summary(&context_messages, dropped);
    let effective_prompt = business::build_effective_system_prompt(app.session, app.db.as_ref());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected =
        business::inject_loaded_worldbook_entries(app.session, &trimmed, user_name, &worldbooks);
    let injected = business::replace_template_vars(app.session, injected, user_name);
    let injected: Vec<libllm::session::Message> = injected
        .into_iter()
        .map(|m| match m.role {
            libllm::session::Role::User => libllm::session::Message {
                role: m.role,
                content: libllm::files::rewrite_user_message(&m.content),
                timestamp: m.timestamp.clone(),
            },
            _ => m,
        })
        .collect();
    let injected_refs: Vec<&libllm::session::Message> = injected.iter().collect();
    render(
        &app.instruct_preset,
        &injected_refs,
        effective_prompt.as_deref(),
    )
}

/// Builds the final prompt string for streaming, with the `dropped` oldest non-summary
/// messages removed. This is the exact byte stream that would be POSTed to `/completion`.
/// Used by both the pre-send shrink loop and the chat pane's displayed-count path.
pub(in crate::tui) fn build_rendered_prompt(app: &crate::tui::App, dropped: usize) -> String {
    let prompt =
        build_rendered_prompt_common(app, dropped, |preset, refs, sys| preset.render(refs, sys));
    match app.reasoning_preset.as_ref() {
        Some(preset) => preset.apply_prefix(&prompt),
        None => prompt,
    }
}

/// Same as `build_rendered_prompt` but uses `InstructPreset::render_continuation` instead
/// of `render`. Used by the `/continue` command path in `commands/mod.rs`.
pub(in crate::tui) fn build_rendered_prompt_continuation(
    app: &crate::tui::App,
    dropped: usize,
) -> String {
    build_rendered_prompt_common(app, dropped, |preset, refs, sys| {
        preset.render_continuation(refs, sys)
    })
}

/// Read-only view of the worldbook cache for `build_rendered_prompt*`. The cache is
/// always populated by a prior `loaded_worldbooks` call in the same request path; a miss
/// yields an empty slice, which is a correct (if degraded) rendering.
fn cached_worldbooks(app: &crate::tui::App) -> Vec<libllm::worldinfo::RuntimeWorldBook> {
    app.worldbook_cache
        .as_ref()
        .map(|cache| cache.books.clone())
        .unwrap_or_default()
}

/// Binary-searches the smallest `k ∈ [0, max_drop]` such that
/// `counter.count_authoritative(&render(k)).await? ≤ budget`. Returns `max_drop` if no
/// value satisfies the budget (defensive fallback; the caller logs this as a warning).
pub(in crate::tui) async fn find_smallest_drop<F>(
    counter: &libllm::tokenizer::TokenCounter,
    budget: usize,
    max_drop: usize,
    render: &F,
) -> anyhow::Result<usize>
where
    F: Fn(usize) -> String,
{
    let full_count = counter.count_authoritative(&render(0)).await?;
    if full_count <= budget {
        return Ok(0);
    }

    let (mut lo, mut hi) = (1usize, max_drop);
    let mut best = max_drop;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let count = counter.count_authoritative(&render(mid)).await?;
        if count <= budget {
            best = mid;
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    Ok(best)
}

enum StreamPreflight {
    Proceed,
    Queued,
    Blocked,
}

fn stream_preflight(app: &mut App<'_>, content: &str) -> StreamPreflight {
    if app.summary_receiver.is_some() {
        app.is_summarizing = true;
        app.message_queue.push(content.to_owned());
        tracing::debug!(
            phase = "queued_for_summary",
            queue_len = app.message_queue.len(),
            "stream.start"
        );
        return StreamPreflight::Queued;
    }
    if app.model_name.is_none() {
        tracing::debug!(phase = "blocked", reason = "model_pending", "stream.start");
        app.set_status(
            "Connecting to API server...".to_owned(),
            StatusLevel::Warning,
        );
        return StreamPreflight::Blocked;
    }
    if !app.api_available {
        tracing::debug!(
            phase = "blocked",
            reason = "api_unavailable",
            "stream.start"
        );
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            StatusLevel::Error,
        );
        return StreamPreflight::Blocked;
    }
    StreamPreflight::Proceed
}

fn push_user_segments(app: &mut App<'_>, content: &str) {
    let mut parent = app.session.tree.head();
    let segments: Vec<String> = if app.session.character.is_some() {
        libllm::side_character::split_user_input(content)
    } else {
        vec![content.to_owned()]
    };
    for segment in segments {
        let new_id = app
            .session
            .tree
            .push(parent, Message::new(Role::User, segment));
        parent = Some(new_id);
    }
}

async fn launch_stream(app: &mut App<'_>, sender: mpsc::Sender<StreamToken>) {
    app.mark_session_dirty(SaveTrigger::Debounced, false);
    app.invalidate_chat_cache();
    app.is_streaming = true;
    app.focus = crate::tui::Focus::Input;
    app.nav_cursor = None;
    app.hover_node = None;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let budget = app.context_mgr.token_limit();
    let branch_path = app.session.tree.branch_path();
    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
    let max_drop = libllm::context::droppable_count(&summary_aware).saturating_sub(1);

    let render = |k: usize| -> String { build_rendered_prompt(app, k) };

    let dropped = match find_smallest_drop(&app.token_counter, budget, max_drop, &render).await {
        Ok(k) => k,
        Err(err) => {
            tracing::warn!(
                result = "fallback_heuristic",
                error = %err,
                "stream.truncate"
            );
            0
        }
    };
    let effective_prompt = business::build_effective_system_prompt(app.session, app.db.as_ref());
    let prompt = build_rendered_prompt(app, dropped);
    let stop_tokens = app.stop_tokens.clone();
    let sampling = app.sampling.clone();

    tracing::info!(
        phase = "dispatch",
        branch_len = branch_path.len(),
        summary_aware_len = summary_aware.len(),
        dropped = dropped,
        worldbook_count = worldbooks.len(),
        has_system_prompt = effective_prompt.is_some(),
        stop_token_count = stop_tokens.len(),
        prompt_bytes = prompt.len(),
        continuation = false,
        "stream.start"
    );

    let client = app.client.clone();
    let handle = tokio::spawn(async move {
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        client
            .stream_completion_to_channel(&prompt, &stop_refs, &sampling, sender)
            .await;
    });
    app.streaming_task = Some(handle);
}

pub(in crate::tui) async fn start_streaming(
    app: &mut App<'_>,
    content: &str,
    sender: mpsc::Sender<StreamToken>,
) {
    match stream_preflight(app, content) {
        StreamPreflight::Proceed => {}
        StreamPreflight::Queued => {
            app.clear_input_textarea_if_holds(content);
            return;
        }
        StreamPreflight::Blocked => return,
    }
    debug_assert!(!content.trim().is_empty(), "start_streaming called with blank content");
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let sys_messages = match libllm::files::resolve_all(content, &cwd, &app.config.files) {
        Ok(v) => v,
        Err(libllm::files::FileError::Collision { path, kind }) => {
            crate::tui::dialogs::injection_warning::open(app, &path, kind);
            return;
        }
        Err(err) => {
            app.set_status(err.to_string(), crate::tui::types::StatusLevel::Error);
            return;
        }
    };
    app.clear_input_textarea_if_holds(content);
    match (
        app.config.files.summarize_mode == libllm::config::FileSummarizeMode::Eager,
        app.config.summarization.enabled,
        app.save_mode.id(),
        app.file_summarizer.as_ref(),
    ) {
        (false, _, _, _) => tracing::debug!(
            reason = "mode_lazy",
            "files.summary.eager_schedule.skipped"
        ),
        (_, false, _, _) => tracing::debug!(
            reason = "summarization_disabled",
            "files.summary.eager_schedule.skipped"
        ),
        (_, _, None, _) => tracing::debug!(
            reason = "no_session_id",
            "files.summary.eager_schedule.skipped"
        ),
        (_, _, _, None) => tracing::debug!(
            reason = "no_summarizer",
            "files.summary.eager_schedule.skipped"
        ),
        (true, true, Some(session_id), Some(summarizer)) => {
            let to_summarize = libllm::files::files_to_summarize_from_messages(&sys_messages);
            tracing::info!(
                session_id = %session_id,
                file_count = to_summarize.len(),
                "files.summary.eager_schedule.dispatching"
            );
            for file in &to_summarize {
                summarizer.schedule(session_id, file);
            }
        }
    }

    let mut parent = app.session.tree.head();
    for sys_msg in sys_messages {
        let new_id = app.session.tree.push(parent, sys_msg);
        parent = Some(new_id);
    }
    push_user_segments(app, content);

    launch_stream(app, sender).await;
}

/// Push a new user message at the current head and stream. Unlike
/// `start_streaming`, this does not resolve `@file` references, so file
/// snapshots already present in the branch are shared with the new sibling
/// rather than duplicated.
pub(in crate::tui) async fn start_retry_streaming(
    app: &mut App<'_>,
    content: &str,
    sender: mpsc::Sender<StreamToken>,
) {
    match stream_preflight(app, content) {
        StreamPreflight::Proceed => {}
        StreamPreflight::Queued => {
            app.clear_input_textarea_if_holds(content);
            return;
        }
        StreamPreflight::Blocked => return,
    }
    debug_assert!(
        !content.trim().is_empty(),
        "start_retry_streaming called with blank content"
    );
    app.clear_input_textarea_if_holds(content);
    push_user_segments(app, content);
    launch_stream(app, sender).await;
}

pub(in crate::tui) async fn handle_stream_token(
    token: StreamToken,
    app: &mut App<'_>,
    sender: mpsc::Sender<StreamToken>,
) -> Result<()> {
    if !app.is_streaming {
        return Ok(());
    }
    match token {
        StreamToken::Token(text) => {
            app.streaming_buffer.push_str(&text);
            app.auto_scroll = true;
        }
        StreamToken::Done(full_response) => {
            let head = app.session.tree.head().unwrap();
            let response_bytes = full_response.len();
            let is_continuation = app.is_continuation;
            if app.is_continuation {
                let existing = app.session.tree.node(head).unwrap().message.content.clone();
                let combined = format!("{}{}", existing, full_response);
                app.session.tree.set_message_content(head, combined);
                app.is_continuation = false;
            } else {
                app.session
                    .tree
                    .push(Some(head), Message::new(Role::Assistant, full_response));
            }
            tracing::info!(
                result = "ok",
                bytes = response_bytes,
                is_continuation,
                node_id = head,
                "stream.done"
            );
            app.mark_session_dirty(SaveTrigger::StreamDone, true);
            app.invalidate_chat_cache();
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.flush_session_save(SaveTrigger::StreamDone)?;
            business::refresh_sidebar(app);
            if app.summarization_enabled && app.summary_receiver.is_none() {
                let budget = app.context_mgr.token_limit();
                let branch_path = app.session.tree.branch_path();
                let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
                let max_drop = libllm::context::droppable_count(&summary_aware).saturating_sub(1);
                let render = |k: usize| -> String { build_rendered_prompt(app, k) };
                let dropped =
                    match find_smallest_drop(&app.token_counter, budget, max_drop, &render).await {
                        Ok(k) => k,
                        Err(err) => {
                            tracing::warn!(
                                result = "fallback_heuristic",
                                error = %err,
                                "stream.summary.truncate"
                            );
                            0
                        }
                    };
                let trigger_threshold = app.config.summarization.trigger_threshold;

                if dropped >= trigger_threshold {
                    let keep_last = app.config.summarization.keep_last;
                    let droppable = libllm::context::droppable_count(&summary_aware);
                    let aggressive = droppable.saturating_sub(keep_last);
                    let dropped = aggressive.max(dropped).min(max_drop);
                    let summary_boundary = branch_path.len() - summary_aware.len();
                    let split_idx = libllm::context::drop_split_index(&summary_aware, dropped);
                    let messages_to_summarize: Vec<Message> = branch_path
                        [summary_boundary..summary_boundary + split_idx]
                        .iter()
                        .map(|m| (*m).clone())
                        .collect();

                    if !messages_to_summarize.is_empty() {
                        tracing::info!(
                            result = "scheduled",
                            dropped,
                            trigger_threshold,
                            summary_boundary,
                            messages_to_summarize = messages_to_summarize.len(),
                            "stream.summary.schedule"
                        );
                        let session_id_for_summarizer = app.save_mode.id().map(str::to_owned);
                        let files_to_wait_on =
                            libllm::files::files_to_summarize_from_messages(&messages_to_summarize);

                        if !files_to_wait_on.is_empty() {
                            if let (Some(session_id), Some(summarizer_svc)) = (
                                session_id_for_summarizer.as_deref(),
                                app.file_summarizer.as_ref(),
                            ) {
                                tracing::info!(
                                    session_id = %session_id,
                                    file_count = files_to_wait_on.len(),
                                    "files.summary.ensure_ready.dispatch"
                                );
                                if let Err(err) = summarizer_svc
                                    .ensure_ready(session_id, &files_to_wait_on)
                                    .await
                                {
                                    tracing::warn!(
                                        result = "error",
                                        error = %err,
                                        "files.summary.ensure_ready_before_auto_summarize"
                                    );
                                }
                            } else {
                                tracing::debug!(
                                    file_count = files_to_wait_on.len(),
                                    session_present = app.save_mode.id().is_some(),
                                    summarizer_present = app.file_summarizer.is_some(),
                                    "files.summary.ensure_ready.skipped"
                                );
                            }
                        }

                        let summaries_snapshot: HashMap<String, libllm::files::FileSummary> =
                            if let (Some(session_id), Some(summarizer_svc)) = (
                                session_id_for_summarizer.as_deref(),
                                app.file_summarizer.as_ref(),
                            ) {
                                let snapshot: HashMap<String, libllm::files::FileSummary> =
                                    files_to_wait_on
                                        .iter()
                                        .filter_map(|f| {
                                            summarizer_svc
                                                .lookup(session_id, &f.content_hash)
                                                .map(|s| (f.content_hash.clone(), s))
                                        })
                                        .collect();
                                tracing::debug!(
                                    session_id = %session_id,
                                    snapshot_size = snapshot.len(),
                                    wanted = files_to_wait_on.len(),
                                    "files.summary.snapshot.built"
                                );
                                snapshot
                            } else {
                                tracing::debug!(
                                    session_id = session_id_for_summarizer.as_deref().unwrap_or(""),
                                    snapshot_size = 0usize,
                                    wanted = files_to_wait_on.len(),
                                    "files.summary.snapshot.built"
                                );
                                HashMap::new()
                            };

                        let summarize_api_url = crate::tui::business::summarize_api_url(
                            &app.config,
                            &app.cli_overrides,
                        );
                        let summarizer_auth = libllm::config::resolve_auth(
                            &app.config,
                            &app.cli_overrides.auth_overrides(),
                        );
                        let summarizer_client = libllm::client::ApiClient::new(
                            &summarize_api_url,
                            app.config.tls_skip_verify || app.cli_overrides.tls_skip_verify,
                            summarizer_auth,
                        );
                        let summarizer = libllm::summarize::Summarizer::new(
                            summarizer_client,
                            app.config.summarization.prompt.clone(),
                        );
                        let token_budget = app.context_mgr.token_limit();
                        let current_head = app.session.tree.head();

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        app.summary_receiver = Some(rx);
                        app.summary_branch_head = current_head;
                        app.summary_pending_dropped = Some(dropped);
                        app.is_summarizing = true;

                        let summary_counter = app.token_counter.clone();
                        tokio::spawn(async move {
                            let refs: Vec<&Message> = messages_to_summarize.iter().collect();
                            let lookup = SnapshotFileSummaryLookup(summaries_snapshot);
                            let result = summarizer
                                .summarize(&refs, token_budget, &summary_counter, &lookup)
                                .await;
                            let _ = tx.send(result.map_err(|e| e.to_string()));
                        });
                    }
                }
            }
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                Box::pin(start_streaming(app, &next, sender)).await;
                if !app.is_streaming {
                    app.message_queue.clear();
                }
            }
        }
        StreamToken::Error(err) => {
            tracing::error!(result = "error", is_continuation = app.is_continuation, error = %err, "stream.done");
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.is_continuation = false;
            app.message_queue.clear();
            app.set_status(format!("Error: {err}"), StatusLevel::Error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn find_smallest_drop_binary_search() {
        use libllm::tokenizer::{HeuristicTokenizer, TokenCounter, TokenizerBackend};

        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );

        let render_at = |k: usize| -> String {
            let chars = 400usize.saturating_sub(40 * k);
            "a".repeat(chars)
        };

        let k = find_smallest_drop(&counter, 60, 8, &render_at)
            .await
            .unwrap();
        assert_eq!(k, 5);
    }

    #[tokio::test]
    async fn find_smallest_drop_zero_when_fits() {
        use libllm::tokenizer::{HeuristicTokenizer, TokenCounter, TokenizerBackend};
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );
        let render_at = |_k: usize| -> String { "x".repeat(40) };
        let k = find_smallest_drop(&counter, 100, 8, &render_at)
            .await
            .unwrap();
        assert_eq!(k, 0);
    }

    #[test]
    fn rewrite_user_message_pass_substitutes_at_tokens_on_user_role_only() {
        use libllm::session::{Message, Role};

        let original = [
            Message::new(Role::User, "@./notes.md please".to_owned()),
            Message::new(Role::Assistant, "first response".to_owned()),
            Message::new(Role::User, "also @./code.rs".to_owned()),
            Message::new(Role::System, "system literal @./leave.md alone".to_owned()),
        ];
        let rewritten: Vec<Message> = original
            .iter()
            .map(|m| match m.role {
                Role::User => Message {
                    role: m.role,
                    content: libllm::files::rewrite_user_message(&m.content),
                    timestamp: m.timestamp.clone(),
                },
                _ => m.clone(),
            })
            .collect();
        assert_eq!(rewritten[0].content, "[notes.md] please");
        assert_eq!(rewritten[1].content, "first response");
        assert_eq!(rewritten[2].content, "also [code.rs]");
        assert_eq!(rewritten[3].content, "system literal @./leave.md alone");
    }
}
