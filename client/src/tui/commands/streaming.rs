//! Streaming completion request lifecycle: start, token handling, and worldbook loading.

use anyhow::Result;
use tokio::sync::mpsc;

use libllm::client::StreamToken;
use libllm::session::{Message, Role};

use crate::tui::business;
use crate::tui::types::{SaveTrigger, StatusLevel, WorldbookCache};

use super::App;

pub(in crate::tui::commands) fn loaded_worldbooks(app: &mut App) -> Vec<libllm::worldinfo::RuntimeWorldBook> {
    let enabled_names = business::enabled_worldbook_names(app.session, &app.config);
    let cache_stale = app
        .worldbook_cache
        .as_ref()
        .is_none_or(|cache| cache.enabled_names != enabled_names);

    if cache_stale {
        let books = libllm::debug_log::timed_kv(
            "worldbook.runtime",
            &[
                libllm::debug_log::field("phase", "load"),
                libllm::debug_log::field("cache", "miss"),
                libllm::debug_log::field("enabled_count", enabled_names.len()),
            ],
            || business::load_runtime_worldbooks(&enabled_names, app.db.as_ref()),
        );
        app.worldbook_cache = Some(WorldbookCache {
            enabled_names,
            books,
        });
    } else if let Some(cache) = app.worldbook_cache.as_ref() {
        libllm::debug_log::log_kv(
            "worldbook.runtime",
            &[
                libllm::debug_log::field("phase", "load"),
                libllm::debug_log::field("cache", "hit"),
                libllm::debug_log::field("enabled_count", enabled_names.len()),
                libllm::debug_log::field("book_count", cache.books.len()),
            ],
        );
    }

    app.worldbook_cache.as_ref().unwrap().books.clone()
}

pub(in crate::tui) fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    if app.summary_receiver.is_some() {
        app.is_summarizing = true;
        app.message_queue.push(content.to_owned());
        libllm::debug_log::log_kv(
            "stream.start",
            &[
                libllm::debug_log::field("phase", "queued_for_summary"),
                libllm::debug_log::field("queue_len", app.message_queue.len()),
            ],
        );
        return;
    }
    if app.model_name.is_none() {
        libllm::debug_log::log_kv(
            "stream.start",
            &[
                libllm::debug_log::field("phase", "blocked"),
                libllm::debug_log::field("reason", "model_pending"),
            ],
        );
        app.set_status(
            "Connecting to API server...".to_owned(),
            StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
        libllm::debug_log::log_kv(
            "stream.start",
            &[
                libllm::debug_log::field("phase", "blocked"),
                libllm::debug_log::field("reason", "api_unavailable"),
            ],
        );
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            StatusLevel::Error,
        );
        return;
    }
    let parent = app.session.tree.head();
    app.session
        .tree
        .push(parent, Message::new(Role::User, content.to_owned()));
    app.mark_session_dirty(SaveTrigger::Debounced, false);
    app.invalidate_chat_cache();
    app.is_streaming = true;
    app.focus = crate::tui::Focus::Input;
    app.nav_cursor = None;
    app.hover_node = None;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let context_messages = app.context_mgr.summary_aware_path(&branch_path);
    let truncated = app.context_mgr.truncated_path(&context_messages);
    let effective_prompt = business::build_effective_system_prompt(app.session, app.db.as_ref());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected = business::inject_loaded_worldbook_entries(
        app.session,
        truncated,
        user_name,
        &worldbooks,
    );
    let injected = business::replace_template_vars(app.session, injected, user_name);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app
        .instruct_preset
        .render(&injected_refs, effective_prompt.as_deref());
    let stop_tokens = app.stop_tokens.clone();
    let sampling = app.sampling.clone();

    libllm::debug_log::log_kv(
        "stream.start",
        &[
            libllm::debug_log::field("phase", "dispatch"),
            libllm::debug_log::field("branch_len", branch_path.len()),
            libllm::debug_log::field("summary_aware_len", context_messages.len()),
            libllm::debug_log::field("truncated_len", truncated.len()),
            libllm::debug_log::field("worldbook_count", worldbooks.len()),
            libllm::debug_log::field("has_system_prompt", effective_prompt.is_some()),
            libllm::debug_log::field("stop_token_count", stop_tokens.len()),
            libllm::debug_log::field("prompt_bytes", prompt.len()),
            libllm::debug_log::field("continuation", false),
        ],
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

pub(in crate::tui) fn handle_stream_token(
    token: StreamToken,
    app: &mut App,
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
            libllm::debug_log::log_kv(
                "stream.done",
                &[
                    libllm::debug_log::field("result", "ok"),
                    libllm::debug_log::field("bytes", response_bytes),
                    libllm::debug_log::field("is_continuation", is_continuation),
                    libllm::debug_log::field("node_id", head),
                ],
            );
            app.mark_session_dirty(SaveTrigger::StreamDone, true);
            app.invalidate_chat_cache();
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.flush_session_save(SaveTrigger::StreamDone)?;
            business::refresh_sidebar(app);
            if app.summarization_enabled && app.summary_receiver.is_none() {
                let branch_path = app.session.tree.branch_path();
                let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
                let dropped = app.context_mgr.dropped_message_count(&summary_aware);
                let trigger_threshold = app.config.summarization.trigger_threshold;

                if dropped >= trigger_threshold {
                    let summary_boundary = branch_path.len() - summary_aware.len();
                    let messages_to_summarize: Vec<Message> = branch_path
                        [..summary_boundary + dropped]
                        .iter()
                        .filter(|m| m.role != Role::Summary)
                        .map(|m| (*m).clone())
                        .collect();

                    if !messages_to_summarize.is_empty() {
                        libllm::debug_log::log_kv(
                            "stream.summary.schedule",
                            &[
                                libllm::debug_log::field("result", "scheduled"),
                                libllm::debug_log::field("dropped", dropped),
                                libllm::debug_log::field("trigger_threshold", trigger_threshold),
                                libllm::debug_log::field("summary_boundary", summary_boundary),
                                libllm::debug_log::field(
                                    "messages_to_summarize",
                                    messages_to_summarize.len(),
                                ),
                            ],
                        );
                        let summarize_api_url = app
                            .config
                            .summarization
                            .api_url
                            .as_deref()
                            .unwrap_or(app.client.base_url());
                        let summarizer_client = libllm::client::ApiClient::new(
                            summarize_api_url,
                            app.config.tls_skip_verify || app.cli_overrides.tls_skip_verify,
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

                        tokio::spawn(async move {
                            let refs: Vec<&Message> = messages_to_summarize.iter().collect();
                            let result = summarizer.summarize(&refs, token_budget).await;
                            let _ = tx.send(result.map_err(|e| e.to_string()));
                        });
                    }
                }
            }
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                start_streaming(app, &next, sender);
                if !app.is_streaming {
                    app.message_queue.clear();
                }
            }
        }
        StreamToken::Error(err) => {
            libllm::debug_log::log_kv(
                "stream.done",
                &[
                    libllm::debug_log::field("result", "error"),
                    libllm::debug_log::field("is_continuation", app.is_continuation),
                    libllm::debug_log::field("error", &err),
                ],
            );
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.is_continuation = false;
            app.message_queue.clear();
            app.set_status(format!("Error: {err}"), StatusLevel::Error);
        }
    }
    Ok(())
}
