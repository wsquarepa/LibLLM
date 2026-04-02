use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::client::StreamToken;
use crate::session::{self, Message, Role, SaveMode};

use super::business::{self, load_config_fields, refresh_sidebar};
use super::{App, Focus, dialogs, maintenance};

const SIDEBAR_METADATA_WORKERS: usize = 4;

fn post_passkey_focus(app: &App) -> Focus {
    if app.model_name.is_none() {
        Focus::LoadingDialog
    } else if !app.api_available {
        Focus::ApiErrorDialog
    } else {
        Focus::Input
    }
}

fn loaded_worldbooks(app: &mut App) -> Vec<crate::worldinfo::RuntimeWorldBook> {
    let enabled_names = super::business::enabled_worldbook_names(app.session, &app.config);
    let cache_stale = app
        .worldbook_cache
        .as_ref()
        .is_none_or(|cache| cache.enabled_names != enabled_names);

    if cache_stale {
        let books = crate::debug_log::timed_kv(
            "worldbook.runtime",
            &[
                crate::debug_log::field("phase", "load"),
                crate::debug_log::field("cache", "miss"),
                crate::debug_log::field("enabled_count", enabled_names.len()),
            ],
            || super::business::load_runtime_worldbooks(&enabled_names, app.save_mode.key()),
        );
        app.worldbook_cache = Some(super::WorldbookCache {
            enabled_names,
            books,
        });
    } else if let Some(cache) = app.worldbook_cache.as_ref() {
        crate::debug_log::log_kv(
            "worldbook.runtime",
            &[
                crate::debug_log::field("phase", "load"),
                crate::debug_log::field("cache", "hit"),
                crate::debug_log::field("enabled_count", enabled_names.len()),
                crate::debug_log::field("book_count", cache.books.len()),
            ],
        );
    }

    app.worldbook_cache.as_ref().unwrap().books.clone()
}

