use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::client::StreamToken;
use crate::session::{self, Message, Role, SaveMode};

use super::business::{load_config_fields, load_self_fields, refresh_sidebar};
use super::dialogs::FieldDialog;
use super::{App, Focus, CONFIG_FIELDS, SELF_FIELDS};

fn post_passkey_focus(app: &App) -> Focus {
    if app.model_name.is_none() {
        Focus::LoadingDialog
    } else if !app.api_available {
        Focus::ApiErrorDialog
    } else {
        Focus::Input
    }
}

pub fn spawn_metadata_loading(
    sessions: &[super::SessionEntry],
    key: &std::sync::Arc<crate::crypto::DerivedKey>,
    bg_tx: &mpsc::Sender<super::BackgroundEvent>,
) {
    for entry in sessions {
        if entry.is_new_chat || entry.message_count.is_some() {
            continue;
        }
        let entry_path = entry.path.clone();
        let key = key.clone();
        let tx = bg_tx.clone();
        tokio::spawn(async move {
            let path_for_event = entry_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                session::load_metadata(&entry_path, &key)
            })
            .await;
            if let Ok(Some(metadata)) = result {
                let _ = tx
                    .send(super::BackgroundEvent::MetadataLoaded {
                        path: path_for_event,
                        metadata,
                    })
                    .await;
            }
        });
    }
}

