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
    if app.model_name.is_none() {
        app.set_status(
            "Connecting to API server...".to_owned(),
            StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
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
    let truncated = app.context_mgr.truncated_path(&branch_path);
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
            app.mark_session_dirty(SaveTrigger::StreamDone, true);
            app.invalidate_chat_cache();
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.flush_session_save(SaveTrigger::StreamDone)?;
            business::refresh_sidebar(app);
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                start_streaming(app, &next, sender);
                if !app.is_streaming {
                    app.message_queue.clear();
                }
            }
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.is_continuation = false;
            app.message_queue.clear();
            app.set_status(format!("Error: {err}"), StatusLevel::Error);
        }
    }
    Ok(())
}
