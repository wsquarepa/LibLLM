//! Slash command execution and dispatch for TUI chat actions.

pub mod macros;
pub use macros::expand_macro;

pub mod background;
pub mod danger;
pub mod export;
pub mod streaming;

pub(super) use background::handle_background_event;
pub(super) use streaming::{
    handle_stream_token, start_group_chat_loop, start_retry_streaming, start_streaming,
};

use tokio::sync::mpsc;

use libllm_core::session::{self, Role, SaveMode};
use libllm_protocol::client::StreamToken;

use super::business::{self, refresh_sidebar};
use super::{App, Focus, SaveTrigger, StatusLevel, dialogs};

pub(super) async fn handle_slash_command(
    cmd: &str,
    arg: &str,
    app: &mut App<'_>,
    sender: mpsc::Sender<StreamToken>,
) {
    let cmd = libllm_core::commands::resolve_alias(cmd);
    tracing::debug!(cmd, has_arg = !arg.is_empty(), "tui.command");
    match cmd {
        "/quit" => cmd_quit(app),
        "/clear" => cmd_clear(app),
        "/retry" => cmd_retry(app, sender).await,
        "/continue" => cmd_continue(app, arg, sender).await,
        "/system" => cmd_system(app),
        "/config" => cmd_config(app),
        "/branch" => cmd_branch(app),
        "/persona" => cmd_persona(app),
        "/note" => cmd_note(app),
        "/worldbook" => cmd_worldbook(app),
        "/regex" => cmd_regex(app),
        "/character" => cmd_character(app),
        "/chat" => cmd_chat(app),
        "/passkey" => cmd_passkey(app),
        "/theme" => cmd_theme(app, arg),
        "/next" => cmd_next(app, arg, sender).await,
        "/export" => export::cmd_export(app, arg),
        "/macro" => cmd_macro(app, arg, sender).await,
        "/report" => cmd_report(app),
        _ => {
            tracing::debug!(cmd, result = "unknown", "tui.command");
            app.set_status(format!("Unknown command: {cmd}"), StatusLevel::Warning);
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
    app.session.characters = vec![];
    app.session.chat_mode = libllm_core::group_chat::ChatMode::default();
    app.character_cards_cache.clear();
    app.session.author_note = None;
    app.active_persona_name = None;
    app.active_persona_desc = None;
    app.active_card_author_note = None;
    app.discard_pending_session_save();
    app.invalidate_chat_caches();
    app.invalidate_worldbook_cache();
    app.chat_scroll = 0;
    app.auto_scroll = true;
    let new_id = session::generate_session_id();
    app.save_mode.set_id(new_id);
    refresh_sidebar(app);
}

async fn cmd_retry(app: &mut App<'_>, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;

    if app.session.characters.len() >= 2
        && let Some(result) = try_group_retry(app, &sender).await
    {
        return result;
    }

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
            streaming::start_retry_streaming(app, &content, sender).await;
        }
        None => {
            app.set_status("No user message to retry.".to_owned(), StatusLevel::Warning);
        }
    }
}

/// Attempts a group-chat-aware retry of the current head assistant message.
///
/// Returns `Some(())` when a group-chat retry was dispatched (caller should return immediately),
/// or `None` when the head message lacks the group-chat fields needed for restoration (caller
/// should fall through to the default single-character retry path).
async fn try_group_retry(app: &mut App<'_>, sender: &mpsc::Sender<StreamToken>) -> Option<()> {
    let head_msg = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .filter(|n| n.message.role == Role::Assistant)
        .map(|n| n.message.clone())?;

    let speaker_slug = head_msg.speaker.clone()?;
    let snapshot_json = head_msg.pre_turn_action_points.clone()?;

    let snapshot: std::collections::HashMap<String, f32> =
        serde_json::from_str(&snapshot_json).ok()?;

    for c in app.session.characters.iter_mut() {
        if let Some(&ap) = snapshot.get(&c.slug) {
            c.action_points = ap;
        }
    }
    tracing::debug!(speaker = %speaker_slug, "group_chat: /retry restored action-points");

    app.session.tree.retreat_head();
    streaming::run_one_group_turn(app, &speaker_slug, &snapshot_json, sender).await;
    Some(())
}