pub fn handle_slash_command(cmd: &str, arg: &str, app: &mut App, sender: mpsc::Sender<StreamToken>) {
    let cmd = crate::commands::resolve_alias(cmd);
    match cmd {
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
                    app.set_status("No user message to retry.".to_owned(), super::StatusLevel::Warning);
                }
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
                super::configure_textarea(&mut editor);
                app.system_editor = Some(editor);
                app.system_editor_roleplay = is_roleplay;
                app.focus = Focus::SystemDialog;
            } else {
                app.session.system_prompt = Some(arg.to_owned());
                app.set_status("System prompt updated.".to_owned(), super::StatusLevel::Info);
                let _ = app.session.maybe_save(&app.save_mode);
            }
        }
        "/save" => {
            if arg.is_empty() {
                match app.session.maybe_save(&app.save_mode) {
                    Ok(()) => match app.save_mode.path() {
                        Some(p) => app.set_status(format!("Saved to {}.", p.display()), super::StatusLevel::Info),
                        None => app.set_status("No session path set.".to_owned(), super::StatusLevel::Warning),
                    },
                    Err(e) => app.set_status(format!("Save error: {e}"), super::StatusLevel::Error),
                }
            } else {
                let path = PathBuf::from(arg);
                match session::save(&path, app.session) {
                    Ok(()) => app.set_status(format!("Saved to {arg}."), super::StatusLevel::Info),
                    Err(e) => app.set_status(format!("Save error: {e}"), super::StatusLevel::Error),
                }
            }
        }
        "/config" => {
            app.config_dialog = Some(FieldDialog::new(
                " Configuration ",
                CONFIG_FIELDS,
                load_config_fields(&crate::config::load()),
                &[],
            ));
            app.focus = Focus::ConfigDialog;
        }
        "/load" => {
            if arg.is_empty() {
                app.set_status("Usage: /load <path>".to_owned(), super::StatusLevel::Warning);
            } else {
                let path = PathBuf::from(arg);
                match session::load(&path) {
                    Ok(loaded) => {
                        *app.session = loaded;
                        let count = app.session.tree.branch_path().len();
                        app.set_status(format!("Loaded from {arg} ({count} messages)."), super::StatusLevel::Info);
                        app.auto_scroll = true;
                    }
                    Err(e) => app.set_status(format!("Load error: {e}"), super::StatusLevel::Error),
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
                app.set_status("No messages to branch.".to_owned(), super::StatusLevel::Warning);
                return;
            };

            let siblings = app.session.tree.siblings_of(target_id);
            if siblings.len() <= 1 {
                app.set_status("No branches at this point.".to_owned(), super::StatusLevel::Warning);
                return;
            }

            const BRANCH_PREVIEW_CHARS: usize = 60;
            app.branch_dialog_items = siblings
                .iter()
                .map(|&sib_id| {
                    let node = app.session.tree.node(sib_id).unwrap();
                    let content = &node.message.content;
                    let preview = if content.len() > BRANCH_PREVIEW_CHARS {
                        format!("{}...", &content[..BRANCH_PREVIEW_CHARS])
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
                load_self_fields(&crate::config::load()),
                &[1],
            ));
            app.focus = Focus::SelfDialog;
        }
        "/worldbook" => {
            let books =
                crate::worldinfo::list_worldbooks(&crate::config::worldinfo_dir(), app.save_mode.key());
            if books.is_empty() {
                app.set_status("No worldbooks found in worldinfo/ directory.".to_owned(), super::StatusLevel::Warning);
            } else {
                app.worldbook_list =
                    books.into_iter().map(|b| b.name).collect();
                app.worldbook_selected = 0;
                app.focus = Focus::WorldbookDialog;
            }
        }
        "/character" => {
            if arg.starts_with("import") {
                let path_str = arg.strip_prefix("import").unwrap_or("").trim();
                if path_str.is_empty() {
                    app.set_status("Usage: /character import <path>".to_owned(), super::StatusLevel::Warning);
                } else {
                    let source = std::path::Path::new(path_str);
                    match crate::character::import_card(source) {
                        Ok(card) => {
                            let name = card.name.clone();
                            match crate::character::save_card(
                                &card,
                                &crate::config::characters_dir(),
                                app.save_mode.key(),
                            ) {
                                Ok(_) => {
                                    app.set_status(format!("Imported character: {name}"), super::StatusLevel::Info);
                                }
                                Err(e) => {
                                    app.set_status(format!("Save error: {e}"), super::StatusLevel::Error);
                                }
                            }
                        }
                        Err(e) => app.set_status(format!("Import error: {e}"), super::StatusLevel::Error),
                    }
                }
            } else {
                let cards =
                    crate::character::list_cards(&crate::config::characters_dir(), app.save_mode.key());
                if cards.is_empty() {
                    app.set_status("No characters found. Use /character import <path>".to_owned(), super::StatusLevel::Warning);
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
        "/passkey" => {
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
                    app.set_status("Encryption is disabled for this session.".to_owned(), super::StatusLevel::Warning);
                }
                SaveMode::PendingPasskey(_) => {
                    app.set_status("Please unlock sessions first.".to_owned(), super::StatusLevel::Warning);
                }
            }
        }
        _ => {
            app.set_status(format!("Unknown command: {cmd}"), super::StatusLevel::Warning);
        }
    }
}

pub fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    if app.model_name.is_none() {
        app.set_status("Connecting to API server...".to_owned(), super::StatusLevel::Warning);
        return;
    }
    if !app.api_available {
        app.set_status("Cannot send: API server is not available".to_owned(), super::StatusLevel::Error);
        return;
    }
    let parent = app.session.tree.head();
    app.session
        .tree
        .push(parent, Message::new(Role::User, content.to_owned()));
    app.is_streaming = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt = super::business::build_effective_system_prompt(app.session, &app.config);
    let injected = super::business::inject_worldbook_entries(app.session, truncated, &app.config, app.save_mode.key());
    let injected = super::business::replace_template_vars(app.session, injected, &app.config);
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
            app.session.maybe_save(&app.save_mode)?;
            refresh_sidebar(app);
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.set_status(format!("Error: {err}"), super::StatusLevel::Error);
        }
    }
    Ok(())
}

