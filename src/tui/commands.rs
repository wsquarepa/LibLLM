use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::client::StreamToken;
use crate::session::{self, Message, Role, SaveMode};

use super::business::{load_config_fields, load_self_fields, refresh_sidebar};
use super::dialogs::FieldDialog;
use super::{App, Focus, CONFIG_FIELDS, SELF_FIELDS};

pub fn handle_slash_command(cmd: &str, arg: &str, app: &mut App, sender: mpsc::Sender<StreamToken>) {
    let cmd = crate::commands::resolve_alias(cmd);
    match cmd {
        "/help" => {
            app.status_message = "Use Tab to complete commands, Up/Down to navigate.".to_owned();
        }
        "/quit" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.session.tree.clear();
            app.session.system_prompt = None;
            app.session.character = None;
            app.session.worldbooks.clear();
            app.chat_scroll = 0;
            app.auto_scroll = true;
            let new_name = session::generate_session_name();
            let new_path = crate::config::sessions_dir().join(&new_name);
            app.save_mode.set_path(new_path);
            app.status_message = "New conversation started.".to_owned();
            refresh_sidebar(app);
        }
        "/retry" => {
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
                    app.status_message = "No user message to retry.".to_owned();
                }
            }
        }
        "/edit" => {
            app.nav_cursor = None;
            if arg.is_empty() {
                super::open_edit_dialog(app);
            } else {
                app.session.retreat_trailing_assistant();
                if app
                    .session
                    .tree
                    .head()
                    .and_then(|id| app.session.tree.node(id))
                    .is_some_and(|n| n.message.role == Role::User)
                {
                    app.session.tree.retreat_head();
                }
                start_streaming(app, arg, sender);
            }
        }
        "/system" => {
            if arg.is_empty() {
                let cfg = crate::config::load();
                let is_roleplay = app.session.character.is_some();
                let content = if is_roleplay {
                    cfg.roleplay_system_prompt.unwrap_or_default()
                } else {
                    cfg.system_prompt.unwrap_or_default()
                };
                let mut editor = tui_textarea::TextArea::from(
                    content.lines().map(String::from).collect::<Vec<_>>(),
                );
                editor.set_cursor_line_style(ratatui::style::Style::default());
                editor.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);
                app.system_editor = Some(editor);
                app.system_editor_roleplay = is_roleplay;
                app.focus = Focus::SystemDialog;
            } else {
                app.session.system_prompt = Some(arg.to_owned());
                app.status_message = "System prompt updated.".to_owned();
                let _ = app.session.maybe_save(&app.save_mode);
            }
        }
        "/save" => {
            if arg.is_empty() {
                match app.session.maybe_save(&app.save_mode) {
                    Ok(()) => match app.save_mode.path() {
                        Some(p) => app.status_message = format!("Saved to {}.", p.display()),
                        None => app.status_message = "No session path set.".to_owned(),
                    },
                    Err(e) => app.status_message = format!("Save error: {e}"),
                }
            } else {
                let path = PathBuf::from(arg);
                match session::save(&path, app.session) {
                    Ok(()) => app.status_message = format!("Saved to {arg}."),
                    Err(e) => app.status_message = format!("Save error: {e}"),
                }
            }
        }
        "/model" => {
            app.status_message = format!("Model: {}", app.model_name);
        }
        "/config" => {
            app.config_dialog = Some(FieldDialog::new(
                " Configuration ",
                CONFIG_FIELDS,
                load_config_fields(),
                &[],
            ));
            app.focus = Focus::ConfigDialog;
        }
        "/load" => {
            if arg.is_empty() {
                app.status_message = "Usage: /load <path>".to_owned();
            } else {
                let path = PathBuf::from(arg);
                match session::load(&path) {
                    Ok(loaded) => {
                        *app.session = loaded;
                        let count = app.session.tree.branch_path().len();
                        app.status_message = format!("Loaded from {arg} ({count} messages).");
                        app.auto_scroll = true;
                    }
                    Err(e) => app.status_message = format!("Load error: {e}"),
                }
            }
        }
        "/branch" => {
            let path_ids = app.session.tree.branch_path_ids();
            let target = app.nav_cursor.or_else(|| {
                if path_ids.len() >= 2 {
                    Some(path_ids[path_ids.len() - 2])
                } else {
                    path_ids.last().copied()
                }
            });

            let Some(target_id) = target else {
                app.status_message = "No messages to branch.".to_owned();
                return;
            };

            let siblings = app.session.tree.siblings_of(target_id);
            if siblings.len() <= 1 {
                app.status_message = "No branches at this point.".to_owned();
                return;
            }

            let max_preview = 60;
            app.branch_dialog_items = siblings
                .iter()
                .map(|&sib_id| {
                    let node = app.session.tree.node(sib_id).unwrap();
                    let content = &node.message.content;
                    let preview = if content.len() > max_preview {
                        format!("{}...", &content[..max_preview])
                    } else {
                        content.clone()
                    };
                    let preview = preview.replace('\n', " ");
                    let label = format!("[{}] {}", node.message.role, preview);
                    (sib_id, label)
                })
                .collect();

            let current_idx = siblings
                .iter()
                .position(|&s| s == target_id)
                .unwrap_or(0);
            app.branch_dialog_selected = current_idx;
            app.focus = Focus::BranchDialog;
        }
        "/self" => {
            app.self_dialog = Some(FieldDialog::new(
                " User Persona ",
                SELF_FIELDS,
                load_self_fields(),
                &[1],
            ));
            app.focus = Focus::SelfDialog;
        }
        "/worldbook" => {
            if app.session.character.is_none() {
                app.status_message =
                    "Worldbooks are only available in character sessions.".to_owned();
            } else {
                let books =
                    crate::worldinfo::list_worldbooks(&crate::config::worldinfo_dir());
                if books.is_empty() {
                    app.status_message =
                        "No worldbooks found in worldinfo/ directory.".to_owned();
                } else {
                    app.worldbook_list =
                        books.into_iter().map(|b| b.name).collect();
                    app.worldbook_selected = 0;
                    app.focus = Focus::WorldbookDialog;
                }
            }
        }
        "/character" => {
            if arg.starts_with("import") {
                let path_str = arg.strip_prefix("import").unwrap_or("").trim();
                if path_str.is_empty() {
                    app.status_message = "Usage: /character import <path>".to_owned();
                } else {
                    let source = std::path::Path::new(path_str);
                    match crate::character::import_card(source) {
                        Ok(card) => {
                            let name = card.name.clone();
                            match crate::character::save_card(
                                &card,
                                &crate::config::characters_dir(),
                            ) {
                                Ok(_) => {
                                    app.status_message =
                                        format!("Imported character: {name}")
                                }
                                Err(e) => {
                                    app.status_message = format!("Save error: {e}")
                                }
                            }
                        }
                        Err(e) => app.status_message = format!("Import error: {e}"),
                    }
                }
            } else {
                let cards =
                    crate::character::list_cards(&crate::config::characters_dir());
                if cards.is_empty() {
                    app.status_message =
                        "No characters found. Use /character import <path>".to_owned();
                } else {
                    app.character_names =
                        cards.iter().map(|c| c.name.clone()).collect();
                    app.character_slugs =
                        cards.into_iter().map(|c| c.slug).collect();
                    app.character_selected = 0;
                    app.focus = Focus::CharacterDialog;
                }
            }
        }
        _ => {
            app.status_message = format!("Unknown command: {cmd}");
        }
    }
}