async fn cmd_continue(app: &mut App<'_>, arg: &str, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;

    let target_speaker = arg.trim();
    if !target_speaker.is_empty() {
        if app.session.characters.len() < 2 {
            app.set_status(
                "/continue <name> only works in group chats.".to_owned(),
                StatusLevel::Warning,
            );
            return;
        }
        let Some(slug) = resolve_speaker_by_name(
            &app.session.characters,
            &app.character_cards_cache,
            target_speaker,
        ) else {
            app.set_status(
                format!("no attached character matches '{target_speaker}'"),
                StatusLevel::Error,
            );
            return;
        };
        let Some(target_node) = most_recent_speaker_node(app, &slug) else {
            app.set_status(
                format!("no message in this branch from '{target_speaker}'"),
                StatusLevel::Warning,
            );
            return;
        };
        app.session.tree.set_head(Some(target_node));
        app.invalidate_chat_caches();
    }

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

    let head_speaker = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .and_then(|n| n.message.speaker.clone());
    if let Some(slug) = head_speaker {
        start_group_continuation(app, &slug, sender).await;
    } else {
        start_continuation(app, sender).await;
    }
}

/// Walks the current branch (head → root) and returns the most recent node whose
/// assistant message was authored by `slug`, or `None` if no such message exists.
fn most_recent_speaker_node(app: &App<'_>, slug: &str) -> Option<libllm_core::session::NodeId> {
    let branch = app.session.tree.current_branch_ids();
    for &node_id in branch.iter().rev() {
        if let Some(node) = app.session.tree.node(node_id)
            && node.message.role == Role::Assistant
            && node.message.speaker.as_deref() == Some(slug)
        {
            return Some(node_id);
        }
    }
    None
}

/// Group-chat-aware continuation: rebuilds the speaker-specific system prompt and
/// nudge using `build_turn_prompt`, then streams onto the existing assistant message
/// (the head is unchanged; `stream_into_message`'s continuation path appends to its
/// content). Mirrors `run_one_group_turn` but reuses the existing node rather than
/// pushing a fresh prefill.
async fn start_group_continuation(
    app: &mut App<'_>,
    speaker_slug: &str,
    sender: mpsc::Sender<StreamToken>,
) {
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

    let tpl_name = app
        .config
        .template_preset
        .as_deref()
        .unwrap_or("Default")
        .to_owned();
    let template = libllm_core::preset::resolve_template_preset(
        &tpl_name,
        &libllm_config::template_presets_dir(),
    );
    let persona = app
        .session
        .persona
        .as_ref()
        .and_then(|slug| app.db.as_ref().and_then(|db| db.load_persona(slug).ok()));
    let base_system_prompt = app.session.system_prompt.clone().or_else(|| {
        app.db
            .as_ref()
            .and_then(|db| {
                db.load_prompt(libllm_core::system_prompt::BUILTIN_ROLEPLAY)
                    .ok()
            })
            .map(|p| p.content)
            .filter(|s| !s.is_empty())
    });
    let nudge_template = app.config.group_chat.nudge_prompt.clone();

    let prompt = match libllm_core::group_chat::build_turn_prompt(
        libllm_core::group_chat::TurnPromptInputs {
            session: app.session,
            cards: &app.character_cards_cache,
            persona: persona.as_ref(),
            template: Some(&template),
            speaker_slug,
            base_system_prompt: base_system_prompt.as_deref(),
            nudge_template: Some(nudge_template.as_str()),
        },
    ) {
        Ok(p) => p,
        Err(e) => {
            app.set_status(
                format!("group continuation prompt build failed: {e}"),
                StatusLevel::Error,
            );
            return;
        }
    };

    streaming::stream_into_message(
        app,
        prompt.system,
        prompt.stop_sequences,
        prompt.nudge,
        sender,
    )
    .await;
}

async fn start_continuation(app: &mut App<'_>, sender: mpsc::Sender<StreamToken>) {
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
    app.stream_started_at = None;
    app.stream_first_think_closed_at = None;
    app.auto_scroll = true;

    streaming::loaded_worldbooks(app);
    let budget = app.context_mgr.token_limit();
    let branch_path = app.session.tree.branch_path();
    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
    let max_drop = libllm_core::context::droppable_count(&summary_aware).saturating_sub(1);

    let render = |k: usize| -> String { streaming::build_rendered_prompt_continuation(app, k).0 };

    let dropped =
        match streaming::find_smallest_drop(&app.token_counter, budget, max_drop, &render).await {
            Ok(k) => k,
            Err(err) => {
                tracing::warn!(
                    result = "fallback_heuristic",
                    error = %err,
                    "continue.truncate"
                );
                app.set_status(
                    format!("Token count failed; continuing without truncation: {err}"),
                    StatusLevel::Warning,
                );
                0
            }
        };
    let prompt = streaming::build_rendered_prompt_continuation(app, dropped).0;
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

    let prompts = app
        .db
        .as_ref()
        .and_then(|db| db.list_prompts().ok())
        .unwrap_or_default();
    if prompts.is_empty() {
        app.set_status("No system prompts found.".to_owned(), StatusLevel::Warning);
    } else {
        app.system_prompt_list = prompts.into_iter().map(|e| e.name).collect();
        app.system_prompt_selected = 0;
        app.open_paged_dialog(Focus::SystemPromptDialog);
    }
}