pub fn handle_background_event(
    event: super::BackgroundEvent,
    app: &mut App,
) {
    match event {
        super::BackgroundEvent::KeyDerived(key, path) => {
            app.save_mode = SaveMode::Encrypted {
                path,
                key: key.clone(),
            };
            for warning in crate::character::auto_import_png_cards(
                &crate::config::characters_dir(), Some(&key),
            ) {
                app.set_status(warning, super::StatusLevel::Warning);
            }
            for warning in crate::worldinfo::normalize_worldbooks(
                &crate::config::worldinfo_dir(), Some(&key),
            ) {
                app.set_status(warning, super::StatusLevel::Warning);
            }
            for warning in crate::character::encrypt_plaintext_cards(
                &crate::config::characters_dir(), &key,
            ) {
                app.set_status(warning, super::StatusLevel::Warning);
            }
            app.passkey_deriving = false;
            app.focus = post_passkey_focus(app);
            refresh_sidebar(app);
        }
        super::BackgroundEvent::KeyDeriveFailed(err) => {
            app.passkey_deriving = false;
            app.passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::PasskeySet(new_key) => {
            app.set_passkey_deriving = false;
            if app.set_passkey_is_initial {
                let path = match &app.save_mode {
                    SaveMode::PendingPasskey(p) => p.clone(),
                    _ => crate::config::sessions_dir().join(session::generate_session_name()),
                };
                app.save_mode = SaveMode::Encrypted {
                    path,
                    key: new_key.clone(),
                };
                for warning in crate::character::auto_import_png_cards(
                    &crate::config::characters_dir(), Some(&new_key),
                ) {
                    app.set_status(warning, super::StatusLevel::Warning);
                }
                for warning in crate::worldinfo::normalize_worldbooks(
                    &crate::config::worldinfo_dir(), Some(&new_key),
                ) {
                    app.set_status(warning, super::StatusLevel::Warning);
                }
                for warning in crate::character::encrypt_plaintext_cards(
                    &crate::config::characters_dir(), &new_key,
                ) {
                    app.set_status(warning, super::StatusLevel::Warning);
                }
                app.focus = post_passkey_focus(app);
                refresh_sidebar(app);
                spawn_metadata_loading(&app.sidebar_sessions, &new_key, &app.bg_tx);
            } else {
                let old_key = match &app.save_mode {
                    SaveMode::Encrypted { key, .. } => key.clone(),
                    _ => {
                        app.set_status("No existing key to re-encrypt from.".to_owned(), super::StatusLevel::Error);
                        return;
                    }
                };
                app.save_mode = match &app.save_mode {
                    SaveMode::Encrypted { path, .. } => SaveMode::Encrypted {
                        path: path.clone(),
                        key: new_key.clone(),
                    },
                    _ => return,
                };
                let _ = app.session.maybe_save(&app.save_mode);
                app.focus = Focus::Input;
                app.set_status("Re-encrypting files...".to_owned(), super::StatusLevel::Info);

                let bg_tx = app.bg_tx.clone();
                tokio::spawn(async move {
                    let mut warnings = Vec::new();
                    warnings.extend(crate::crypto::re_encrypt_directory(
                        &crate::config::sessions_dir(),
                        &["session"],
                        &old_key,
                        &new_key,
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
                    let _ = bg_tx.send(super::BackgroundEvent::ReEncryptionComplete(warnings)).await;
                });
            }
        }
        super::BackgroundEvent::PasskeySetFailed(err) => {
            app.set_passkey_deriving = false;
            app.set_passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::ReEncryptionComplete(_warnings) => {
            app.passkey_changed = true;
            app.should_quit = true;
        }
        super::BackgroundEvent::MetadataLoaded { path, metadata } => {
            if let Some(entry) = app.sidebar_sessions.iter_mut().find(|e| e.path == path) {
                if let Some(character) = metadata.character {
                    entry.display_name = character;
                }
                entry.message_count = Some(metadata.message_count);
                entry.first_message = metadata.first_message;
            }
        }
        super::BackgroundEvent::ModelFetched(Ok(name)) => {
            app.model_name = Some(name);
            if app.focus == Focus::LoadingDialog {
                app.focus = Focus::Input;
            }
        }
        super::BackgroundEvent::ModelFetched(Err(err)) => {
            app.model_name = Some("unknown".to_owned());
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
