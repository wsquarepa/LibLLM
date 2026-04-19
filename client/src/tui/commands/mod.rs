//! Slash command execution and dispatch for TUI chat actions.

pub mod macros;
pub use macros::expand_macro;

pub mod background;
pub mod export;
pub mod streaming;

pub(super) use background::handle_background_event;
pub(super) use streaming::{handle_stream_token, start_streaming};

use tokio::sync::mpsc;

use libllm::client::StreamToken;
use libllm::session::{self, Message, Role, SaveMode};

use super::business::{self, refresh_sidebar};
use super::{App, Focus, StatusLevel, dialogs};

pub(super) fn handle_slash_command(
    cmd: &str,
    arg: &str,
    app: &mut App,
    sender: mpsc::Sender<StreamToken>,
) {
    let cmd = libllm::commands::resolve_alias(cmd);
    tracing::debug!(cmd, has_arg = !arg.is_empty(), "tui.command");
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
        "/theme" => cmd_theme(app, arg),
        "/export" => export::cmd_export(app, arg),
        "/macro" => cmd_macro(app, arg, sender),
        "/report" => cmd_report(app),
        _ => {
            tracing::debug!(cmd, result = "unknown", "tui.command");
            app.set_status(
                format!("Unknown command: {cmd}"),
                StatusLevel::Warning,
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
    let new_id = session::generate_session_id();
    app.save_mode.set_id(new_id);
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
            streaming::start_streaming(app, &content, sender);
        }
        None => {
            app.set_status(
                "No user message to retry.".to_owned(),
                StatusLevel::Warning,
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
            StatusLevel::Warning,
        );
        return;
    }

    start_continuation(app, sender);
}

fn start_continuation(app: &mut App, sender: mpsc::Sender<StreamToken>) {
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

    app.is_streaming = true;
    app.is_continuation = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = streaming::loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt =
        super::business::build_effective_system_prompt(app.session, app.db.as_ref());
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

    let prompts = app.db.as_ref().and_then(|db| db.list_prompts().ok()).unwrap_or_default();
    if prompts.is_empty() {
        app.set_status(
            "No system prompts found.".to_owned(),
            StatusLevel::Warning,
        );
    } else {
        app.system_prompt_list = prompts.into_iter().map(|e| e.name).collect();
        app.system_prompt_selected = 0;
        app.open_paged_dialog(Focus::SystemPromptDialog);
    }
}

fn cmd_config(app: &mut App) {
    let cfg = libllm::config::load();
    let sections = business::load_tabbed_config_sections(&cfg, &app.cli_overrides);
    let locked = business::config_locked_fields_by_section(&app.cli_overrides);
    app.config_dialog = Some(dialogs::open_config_editor(sections, locked));
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
            StatusLevel::Warning,
        );
        return;
    };

    let siblings = app.session.tree.siblings_of(target_id);
    if siblings.len() <= 1 {
        app.set_status(
            "No branches at this point.".to_owned(),
            StatusLevel::Warning,
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
    app.open_paged_dialog(Focus::BranchDialog);
}

fn cmd_persona(app: &mut App) {
    if let Some(ref persona_slug) = app.cli_overrides.persona {
        let pf = app.db.as_ref().and_then(|db| db.load_persona(persona_slug).ok());
        let values = match pf {
            Some(pf) => vec![pf.name, pf.persona],
            None => vec![persona_slug.clone(), String::new()],
        };
        let all_locked = vec![0, 1];
        app.persona_editor_slug = persona_slug.clone();
        app.persona_editor =
            Some(dialogs::open_persona_editor(values).with_locked_fields(all_locked));
        app.focus = Focus::PersonaEditorDialog;
        return;
    }

    let personas = app.db.as_ref().and_then(|db| db.list_personas().ok()).unwrap_or_default();
    app.persona_names = personas.iter().map(|(_, name)| name.clone()).collect();
    app.persona_slugs = personas.into_iter().map(|(slug, _)| slug).collect();
    app.persona_selected = 0;
    app.open_paged_dialog(Focus::PersonaDialog);
}

fn cmd_worldbook(app: &mut App) {
    let books = app.db.as_ref().and_then(|db| db.list_worldbooks().ok()).unwrap_or_default();
    app.worldbook_list = books.into_iter().map(|(_, name)| name).collect();
    app.worldbook_selected = 0;
    app.open_paged_dialog(Focus::WorldbookDialog);
}

fn cmd_character(app: &mut App) {
    let chars = app.db.as_ref().and_then(|db| db.list_characters().ok()).unwrap_or_default();
    app.character_names = chars.iter().map(|(_, name)| name.clone()).collect();
    app.character_slugs = chars.into_iter().map(|(slug, _)| slug).collect();
    app.character_selected = 0;
    app.open_paged_dialog(Focus::CharacterDialog);
}

fn cmd_passkey(app: &mut App) {
    match &app.save_mode {
        SaveMode::Database { .. } => {
            if app.db.is_some() {
                app.set_passkey_input.clear();
                app.set_passkey_confirm.clear();
                app.set_passkey_active_field = 0;
                app.set_passkey_error.clear();
                app.set_passkey_deriving = false;
                app.set_passkey_is_initial = false;
                app.focus = Focus::SetPasskeyDialog;
            } else {
                app.set_status(
                    "Database not available.".to_owned(),
                    StatusLevel::Error,
                );
            }
        }
        SaveMode::None => {
            app.set_status(
                "Encryption is disabled for this session.".to_owned(),
                StatusLevel::Warning,
            );
        }
        SaveMode::PendingPasskey { .. } => {
            app.set_status(
                "Please unlock sessions first.".to_owned(),
                StatusLevel::Warning,
            );
        }
    }
}

fn cmd_theme(app: &mut App, arg: &str) {
    let arg = arg.trim();
    if arg.is_empty() {
        let cfg = libllm::config::load();
        app.theme_dialog = Some(dialogs::open_theme_editor(&cfg));
        app.focus = Focus::ThemeDialog;
        return;
    }

    if super::theme::Theme::from_name(arg).is_none() {
        let available = super::theme::Theme::available_themes().join(", ");
        app.set_status(
            format!("Unknown theme: {arg}. Available: {available}"),
            StatusLevel::Error,
        );
        return;
    }

    app.config.theme = Some(arg.to_owned());
    app.theme = super::theme::resolve_theme(&app.config);
    app.invalidate_chat_cache();

    if let Err(err) = libllm::config::save(&app.config) {
        app.set_status(
            format!("Theme applied but failed to save config: {err}"),
            StatusLevel::Warning,
        );
    } else {
        app.set_status(
            format!("Switched to {arg} theme"),
            StatusLevel::Info,
        );
    }
}

fn cmd_macro(app: &mut App, arg: &str, sender: mpsc::Sender<StreamToken>) {
    let arg = arg.trim();
    if arg.is_empty() {
        let names: Vec<&String> = app.config.macros.keys().collect();
        tracing::debug!(result = "listed", macro_count = names.len(), "tui.command.macro");
        if names.is_empty() {
            app.set_status(
                "No macros defined. Add [macros] to config.toml".to_owned(),
                StatusLevel::Warning,
            );
        } else {
            let list = names
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            app.set_status(
                format!("Available macros: {list}"),
                StatusLevel::Info,
            );
        }
        return;
    }

    let (name, macro_args) = match arg.split_once(' ') {
        Some((n, rest)) => (n, rest),
        None => (arg, ""),
    };

    let template = match app.config.macros.get(name) {
        Some(t) => t.clone(),
        None => {
            tracing::debug!(name, result = "unknown", "tui.command.macro");
            app.set_status(
                format!("Unknown macro: {name}"),
                StatusLevel::Warning,
            );
            return;
        }
    };

    match macros::expand_macro(&template, macro_args) {
        Ok(expanded) => {
            tracing::debug!(name, result = "expanded", expanded_bytes = expanded.len(), "tui.command.macro");
            streaming::start_streaming(app, &expanded, sender)
        }
        Err(err) => {
            tracing::warn!(name, result = "error", error = %err, "tui.command.macro");
            app.set_status(err, StatusLevel::Error)
        }
    }
}

fn cmd_report(app: &mut App) {
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            tracing::error!(result = "error", reason = "cwd_error", error = %err, "tui.command.report");
            app.set_status(
                format!("Cannot resolve current directory: {err}"),
                StatusLevel::Error,
            );
            return;
        }
    };
    let output_path = current_dir.join("debug.log");
    if output_path.exists() {
        let output_path_str = output_path.display().to_string();
        tracing::error!(result = "error", reason = "collision", output_path = output_path_str.as_str(), "tui.command.report");
        app.set_status(
            format!("Refusing to overwrite existing {}", output_path.display()),
            StatusLevel::Error,
        );
        return;
    }

    match libllm::diagnostics::copy_current_log_to(&output_path) {
        Ok(()) => {
            let output_path_str = output_path.display().to_string();
            tracing::info!(result = "ok", output_path = output_path_str.as_str(), "tui.command.report");
            app.set_status(
                format!("Debug log copied to {}", output_path.display()),
                StatusLevel::Info,
            )
        }
        Err(err) => {
            let output_path_str = output_path.display().to_string();
            tracing::error!(result = "error", reason = "copy_error", output_path = output_path_str.as_str(), error = %err, "tui.command.report");
            app.set_status(
                format!("Failed to write debug report: {err}"),
                StatusLevel::Error,
            )
        }
    }
}