fn cmd_config(app: &mut App) {
    let cfg = libllm_config::load();
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
        app.set_status("No messages to branch.".to_owned(), StatusLevel::Warning);
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
            let node = app
                .session
                .tree
                .node(sib_id)
                .expect("sib_id was obtained from tree.siblings_of() so it is a valid node id");
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
        let pf = app
            .db
            .as_ref()
            .and_then(|db| db.load_persona(persona_slug).ok());
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

    let personas = app
        .db
        .as_ref()
        .and_then(|db| db.list_personas().ok())
        .unwrap_or_default();
    app.persona_names = personas.iter().map(|(_, name)| name.clone()).collect();
    app.persona_slugs = personas.into_iter().map(|(slug, _)| slug).collect();
    app.persona_selected = 0;
    app.open_paged_dialog(Focus::PersonaDialog);
}

fn cmd_note(app: &mut App) {
    let (text, depth, at_top) = match app.session.author_note.as_ref() {
        Some(note) => (note.text.clone(), note.depth.to_string(), note.at_top),
        None => (
            String::new(),
            libllm_core::author_note::DEFAULT_DEPTH.to_string(),
            false,
        ),
    };

    let pin_value = if at_top { "true" } else { "false" }.to_owned();
    let values = vec![text, depth, pin_value];

    let mut dialog = dialogs::open_author_note_editor(values);
    if app.cli_overrides.author_note.is_some() {
        let mut locks = vec![0_usize];
        if app.cli_overrides.author_note_depth.is_some() {
            locks.push(1);
        }
        if app.cli_overrides.author_note_at_top.is_some() {
            locks.push(2);
        }
        dialog = dialog.with_locked_fields(locks);
    }
    app.author_note_editor = Some(dialog);
    app.focus = Focus::AuthorNoteEditorDialog;
}

fn cmd_worldbook(app: &mut App) {
    let books = app
        .db
        .as_ref()
        .and_then(|db| db.list_worldbooks().ok())
        .unwrap_or_default();
    app.worldbook_list = books.into_iter().map(|(_, name)| name).collect();
    app.worldbook_selected = 0;
    app.open_paged_dialog(Focus::WorldbookDialog);
}

fn cmd_regex(app: &mut App) {
    super::dialogs::regex::open(app);
}

fn cmd_character(app: &mut App) {
    let chars = app
        .db
        .as_ref()
        .and_then(|db| db.list_characters().ok())
        .unwrap_or_default();
    app.character_names = chars.iter().map(|(_, name)| name.clone()).collect();
    app.character_slugs = chars.into_iter().map(|(slug, _)| slug).collect();
    app.character_selected = 0;
    let active_slugs: std::collections::HashSet<&str> = app
        .session
        .characters
        .iter()
        .map(|a| a.slug.as_str())
        .collect();
    app.character_picks = app
        .character_slugs
        .iter()
        .map(|s| active_slugs.contains(s.as_str()))
        .collect();
    app.open_paged_dialog(Focus::CharacterDialog);
}

fn cmd_chat(app: &mut App) {
    app.chat_settings_dialog = Some(dialogs::chat_settings::ChatSettingsDialog::for_session(
        app.session,
    ));
    app.focus = Focus::ChatSettingsDialog;
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
                app.set_status("Database not available.".to_owned(), StatusLevel::Error);
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
        let cfg = libllm_config::load();
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
    app.invalidate_chat_render_cache();

    if let Err(err) = libllm_config::save(&app.config) {
        app.set_status(
            format!("Theme applied but failed to save config: {err}"),
            StatusLevel::Warning,
        );
    } else {
        app.set_status(format!("Switched to {arg} theme"), StatusLevel::Info);
    }
}