pub(super) fn spawn_metadata_loading(app: &mut App) {
    let key = match &app.save_mode {
        SaveMode::Encrypted { key, .. } => key.clone(),
        _ => return,
    };

    app.sidebar_hydration_generation = app.sidebar_hydration_generation.wrapping_add(1);
    let generation = app.sidebar_hydration_generation;
    let candidate_paths = collect_metadata_loading_paths(&app.sidebar_sessions);

    if candidate_paths.is_empty() {
        crate::debug_log::log_kv(
            "sidebar.hydration",
            &[
                crate::debug_log::field("phase", "schedule"),
                crate::debug_log::field("generation", generation),
                crate::debug_log::field("scheduled", 0),
                crate::debug_log::field("workers", 0),
                crate::debug_log::field("result", "skipped"),
            ],
        );
        app.hydration_debug = None;
        return;
    }

    let worker_count = candidate_paths.len().min(SIDEBAR_METADATA_WORKERS);
    let batches = split_metadata_loading_batches(candidate_paths, worker_count);
    crate::debug_log::log_kv(
        "sidebar.hydration",
        &[
            crate::debug_log::field("phase", "schedule"),
            crate::debug_log::field("generation", generation),
            crate::debug_log::field("scheduled", batches.iter().map(Vec::len).sum::<usize>()),
            crate::debug_log::field("workers", worker_count),
            crate::debug_log::field("batch_count", batches.len()),
            crate::debug_log::field("result", "scheduled"),
        ],
    );
    app.hydration_debug = Some(super::HydrationDebugState {
        generation,
        started_at: Instant::now(),
        scheduled: batches.iter().map(Vec::len).sum(),
        completed: 0,
        failed: 0,
        stale_dropped: 0,
        missing_dropped: 0,
        batch_total: batches.len(),
        batch_finished: 0,
    });

    for (batch_index, batch) in batches.into_iter().enumerate() {
        let key = key.clone();
        let tx = app.bg_tx.clone();
        crate::debug_log::log_kv(
            "sidebar.hydration",
            &[
                crate::debug_log::field("phase", "batch_schedule"),
                crate::debug_log::field("generation", generation),
                crate::debug_log::field("batch", batch_index),
                crate::debug_log::field("batch_size", batch.len()),
            ],
        );
        tokio::spawn(async move {
            let batch_start = Instant::now();
            let mut loaded_count = 0usize;
            let mut failed_count = 0usize;
            for entry_path in batch {
                let path_for_event = entry_path.clone();
                let key = key.clone();
                let result =
                    tokio::task::spawn_blocking(move || session::load_metadata(&entry_path, &key))
                        .await;

                match result {
                    Ok(Ok(metadata)) => {
                        loaded_count += 1;
                        if tx
                            .send(super::BackgroundEvent::MetadataLoaded {
                                generation,
                                path: path_for_event,
                                metadata,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(Err(err)) => {
                        failed_count += 1;
                        crate::debug_log::log_kv(
                            "sidebar.hydration",
                            &[
                                crate::debug_log::field("phase", "load"),
                                crate::debug_log::field("generation", generation),
                                crate::debug_log::field("result", "error"),
                                crate::debug_log::field("path", path_for_event.display()),
                                crate::debug_log::field("error", err),
                            ],
                        );
                    }
                    Err(err) => {
                        failed_count += 1;
                        crate::debug_log::log_kv(
                            "sidebar.hydration",
                            &[
                                crate::debug_log::field("phase", "task"),
                                crate::debug_log::field("generation", generation),
                                crate::debug_log::field("result", "error"),
                                crate::debug_log::field("path", path_for_event.display()),
                                crate::debug_log::field("error", err),
                            ],
                        );
                    }
                }
            }
            crate::debug_log::log_kv(
                "sidebar.hydration",
                &[
                    crate::debug_log::field("phase", "batch_complete"),
                    crate::debug_log::field("generation", generation),
                    crate::debug_log::field("batch", batch_index),
                    crate::debug_log::field("loaded", loaded_count),
                    crate::debug_log::field("failed", failed_count),
                    crate::debug_log::field(
                        "elapsed_ms",
                        format!("{:.3}", batch_start.elapsed().as_secs_f64() * 1000.0),
                    ),
                ],
            );
            let _ = tx
                .send(super::BackgroundEvent::MetadataBatchFinished {
                    generation,
                    loaded_count,
                    failed_count,
                })
                .await;
        });
    }
}

fn collect_metadata_loading_paths(sessions: &[super::SessionEntry]) -> Vec<PathBuf> {
    sessions
        .iter()
        .filter(|entry| !entry.is_new_chat && entry.message_count.is_none())
        .map(|entry| entry.path.clone())
        .collect()
}

fn split_metadata_loading_batches(paths: Vec<PathBuf>, worker_count: usize) -> Vec<Vec<PathBuf>> {
    let mut batches = vec![Vec::new(); worker_count];
    for (index, path) in paths.into_iter().enumerate() {
        batches[index % worker_count].push(path);
    }
    batches
        .into_iter()
        .filter(|batch| !batch.is_empty())
        .collect()
}

pub(super) fn handle_slash_command(
    cmd: &str,
    _arg: &str,
    app: &mut App,
    sender: mpsc::Sender<StreamToken>,
) {
    let cmd = crate::commands::resolve_alias(cmd);
    match cmd {
        "/quit" => cmd_quit(app),
        "/clear" => cmd_clear(app),
        "/retry" => cmd_retry(app, sender),
        "/continue" => cmd_continue(app, sender),
        "/system" => cmd_system(app),
        "/config" => cmd_config(app),
        "/branch" => cmd_branch(app),
        "/persona" => cmd_persona(app),
        "/worldbook" => cmd_worldbook(app),
        "/character" => cmd_character(app),
        "/passkey" => cmd_passkey(app),
        "/report" => cmd_report(app),
        _ => {
            app.set_status(
                format!("Unknown command: {cmd}"),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_quit(app: &mut App) {
    app.should_quit = true;
}

fn cmd_clear(app: &mut App) {
    if !app.flush_session_before_transition() {
        return;
    }
    app.session.tree.clear();
    app.session.system_prompt = None;
    app.session.character = None;
    app.session.worldbooks.clear();
    app.session.persona = None;
    app.active_persona_name = None;
    app.active_persona_desc = None;
    app.discard_pending_session_save();
    app.invalidate_chat_cache();
    app.invalidate_worldbook_cache();
    app.chat_scroll = 0;
    app.auto_scroll = true;
    let new_name = session::generate_session_name();
    let new_path = crate::config::sessions_dir().join(&new_name);
    app.save_mode.set_path(new_path);
    refresh_sidebar(app);
}

fn cmd_retry(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;
    app.session.retreat_trailing_assistant();

    let last_user_content = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .filter(|n| n.message.role == Role::User)
        .map(|n| n.message.content.clone());

    match last_user_content {
        Some(content) => {
            app.session.tree.retreat_head();
            start_streaming(app, &content, sender);
        }
        None => {
            app.set_status(
                "No user message to retry.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_continue(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;

    let head_is_assistant = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .is_some_and(|n| n.message.role == Role::Assistant);

    if !head_is_assistant {
        app.set_status(
            "Cannot continue: last message is not from assistant.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }

    start_continuation(app, sender);
}

fn start_continuation(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    if app.model_name.is_none() {
        app.set_status(
            "Connecting to API server...".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }

    app.is_streaming = true;
    app.is_continuation = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt =
        super::business::build_effective_system_prompt(app.session, app.save_mode.key());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected = super::business::inject_loaded_worldbook_entries(
        app.session,
        truncated,
        user_name,
        &worldbooks,
    );
    let injected = super::business::replace_template_vars(app.session, injected, user_name);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app
        .instruct_preset
        .render_continuation(&injected_refs, effective_prompt.as_deref());
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

fn cmd_system(app: &mut App) {
    if app.cli_overrides.system_prompt.is_some() {
        let content = app
            .session
            .system_prompt
            .as_deref()
            .unwrap_or("")
            .to_owned();
        let values = vec!["(set via -r)".to_owned(), content];
        let dialog = dialogs::open_system_prompt_editor(values).with_locked_fields(vec![0, 1]);
        app.system_prompt_editor = Some(dialog);
        app.system_editor_read_only = true;
        app.system_editor_prompt_name = String::new();
        app.system_editor_return_focus = Focus::Input;
        app.focus = Focus::SystemPromptEditorDialog;
        return;
    }

    let dir = crate::config::system_prompts_dir();
    let prompts = crate::system_prompt::list_prompts(&dir, app.save_mode.key());
    if prompts.is_empty() {
        app.set_status(
            "No system prompts found.".to_owned(),
            super::StatusLevel::Warning,
        );
    } else {
        app.system_prompt_list = prompts.into_iter().map(|p| p.name).collect();
        app.system_prompt_selected = 0;
        app.focus = Focus::SystemPromptDialog;
    }
}

fn cmd_config(app: &mut App) {
    let locked = business::config_locked_fields(&app.cli_overrides);
    app.config_dialog = Some(dialogs::open_config_editor(
        load_config_fields(&crate::config::load(), &app.cli_overrides),
        locked,
    ));
    app.focus = Focus::ConfigDialog;
}

fn cmd_branch(app: &mut App) {
    let target = {
        let path_ids = app.session.tree.current_branch_ids();
        app.nav_cursor.or_else(|| {
            if path_ids.len() >= 2 {
                Some(path_ids[path_ids.len() - 2])
            } else {
                path_ids.last().copied()
            }
        })
    };

    let Some(target_id) = target else {
        app.set_status(
            "No messages to branch.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    };

    let siblings = app.session.tree.siblings_of(target_id);
    if siblings.len() <= 1 {
        app.set_status(
            "No branches at this point.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }

    const BRANCH_PREVIEW_CHARS: usize = 60;
    app.branch_dialog_items = siblings
        .iter()
        .map(|&sib_id| {
            let node = app.session.tree.node(sib_id).unwrap();
            let content = &node.message.content;
            let preview = if content.len() > BRANCH_PREVIEW_CHARS {
                let end = content[..BRANCH_PREVIEW_CHARS]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, c)| i + c.len_utf8());
                format!("{}...", &content[..end])
            } else {
                content.clone()
            };
            let preview = preview.replace('\n', " ");
            let label = format!("[{}] {}", node.message.role, preview);
            (sib_id, label)
        })
        .collect();

    let current_idx = siblings.iter().position(|&s| s == target_id).unwrap_or(0);
    app.branch_dialog_selected = current_idx;
    app.focus = Focus::BranchDialog;
}

fn cmd_persona(app: &mut App) {
    if let Some(ref persona_name) = app.cli_overrides.persona {
        let dir = crate::config::personas_dir();
        let pf = crate::persona::load_persona_by_name(&dir, persona_name, app.save_mode.key());
        let values = match pf {
            Some(pf) => vec![pf.name, pf.persona],
            None => vec![persona_name.clone(), String::new()],
        };
        let all_locked = vec![0, 1];
        app.persona_editor_file_name = persona_name.clone();
        app.persona_editor =
            Some(dialogs::open_persona_editor(values).with_locked_fields(all_locked));
        app.focus = Focus::PersonaEditorDialog;
        return;
    }

    let personas =
        crate::persona::list_personas(&crate::config::personas_dir(), app.save_mode.key());
    app.persona_list = personas.into_iter().map(|p| p.name).collect();
    app.persona_selected = 0;
    app.focus = Focus::PersonaDialog;
}

fn cmd_worldbook(app: &mut App) {
    let books =
        crate::worldinfo::list_worldbooks(&crate::config::worldinfo_dir(), app.save_mode.key());
    if books.is_empty() {
        app.set_status(
            "No worldbooks found in worldinfo/ directory.".to_owned(),
            super::StatusLevel::Warning,
        );
    } else {
        app.worldbook_list = books.into_iter().map(|b| b.name).collect();
        app.worldbook_selected = 0;
        app.focus = Focus::WorldbookDialog;
    }
}

fn cmd_character(app: &mut App) {
    let cards = crate::character::list_cards(&crate::config::characters_dir(), app.save_mode.key());
    if cards.is_empty() {
        app.set_status(
            "No characters found. Drop a .png or .json file to import.".to_owned(),
            super::StatusLevel::Warning,
        );
    } else {
        app.character_names = cards.iter().map(|c| c.name.clone()).collect();
        app.character_slugs = cards.into_iter().map(|c| c.slug).collect();
        app.character_selected = 0;
        app.focus = Focus::CharacterDialog;
    }
}

fn cmd_passkey(app: &mut App) {
    match &app.save_mode {
        SaveMode::Encrypted { .. } => {
            app.set_passkey_input.clear();
            app.set_passkey_confirm.clear();
            app.set_passkey_active_field = 0;
            app.set_passkey_error.clear();
            app.set_passkey_deriving = false;
            app.set_passkey_is_initial = false;
            app.focus = Focus::SetPasskeyDialog;
        }
        SaveMode::Plaintext(_) | SaveMode::None => {
            app.set_status(
                "Encryption is disabled for this session.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
        SaveMode::PendingPasskey(_) => {
            app.set_status(
                "Please unlock sessions first.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_report(app: &mut App) {
    if !crate::config::load().debug_log {
        app.set_status(
            "Debug logging is disabled in config".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            app.set_status(
                format!("Cannot resolve current directory: {err}"),
                super::StatusLevel::Error,
            );
            return;
        }
    };
    let output_path = current_dir.join("debug.log");
    if output_path.exists() {
        app.set_status(
            format!("Refusing to overwrite existing {}", output_path.display()),
            super::StatusLevel::Error,
        );
        return;
    }

    match crate::debug_log::copy_current_log_to(&output_path) {
        Ok(()) => app.set_status(
            format!("Debug log copied to {}", output_path.display()),
            super::StatusLevel::Info,
        ),
        Err(err) => app.set_status(
            format!("Failed to write debug report: {err}"),
            super::StatusLevel::Error,
        ),
    }
}

pub(super) fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    if app.model_name.is_none() {
        app.set_status(
            "Connecting to API server...".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }
    let parent = app.session.tree.head();
    app.session
        .tree
        .push(parent, Message::new(Role::User, content.to_owned()));
    app.mark_session_dirty(super::SaveTrigger::Debounced, false);
    app.invalidate_chat_cache();
    app.is_streaming = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt =
        super::business::build_effective_system_prompt(app.session, app.save_mode.key());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected = super::business::inject_loaded_worldbook_entries(
        app.session,
        truncated,
        user_name,
        &worldbooks,
    );
    let injected = super::business::replace_template_vars(app.session, injected, user_name);
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

pub(super) fn handle_stream_token(token: StreamToken, app: &mut App) -> Result<()> {
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
            app.mark_session_dirty(super::SaveTrigger::StreamDone, true);
            app.invalidate_chat_cache();
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.flush_session_save(super::SaveTrigger::StreamDone)?;
            refresh_sidebar(app);
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.is_continuation = false;
            app.set_status(format!("Error: {err}"), super::StatusLevel::Error);
        }
    }
    Ok(())
}

pub(super) fn handle_background_event(event: super::BackgroundEvent, app: &mut App) {
    match event {
        super::BackgroundEvent::KeyDerived(key, path) => {
            if let Some(debug) = app.unlock_debug.take() {
                crate::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        crate::debug_log::field("phase", "ui_complete"),
                        crate::debug_log::field("kind", debug.kind),
                        crate::debug_log::field("result", "ok"),
                        crate::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
                );
            }
            app.save_mode = SaveMode::Encrypted {
                path,
                key: key.clone(),
            };
            if let Err(err) = app.flush_session_save(super::SaveTrigger::Unlock) {
                app.set_status(format!("Save error: {err}"), super::StatusLevel::Error);
            }
            app.invalidate_worldbook_cache();
            app.passkey_deriving = false;
            app.focus = post_passkey_focus(app);
            refresh_sidebar(app);
            maintenance::spawn_unlocked_maintenance(key, &app.bg_tx);
        }
        super::BackgroundEvent::KeyDeriveFailed(err) => {
            if let Some(debug) = app.unlock_debug.take() {
                crate::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        crate::debug_log::field("phase", "ui_complete"),
                        crate::debug_log::field("kind", debug.kind),
                        crate::debug_log::field("result", "error"),
                        crate::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        crate::debug_log::field("error", &err),
                    ],
                );
            }
            app.passkey_deriving = false;
            app.passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::PasskeySet(new_key) => {
            if let Some(debug) = app.unlock_debug.take() {
                crate::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        crate::debug_log::field("phase", "ui_complete"),
                        crate::debug_log::field("kind", debug.kind),
                        crate::debug_log::field("result", "ok"),
                        crate::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
                );
            }
            app.set_passkey_deriving = false;
            app.invalidate_worldbook_cache();
            if app.set_passkey_is_initial {
                let path = match &app.save_mode {
                    SaveMode::PendingPasskey(p) => p.clone(),
                    _ => crate::config::sessions_dir().join(session::generate_session_name()),
                };
                app.save_mode = SaveMode::Encrypted {
                    path,
                    key: new_key.clone(),
                };
                if let Err(err) = app.flush_session_save(super::SaveTrigger::Unlock) {
                    app.set_status(format!("Save error: {err}"), super::StatusLevel::Error);
                }
                app.focus = post_passkey_focus(app);
                refresh_sidebar(app);
                maintenance::spawn_unlocked_maintenance(new_key, &app.bg_tx);
            } else {
                let old_key = match &app.save_mode {
                    SaveMode::Encrypted { key, .. } => key.clone(),
                    _ => {
                        app.set_status(
                            "No existing key to re-encrypt from.".to_owned(),
                            super::StatusLevel::Error,
                        );
                        return;
                    }
                };
                app.re_encrypt_old_key = Some(old_key.clone());
                app.save_mode = match &app.save_mode {
                    SaveMode::Encrypted { path, .. } => SaveMode::Encrypted {
                        path: path.clone(),
                        key: new_key.clone(),
                    },
                    _ => return,
                };
                app.mark_session_dirty(super::SaveTrigger::Explicit, true);
                match app.flush_session_save(super::SaveTrigger::Explicit) {
                    Ok(()) => app.discard_pending_session_save(),
                    Err(err) => {
                        app.set_status(format!("Save error: {err}"), super::StatusLevel::Error);
                        return;
                    }
                }
                let current_session_path = match &app.save_mode {
                    SaveMode::Encrypted { path, .. } => Some(path.clone()),
                    _ => None,
                };
                app.focus = Focus::Input;
                app.set_status(
                    "Re-encrypting files...".to_owned(),
                    super::StatusLevel::Info,
                );

                let bg_tx = app.bg_tx.clone();
                tokio::spawn(async move {
                    let warnings = match tokio::task::spawn_blocking(move || {
                        let mut warnings = Vec::new();
                        warnings.extend(crate::crypto::re_encrypt_directory_excluding(
                            &crate::config::sessions_dir(),
                            &["session"],
                            &old_key,
                            &new_key,
                            current_session_path.as_deref(),
                        ));
                        warnings.extend(crate::crypto::re_encrypt_directory(
                            &crate::config::characters_dir(),
                            &["character"],
                            &old_key,
                            &new_key,
                        ));
                        warnings.extend(crate::crypto::re_encrypt_directory(
                            &crate::config::worldinfo_dir(),
                            &["worldbook"],
                            &old_key,
                            &new_key,
                        ));
                        warnings.extend(crate::crypto::re_encrypt_directory(
                            &crate::config::system_prompts_dir(),
                            &["prompt"],
                            &old_key,
                            &new_key,
                        ));
                        warnings.extend(crate::crypto::re_encrypt_directory(
                            &crate::config::personas_dir(),
                            &["persona"],
                            &old_key,
                            &new_key,
                        ));
                        if let Err(e) = crate::crypto::re_encrypt_file(
                            &crate::config::index_path(),
                            &old_key,
                            &new_key,
                        ) {
                            warnings.push(format!("index.meta: {e}"));
                        }
                        warnings
                    })
                    .await
                    {
                        Ok(warnings) => warnings,
                        Err(err) => vec![format!("re-encryption task failed: {err}")],
                    };
                    let _ = bg_tx
                        .send(super::BackgroundEvent::ReEncryptionComplete(warnings))
                        .await;
                });
            }
        }
        super::BackgroundEvent::PasskeySetFailed(err) => {
            if let Some(debug) = app.unlock_debug.take() {
                crate::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        crate::debug_log::field("phase", "ui_complete"),
                        crate::debug_log::field("kind", debug.kind),
                        crate::debug_log::field("result", "error"),
                        crate::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        crate::debug_log::field("error", &err),
                    ],
                );
            }
            app.set_passkey_deriving = false;
            app.set_passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::ReEncryptionComplete(warnings) => {
            if warnings.is_empty() {
                let check_path = crate::config::key_check_path();
                if let SaveMode::Encrypted { key, .. } = &app.save_mode {
                    if let Err(err) = crate::crypto::set_key_fingerprint(&check_path, key) {
                        app.set_status(
                            format!("Failed to update key fingerprint: {err}"),
                            super::StatusLevel::Error,
                        );
                        return;
                    }
                }
                app.re_encrypt_old_key = None;
                app.passkey_changed = true;
                app.should_quit = true;
            } else {
                if let Some(old_key) = app.re_encrypt_old_key.take() {
                    app.save_mode = match &app.save_mode {
                        SaveMode::Encrypted { path, .. } => SaveMode::Encrypted {
                            path: path.clone(),
                            key: old_key,
                        },
                        other => other.clone(),
                    };
                }
                for warning in &warnings {
                    app.set_status(warning.clone(), super::StatusLevel::Error);
                }
                app.set_status(
                    format!(
                        "Re-encryption failed for {} file(s). Passkey NOT changed.",
                        warnings.len()
                    ),
                    super::StatusLevel::Error,
                );
            }
        }
        super::BackgroundEvent::MetadataLoaded {
            generation,
            path,
            metadata,
        } => {
            if generation != app.sidebar_hydration_generation {
                if let Some(debug) = app.hydration_debug.as_mut() {
                    if debug.generation == generation {
                        debug.stale_dropped += 1;
                    }
                }
                crate::debug_log::log_kv(
                    "sidebar.hydration",
                    &[
                        crate::debug_log::field("phase", "apply"),
                        crate::debug_log::field("result", "stale_drop"),
                        crate::debug_log::field("generation", generation),
                        crate::debug_log::field(
                            "current_generation",
                            app.sidebar_hydration_generation,
                        ),
                        crate::debug_log::field("path", path.display()),
                    ],
                );
                return;
            }

            let Some(entry) = app.sidebar_sessions.iter_mut().find(|e| e.path == path) else {
                if let Some(debug) = app.hydration_debug.as_mut() {
                    if debug.generation == generation {
                        debug.missing_dropped += 1;
                    }
                }
                crate::debug_log::log_kv(
                    "sidebar.hydration",
                    &[
                        crate::debug_log::field("phase", "apply"),
                        crate::debug_log::field("result", "missing_drop"),
                        crate::debug_log::field("generation", generation),
                        crate::debug_log::field("path", path.display()),
                    ],
                );
                return;
            };

            if let Some(character) = &metadata.character {
                entry.display_name = character.clone();
            }
            entry.message_count = Some(metadata.message_count);
            entry.last_assistant_preview = metadata.last_assistant_preview.clone();

            session::persist_loaded_metadata_index(
                &path,
                &metadata,
                crate::index::SessionStorageMode::Encrypted,
                app.save_mode.key(),
            );
            super::business::prepare_sidebar_entries(&mut app.sidebar_sessions);
            app.invalidate_sidebar_cache();
        }
        super::BackgroundEvent::MetadataBatchFinished {
            generation: _generation,
            loaded_count: _loaded_count,
            failed_count: _failed_count,
        } => {
            if let Some(debug) = app.hydration_debug.as_mut() {
                if debug.generation == _generation {
                    debug.completed += _loaded_count;
                    debug.failed += _failed_count;
                    debug.batch_finished += 1;
                    if debug.batch_finished == debug.batch_total {
                        crate::debug_log::log_kv(
                            "sidebar.hydration",
                            &[
                                crate::debug_log::field("phase", "complete"),
                                crate::debug_log::field("generation", debug.generation),
                                crate::debug_log::field("scheduled", debug.scheduled),
                                crate::debug_log::field("loaded", debug.completed),
                                crate::debug_log::field("failed", debug.failed),
                                crate::debug_log::field("stale_dropped", debug.stale_dropped),
                                crate::debug_log::field("missing_dropped", debug.missing_dropped),
                                crate::debug_log::field(
                                    "elapsed_ms",
                                    format!(
                                        "{:.3}",
                                        debug.started_at.elapsed().as_secs_f64() * 1000.0
                                    ),
                                ),
                            ],
                        );
                        app.hydration_debug = None;
                    }
                }
            }
        }
        super::BackgroundEvent::MaintenanceFinished(update) => {
            maintenance::handle_finished(update, app);
        }
        super::BackgroundEvent::ModelFetched(Ok(name)) => {
            app.model_name = Some(name);
            if app.focus == Focus::LoadingDialog {
                app.focus = Focus::Input;
            }
        }
        super::BackgroundEvent::ModelFetched(Err(err)) => {
            app.model_name = Some("Backend connection failure".to_owned());
            app.api_available = false;
            app.api_error = err;
            match app.focus {
                Focus::PasskeyDialog | Focus::SetPasskeyDialog => {}
                _ => {
                    app.focus = Focus::ApiErrorDialog;
                }
            }
        }
    }
}