pub fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    let parent = app.session.tree.head();
    app.session
        .tree
        .push(parent, Message::new(Role::User, content.to_owned()));
    app.is_streaming = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;
    app.status_message = "Generating... (Esc: cancel)".to_owned();

    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt = super::business::build_effective_system_prompt(app.session);
    let injected = super::business::inject_worldbook_entries(app.session, truncated);
    let injected = super::business::replace_template_vars(app.session, injected);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app
        .template
        .render(&injected_refs, effective_prompt.as_deref());
    let stop_tokens = app.stop_tokens;
    let sampling = app.sampling.clone();

    let client = app.client.clone();
    tokio::spawn(async move {
        client
            .stream_completion_to_channel(&prompt, stop_tokens, &sampling, sender)
            .await;
    });
}

pub fn handle_stream_token(token: StreamToken, app: &mut App) -> Result<()> {
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
            app.session
                .tree
                .push(Some(head), Message::new(Role::Assistant, full_response));
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.status_message.clear();
            app.session.maybe_save(&app.save_mode)?;
            refresh_sidebar(app);
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.status_message = format!("Error: {err}");
        }
    }
    Ok(())
}

pub fn handle_background_event(
    event: super::BackgroundEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<super::BackgroundEvent>,
) {
    match event {
        super::BackgroundEvent::KeyDerived(key, path) => {
            app.save_mode = SaveMode::Encrypted {
                path,
                key: key.clone(),
            };
            app.focus = Focus::Input;
            app.status_message.clear();

            let sessions_dir = crate::config::sessions_dir();
            let mut sessions = session::list_session_paths(&sessions_dir);
            sessions.insert(0, super::business::new_chat_entry());
            app.sidebar_sessions = sessions;
            app.sidebar_state.select(Some(0));

            for i in 0..app.sidebar_sessions.len() {
                if app.sidebar_sessions[i].is_new_chat {
                    continue;
                }
                let entry_path = app.sidebar_sessions[i].path.clone();
                let key = key.clone();
                let tx = bg_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        session::load_metadata(&entry_path, &key)
                    }).await;
                    if let Ok(Some(metadata)) = result {
                        let _ = tx
                            .send(super::BackgroundEvent::MetadataLoaded {
                                index: i,
                                metadata,
                            })
                            .await;
                    }
                });
            }
        }
        super::BackgroundEvent::KeyDeriveFailed(err) => {
            app.passkey_error = format!("Failed: {err}");
            app.status_message.clear();
        }
        super::BackgroundEvent::MetadataLoaded { index, metadata } => {
            if index < app.sidebar_sessions.len() {
                let entry = &mut app.sidebar_sessions[index];
                if let Some(character) = metadata.character {
                    entry.display_name = character;
                }
                entry.message_count = Some(metadata.message_count);
                entry.first_message = metadata.first_message;
            }
        }
    }
}