async fn cmd_macro(app: &mut App<'_>, arg: &str, sender: mpsc::Sender<StreamToken>) {
    let arg = arg.trim();
    if arg.is_empty() {
        let names: Vec<&String> = app.config.macros.keys().collect();
        tracing::debug!(
            result = "listed",
            macro_count = names.len(),
            "tui.command.macro"
        );
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
            app.set_status(format!("Available macros: {list}"), StatusLevel::Info);
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
            app.set_status(format!("Unknown macro: {name}"), StatusLevel::Warning);
            return;
        }
    };

    match macros::expand_macro(&template, macro_args) {
        Ok(expanded) => {
            tracing::debug!(
                name,
                result = "expanded",
                expanded_bytes = expanded.len(),
                "tui.command.macro"
            );
            streaming::start_streaming(app, &expanded, sender).await
        }
        Err(err) => {
            tracing::warn!(name, result = "error", error = %err, "tui.command.macro");
            app.set_status(err, StatusLevel::Error)
        }
    }
}

async fn cmd_next(app: &mut App<'_>, arg: &str, sender: mpsc::Sender<StreamToken>) {
    if app.session.characters.len() < 2 {
        app.set_status(
            "/next requires 2 or more attached characters".to_owned(),
            StatusLevel::Warning,
        );
        return;
    }
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

    let needle = arg.trim();
    if needle.is_empty() {
        if matches!(
            app.session.chat_mode,
            libllm_core::group_chat::ChatMode::Directed
        ) {
            app.set_status(
                "directed mode: use /next <name>".to_owned(),
                StatusLevel::Warning,
            );
            return;
        }
        start_group_chat_loop(app, &sender).await;
        return;
    }
    match resolve_speaker_by_name(&app.session.characters, &app.character_cards_cache, needle) {
        Some(slug) => {
            let snapshot_before: std::collections::HashMap<String, f32> = app
                .session
                .characters
                .iter()
                .map(|c| (c.slug.clone(), c.action_points))
                .collect();
            let snapshot_json = serde_json::to_string(&snapshot_before).unwrap_or_default();
            streaming::run_one_group_turn(app, &slug, &snapshot_json, &sender).await;
        }
        None => {
            app.set_status(
                format!("no attached character matches '{needle}'"),
                StatusLevel::Error,
            );
        }
    }
}

fn resolve_speaker_by_name(
    chars: &[libllm_core::group_chat::CharacterAttachment],
    card_cache: &std::collections::HashMap<String, libllm_core::character::CharacterCard>,
    needle: &str,
) -> Option<String> {
    let cast: Vec<(&str, &str)> = chars
        .iter()
        .filter_map(|c| {
            card_cache
                .get(&c.slug)
                .map(|card| (c.slug.as_str(), card.name.as_str()))
        })
        .collect();
    let matched_name = crate::match_next_candidates(needle, &cast)
        .into_iter()
        .next()?;
    chars
        .iter()
        .find(|c| {
            card_cache
                .get(&c.slug)
                .is_some_and(|card| card.name == matched_name)
        })
        .map(|c| c.slug.clone())
}

fn cmd_report(app: &mut App) {
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            tracing::warn!(result = "error", reason = "cwd_error", error = %err, "tui.command.report");
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
        tracing::warn!(
            result = "error",
            reason = "collision",
            output_path = output_path_str.as_str(),
            "tui.command.report"
        );
        app.set_status(
            format!("Refusing to overwrite existing {}", output_path.display()),
            StatusLevel::Error,
        );
        return;
    }

    match libllm_core::diagnostics::copy_current_log_to(&output_path) {
        Ok(()) => {
            let output_path_str = output_path.display().to_string();
            tracing::info!(
                result = "ok",
                output_path = output_path_str.as_str(),
                "tui.command.report"
            );
            app.set_status(
                format!("Debug log copied to {}", output_path.display()),
                StatusLevel::Info,
            )
        }
        Err(err) => {
            let output_path_str = output_path.display().to_string();
            tracing::warn!(result = "error", reason = "copy_error", output_path = output_path_str.as_str(), error = %err, "tui.command.report");
            app.set_status(
                format!("Failed to write debug report: {err}"),
                StatusLevel::Error,
            )
        }
    }
}

/// Periodic housekeeping: completed-summary splicing, file-summary readiness drain,
/// debounced search, save-deadline flush, sidebar age refresh, status-message expiry,
/// and reject-flash decay.
///
/// Returns `true` when any of these mutated state in a way that requires a redraw.
pub(crate) async fn run_periodic_tasks(
    app: &mut App<'_>,
    token_tx: mpsc::Sender<StreamToken>,
) -> bool {
    let mut needs_redraw = false;
    if app.summary_receiver.is_some() {
        let completed = app
            .summary_receiver
            .as_mut()
            .expect("summary_receiver is Some, checked by the enclosing is_some() guard")
            .try_recv();
        if let Ok(result) = completed {
            let current_head = app.session.tree.head();
            let expected_head = app.summary_branch_head;
            app.summary_receiver = None;
            app.summary_branch_head = None;

            if current_head == expected_head
                && app.summarization_enabled
                && let Ok(summary_text) = result
            {
                let dropped = app.summary_pending_dropped.take().unwrap_or(0);

                if dropped > 0 {
                    let branch_path = app.session.tree.branch_path();
                    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
                    let branch_ids = app.session.tree.current_branch_ids();
                    let summary_boundary = branch_ids.len() - summary_aware.len();
                    let split_idx = libllm_core::context::drop_split_index(&summary_aware, dropped);
                    let insert_idx = summary_boundary + split_idx - 1;
                    if split_idx > 0 && insert_idx < branch_ids.len() {
                        let parent_node_id = branch_ids[insert_idx];
                        app.session.tree.splice_between(
                            parent_node_id,
                            libllm_core::session::Message::new(
                                libllm_core::session::Role::Summary,
                                summary_text,
                            ),
                        );
                        app.mark_session_dirty(SaveTrigger::StreamDone, true);
                        app.invalidate_chat_caches();
                    }
                }
            }

            app.is_summarizing = false;
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                start_streaming(app, &next, token_tx).await;
                if !app.is_streaming {
                    app.message_queue.clear();
                }
            }
            needs_redraw = true;
        }
    }
    while let Ok(event) = app.file_summary_ready_rx.try_recv() {
        tracing::debug!(
            session_id = %event.session_id,
            content_hash = %event.content_hash,
            status = ?event.status,
            "tui.file_summary.ready"
        );
        app.invalidate_chat_render_cache();
        app.file_summary_revision = app.file_summary_revision.wrapping_add(1);
        needs_redraw = true;
    }
    if matches!(app.focus, Focus::SearchDialog)
        && let (Some(state), Some(db)) = (app.search_dialog.as_mut(), app.db.as_ref())
    {
        dialogs::search::maybe_run_query(state, db, std::time::Instant::now());
        needs_redraw = true;
    }
    if app
        .pending_save_deadline
        .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    {
        let trigger = app.pending_save_trigger.unwrap_or(SaveTrigger::Retry);
        if let Err(err) = app.flush_session_save(trigger) {
            app.set_status(format!("Save error: {err}"), StatusLevel::Error);
        }
        needs_redraw = true;
    }
    if std::time::Instant::now() >= app.sidebar_age_refresh_at {
        business::refresh_sidebar_ages(app);
        needs_redraw = true;
    }
    if let Some(ref msg) = app.status_message {
        if std::time::Instant::now() >= msg.expires {
            app.status_message = None;
        }
        needs_redraw = true;
    }
    if app.tick_reject_flashes() {
        needs_redraw = true;
    }
    needs_redraw
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use libllm_core::character::CharacterCard;
    use libllm_core::group_chat::CharacterAttachment;

    use super::resolve_speaker_by_name;

    fn make_card(name: &str) -> CharacterCard {
        CharacterCard {
            name: name.to_owned(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            alternate_greetings: vec![],
            author_note: None,
        }
    }

    #[test]
    fn resolve_speaker_substring_match() {
        let chars = vec![
            CharacterAttachment {
                slug: "alice-slug".to_owned(),
                talkativeness: 1.0,
                action_points: 0.0,
                spoke_this_round: false,
            },
            CharacterAttachment {
                slug: "bob-slug".to_owned(),
                talkativeness: 1.0,
                action_points: 0.0,
                spoke_this_round: false,
            },
        ];
        let mut cache = HashMap::new();
        cache.insert("alice-slug".to_owned(), make_card("Alice the Wise"));
        cache.insert("bob-slug".to_owned(), make_card("Bob the Knight"));

        assert_eq!(
            resolve_speaker_by_name(&chars, &cache, "ali"),
            Some("alice-slug".to_owned()),
        );
        assert_eq!(
            resolve_speaker_by_name(&chars, &cache, "ali wi"),
            Some("alice-slug".to_owned()),
        );
        assert_eq!(
            resolve_speaker_by_name(&chars, &cache, "bob kni"),
            Some("bob-slug".to_owned()),
        );
        assert_eq!(resolve_speaker_by_name(&chars, &cache, "zelda"), None);
    }
}
