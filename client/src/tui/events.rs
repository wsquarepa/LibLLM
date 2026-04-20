//! Top-level event dispatch for keyboard, mouse, and paste events.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use tokio::sync::mpsc;
use tui_textarea::{CursorMove, TextArea};

use libllm::client::StreamToken;

use super::dialog_handler::{
    DialogKind, cancel_generation, configure_textarea, handle_field_dialog_key,
    live_apply_theme_dialog,
};
use super::types::*;
use super::{clipboard, commands, dialogs, input, render};

pub(super) fn handle_event(
    event: Event,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(key, app, bg_tx),
        Event::Paste(ref text) => handle_paste(text.clone(), event, app),
        Event::Mouse(mouse) => handle_mouse(mouse, app),
        _ => None,
    }
}

fn handle_paste(text: String, raw_event: Event, app: &mut App) -> Option<Action> {
    let cleaned = clean_pasted_path(&text);
    let path = std::path::Path::new(&cleaned);

    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let handled = match app.focus {
            Focus::CharacterDialog => dialogs::character::handle_character_paste(path, &ext, app),
            Focus::WorldbookDialog => dialogs::worldbook::handle_worldbook_paste(path, &ext, app),
            Focus::SystemPromptDialog => {
                dialogs::system_prompt::handle_system_prompt_paste(path, &ext, app)
            }
            Focus::PersonaDialog => dialogs::persona::handle_persona_paste(path, &ext, app),
            Focus::Sidebar => input::handle_sidebar_paste(path, &ext, app),
            _ => false,
        };

        if handled {
            return None;
        }
    }

    match app.focus {
        Focus::Input => {
            if let Some(token) = paste_as_file_reference(&text, &app.config.files) {
                app.textarea.insert_str(&token);
            } else {
                app.textarea.input(raw_event);
            }
        }
        Focus::EditDialog => {
            if let Some(ref mut editor) = app.edit_editor {
                editor.insert_str(&text);
            }
        }
        Focus::PresetEditorDialog => {
            if let Some(ref mut d) = app.preset_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::PersonaEditorDialog => {
            if let Some(ref mut d) = app.persona_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::CharacterEditorDialog => {
            if let Some(ref mut d) = app.character_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::SystemPromptEditorDialog => {
            if let Some(ref mut d) = app.system_prompt_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::WorldbookEntryEditorDialog => {
            if let Some(ref mut d) = app.worldbook_entry_editor {
                d.insert_into_active_editor(&text);
            }
        }
        _ => {}
    }
    None
}

/// If `raw` resolves to an existing, supported file, return the
/// canonical `@<path>` string to insert. Otherwise return `None` so the
/// caller falls through to raw paste. Silent on any resolution failure.
fn paste_as_file_reference(raw: &str, config: &libllm::config::FilesConfig) -> Option<String> {
    if !config.enabled {
        return None;
    }
    let trimmed = clean_pasted_path(raw);
    let path = std::path::Path::new(&trimmed);
    if !path.is_file() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    let metadata = std::fs::metadata(&canonical).ok()?;
    if (metadata.len() as usize) > config.per_file_bytes {
        return None;
    }
    let bytes = std::fs::read(&canonical).ok()?;
    libllm::files::classify(&canonical, &bytes).ok()?;
    let display = canonical.to_string_lossy();
    Some(format_at_token(&display))
}

/// Build an `@<path>` token, wrapping the path in double quotes when it
/// contains any whitespace so the tokeniser captures the full path.
fn format_at_token(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@\"{path}\"")
    } else {
        format!("@{path}")
    }
}

fn clean_pasted_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(super) async fn process_action(
    action: Action,
    app: &mut App<'_>,
    token_tx: mpsc::Sender<StreamToken>,
) {
    match action {
        Action::Quit => {
            app.should_quit = true;
        }
        Action::SendMessage(text) => {
            app.nav_cursor = None;
            let recall_refs = app.recall_refs.take();
            let reuse_parent_chain = recall_refs
                .as_ref()
                .is_some_and(|refs| refs == &file_ref_paths(&text));
            if reuse_parent_chain {
                commands::start_retry_streaming(app, &text, token_tx).await;
            } else {
                if recall_refs.is_some() {
                    let head = app.session.tree.head();
                    let anchor = input::retreat_past_snapshot_chain(&app.session.tree, head);
                    app.session.tree.set_head(anchor);
                }
                commands::start_streaming(app, &text, token_tx).await;
            }
        }
        Action::EditMessage { node_id, content } => {
            handle_edit_message(app, node_id, content);
        }
        Action::SlashCommand(cmd, arg) => {
            commands::handle_slash_command(&cmd, &arg, app, token_tx).await;
        }
    }
}

fn file_ref_paths(raw: &str) -> Vec<String> {
    libllm::files::file_reference_ranges(raw)
        .into_iter()
        .filter(|r| r.path() != "stdin")
        .map(|r| r.path().to_owned())
        .collect()
}

fn handle_edit_message(
    app: &mut App<'_>,
    node_id: libllm::session::NodeId,
    content: String,
) {
    let old_content = match app.session.tree.node(node_id) {
        Some(n) => n.message.content.clone(),
        None => {
            app.set_status(
                "edit target vanished from the tree".to_owned(),
                crate::tui::types::StatusLevel::Error,
            );
            return;
        }
    };

    let file_refs_unchanged = file_ref_paths(&old_content) == file_ref_paths(&content);

    if file_refs_unchanged {
        if let Some(new_root) = app.session.tree.duplicate_subtree(node_id)
            && app.session.tree.set_message_content(new_root, content)
        {
            app.session.tree.switch_to(new_root);
            app.invalidate_chat_cache();
            app.nav_cursor = Some(new_root);
            app.focus = Focus::Chat;
            app.mark_session_dirty(SaveTrigger::Debounced, false);
        }
        return;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let sys_messages = match libllm::files::resolve_all(&content, &cwd, &app.config.files) {
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

    let original_parent = app
        .session
        .tree
        .node(node_id)
        .and_then(|n| n.parent);

    let mut parent = original_parent;
    for sys_msg in sys_messages {
        let id = app.session.tree.push(parent, sys_msg);
        parent = Some(id);
    }
    let new_user = app.session.tree.push(
        parent,
        libllm::session::Message::new(libllm::session::Role::User, content),
    );

    app.session.tree.switch_to(new_user);
    app.invalidate_chat_cache();
    app.nav_cursor = Some(new_user);
    app.focus = Focus::Chat;
    app.mark_session_dirty(SaveTrigger::Debounced, false);
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    #[cfg(debug_assertions)]
    {
        let invariant_ok = match app.focus {
            Focus::ConfigDialog => app.config_dialog.is_some(),
            Focus::ThemeDialog => app.theme_dialog.is_some(),
            Focus::PresetEditorDialog => app.preset_editor.is_some(),
            Focus::PersonaEditorDialog => app.persona_editor.is_some(),
            Focus::CharacterEditorDialog => app.character_editor.is_some(),
            Focus::SystemPromptEditorDialog => app.system_prompt_editor.is_some(),
            Focus::WorldbookEntryEditorDialog => app.worldbook_entry_editor.is_some(),
            Focus::EditDialog => app.edit_editor.is_some(),
            Focus::EditConfirmDialog => app.edit_editor.is_some(),
            Focus::FilePickerDialog => app.file_picker.is_some(),
            Focus::InjectionWarningDialog => app.injection_warning.is_some(),
            Focus::Input
            | Focus::Chat
            | Focus::Sidebar
            | Focus::PasskeyDialog
            | Focus::SetPasskeyDialog
            | Focus::BaseThemePickerDialog
            | Focus::PresetPickerDialog
            | Focus::AuthDialog
            | Focus::AuthTypePicker
            | Focus::PersonaDialog
            | Focus::CharacterDialog
            | Focus::WorldbookDialog
            | Focus::WorldbookEditorDialog
            | Focus::WorldbookEntryDeleteDialog
            | Focus::SystemPromptDialog
            | Focus::BranchDialog
            | Focus::DeleteConfirmDialog
            | Focus::ApiErrorDialog
            | Focus::LoadingDialog => true,
        };
        debug_assert!(
            invariant_ok,
            "focus {:?} points at a dialog whose state is None",
            app.focus
        );
    }
    if app.focus == Focus::PasskeyDialog {
        return dialogs::passkey::handle_passkey_key(key, app, bg_tx.clone());
    }
    if app.focus == Focus::SetPasskeyDialog {
        return dialogs::set_passkey::handle_set_passkey_key(key, app, bg_tx);
    }
    if app.focus == Focus::PresetPickerDialog {
        return dialogs::preset::handle_preset_dialog_key(key, app);
    }
    if app.focus == Focus::AuthDialog {
        match dialogs::auth::handle_auth_dialog_key(key, app) {
            dialogs::auth::AuthDialogAction::Continue => return None,
            dialogs::auth::AuthDialogAction::Close => {
                dialogs::auth::close_and_persist(app);
                return None;
            }
            dialogs::auth::AuthDialogAction::OpenTypePicker => {
                dialogs::auth::open_type_picker(app);
                return None;
            }
        }
    }
    if app.focus == Focus::AuthTypePicker {
        return dialogs::auth::handle_type_picker_key(key, app);
    }
    if app.focus == Focus::PresetEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::PresetEditor);
    }
    if app.focus == Focus::ConfigDialog {
        return handle_field_dialog_key(key, app, DialogKind::Config);
    }
    if app.focus == Focus::ThemeDialog {
        return handle_field_dialog_key(key, app, DialogKind::Theme);
    }
    if app.focus == Focus::BaseThemePickerDialog {
        return handle_base_theme_picker_key(key, app);
    }
    if app.focus == Focus::PersonaDialog {
        return dialogs::persona::handle_persona_dialog_key(key, app);
    }
    if app.focus == Focus::PersonaEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::PersonaEditor);
    }
    if app.focus == Focus::CharacterDialog {
        return dialogs::character::handle_character_dialog_key(key, app);
    }
    if app.focus == Focus::CharacterEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::CharacterEditor);
    }
    if app.focus == Focus::WorldbookDialog {
        return dialogs::worldbook::handle_worldbook_dialog_key(key, app);
    }
    if app.focus == Focus::WorldbookEditorDialog {
        return dialogs::worldbook::handle_worldbook_editor_key(key, app);
    }
    if app.focus == Focus::WorldbookEntryEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::WorldbookEntryEditor);
    }
    if app.focus == Focus::WorldbookEntryDeleteDialog {
        return dialogs::worldbook::handle_entry_delete_key(key, app);
    }
    if app.focus == Focus::SystemPromptDialog {
        return dialogs::system_prompt::handle_system_prompt_dialog_key(key, app);
    }
    if app.focus == Focus::SystemPromptEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::SystemPromptEditor);
    }
    if app.focus == Focus::EditDialog {
        return dialogs::edit::handle_edit_key(key, app);
    }
    if app.focus == Focus::EditConfirmDialog {
        return dialogs::edit::handle_edit_confirm_key(key, app);
    }
    if app.focus == Focus::BranchDialog {
        return dialogs::branch::handle_branch_dialog_key(key, app);
    }
    if app.focus == Focus::DeleteConfirmDialog {
        return dialogs::delete_confirm::handle_delete_confirm_key(key, app);
    }
    if app.focus == Focus::ApiErrorDialog {
        return dialogs::api_error::handle_api_error_key(key, app);
    }
    if app.focus == Focus::FilePickerDialog {
        return dialogs::file_picker::handle_key(key, app);
    }
    if app.focus == Focus::InjectionWarningDialog {
        return dialogs::injection_warning::handle_key(key, app);
    }
    if app.focus == Focus::LoadingDialog {
        return dialogs::api_error::handle_loading_key(key);
    }

    if app.is_streaming {
        return handle_streaming_key(key, app);
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if app.focus == Focus::Input && app.textarea.selection_range().is_some() {
            let (consumed, warning) = clipboard::handle_clipboard_key(&key, &mut app.textarea);
            if let Some(msg) = warning {
                app.set_status(msg, StatusLevel::Warning);
            }
            if consumed {
                return None;
            }
        }
        return Some(Action::Quit);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        let previous_head = app.session.tree.head();
        app.session.tree.switch_sibling(-1);
        if app.session.tree.head() != previous_head {
            app.mark_session_dirty(SaveTrigger::Debounced, false);
        }
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        let previous_head = app.session.tree.head();
        app.session.tree.switch_sibling(1);
        if app.session.tree.head() != previous_head {
            app.mark_session_dirty(SaveTrigger::Debounced, false);
        }
        return None;
    }

    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Input => {
                app.nav_cursor = app.session.tree.current_branch_ids().last().copied();
                app.auto_scroll = false;
                Focus::Chat
            }
            Focus::Chat => {
                app.nav_cursor = None;
                Focus::Sidebar
            }
            _ => {
                app.nav_cursor = None;
                Focus::Input
            }
        };
        return None;
    }

    if key.code == KeyCode::Esc {
        if app.focus == Focus::Sidebar && app.sidebar_search.active {
            return input::handle_sidebar_key(key, app);
        }
        app.nav_cursor = None;
        app.focus = Focus::Input;
        app.auto_scroll = true;
        return None;
    }

    match app.focus {
        Focus::Input => input::handle_input_key(key, app),
        Focus::Chat => input::handle_chat_key(key, app),
        Focus::Sidebar => input::handle_sidebar_key(key, app),
        _ => None,
    }
}

fn handle_streaming_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if key.code == KeyCode::Esc {
        cancel_generation(app);
        if !app.message_queue.is_empty() {
            let next = app.message_queue.remove(0);
            return Some(Action::SendMessage(next));
        }
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Enter && key.modifiers.is_empty() {
        let lines: Vec<String> = app.textarea.lines().to_vec();
        let trimmed = lines.join("\n").trim().to_owned();

        if trimmed.is_empty() {
            return None;
        }

        if trimmed.starts_with('/') {
            app.set_status(
                "Slash commands cannot be queued during generation".to_owned(),
                StatusLevel::Warning,
            );
            return None;
        }

        app.textarea = TextArea::default();
        configure_textarea(&mut app.textarea);
        app.message_queue.push(trimmed);
        return None;
    }

    app.textarea.input(key);
    None
}

fn handle_mouse(mouse: MouseEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }
    let areas = app.layout_areas.as_ref()?;
    let sidebar = areas.sidebar;
    let chat = areas.chat;
    let input = areas.input;
    let pos = Position::new(mouse.column, mouse.row);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if is_dialog_focus(app.focus) {
                dialogs::handle_dialog_mouse_click(mouse, app);
                return None;
            }

            if sidebar.contains(pos) {
                app.focus = Focus::Sidebar;
                app.nav_cursor = None;
                if mouse.row + 1 == sidebar.y + sidebar.height
                    && hit_search_title(&app.sidebar_search, sidebar, mouse.column)
                {
                    if !app.sidebar_search.active {
                        let current = app.sidebar_state.selected().unwrap_or(0);
                        app.sidebar_search.enter(current);
                        app.sidebar_cache = None;
                    }
                    return None;
                }
                let inner_row = mouse.row.saturating_sub(sidebar.y + 1) as usize;
                let offset = app.sidebar_state.offset();
                let selected_idx = app.sidebar_state.selected();
                let mut cumulative: usize = 0;
                let mut hit_index: Option<usize> = None;
                for i in offset..app.sidebar_sessions.len() {
                    let has_preview = selected_idx == Some(i)
                        && app.sidebar_sessions[i].sidebar_preview.is_some();
                    let item_height: usize = if has_preview { 2 } else { 1 };
                    if inner_row < cumulative + item_height {
                        hit_index = Some(i);
                        break;
                    }
                    cumulative += item_height;
                }
                if let Some(index) = hit_index
                    && selected_idx != Some(index)
                {
                    app.sidebar_state.select(Some(index));
                    input::load_sidebar_selection(app);
                }
            } else if chat.contains(pos) {
                app.sidebar_search.commit();
                app.focus = Focus::Chat;
                if let Some(ref cache) = app.chat_content_cache {
                    let branch_ids = app.session.tree.current_branch_ids();
                    if let Some(node_id) = render::hit_test_chat_message(
                        cache,
                        branch_ids,
                        chat,
                        app.chat_scroll,
                        mouse.row,
                    ) {
                        app.nav_cursor = Some(node_id);
                    }
                }
                app.auto_scroll = false;
            } else if input.contains(pos) {
                app.sidebar_search.commit();
                app.focus = Focus::Input;
                app.nav_cursor = None;
                app.auto_scroll = true;
                app.textarea.cancel_selection();
                move_textarea_cursor_to_mouse(&mut app.textarea, input, mouse.column, mouse.row);
            }
            None
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.focus == Focus::Input && input.contains(pos) {
                if app.textarea.selection_range().is_none() {
                    app.textarea.start_selection();
                }
                move_textarea_cursor_to_mouse(&mut app.textarea, input, mouse.column, mouse.row);
            } else if app.focus == Focus::EditDialog
                && let Some(ref mut editor) = app.edit_editor
                && let Ok((tw, th)) = crossterm::terminal::size()
            {
                let terminal_area = Rect::new(0, 0, tw, th);
                let width = (tw as f32 * dialogs::DIALOG_WIDTH_RATIO) as u16;
                let height = (th as f32 * dialogs::DIALOG_HEIGHT_RATIO) as u16;
                let dialog = render::centered_rect(width, height, terminal_area);
                let editor_area = Rect {
                    x: dialog.x + 2,
                    y: dialog.y + 1,
                    width: dialog.width.saturating_sub(4),
                    height: dialog.height.saturating_sub(2),
                };
                if editor.selection_range().is_none() {
                    editor.start_selection();
                }
                move_textarea_cursor_to_mouse(editor, editor_area, mouse.column, mouse.row);
            }
            None
        }
        MouseEventKind::ScrollUp => {
            if is_dialog_focus(app.focus) {
                scroll_dialog(app, ScrollDirection::Up);
            } else if chat.contains(pos) {
                app.chat_scroll = app.chat_scroll.saturating_sub(3);
                app.auto_scroll = false;
            } else if sidebar.contains(pos) {
                let selected = app.sidebar_state.selected().unwrap_or(0);
                let new = selected.saturating_sub(1);
                app.sidebar_state.select(Some(new));
                input::load_sidebar_selection(app);
            }
            None
        }
        MouseEventKind::ScrollDown => {
            if is_dialog_focus(app.focus) {
                scroll_dialog(app, ScrollDirection::Down);
            } else if chat.contains(pos) {
                app.chat_scroll = app.chat_scroll.saturating_add(3).min(app.chat_max_scroll);
                app.auto_scroll = false;
            } else if sidebar.contains(pos) {
                let selected = app.sidebar_state.selected().unwrap_or(0);
                let count = app.sidebar_sessions.len();
                if count > 0 {
                    let new = (selected + 1).min(count - 1);
                    app.sidebar_state.select(Some(new));
                    input::load_sidebar_selection(app);
                }
            }
            None
        }
        MouseEventKind::Moved => {
            if chat.contains(pos) {
                if let Some(ref cache) = app.chat_content_cache {
                    let branch_ids = app.session.tree.current_branch_ids();
                    app.hover_node = render::hit_test_chat_message(
                        cache,
                        branch_ids,
                        chat,
                        app.chat_scroll,
                        mouse.row,
                    );
                } else {
                    app.hover_node = None;
                }
            } else {
                app.hover_node = None;
            }
            None
        }
        _ => None,
    }
}

fn hit_search_title(state: &dialogs::SearchState, container: Rect, click_col: u16) -> bool {
    let max = container.width.saturating_sub(2);
    let width = dialogs::search_title_width(state, max);
    let left_edge = container.x;
    let right_edge = left_edge + width;
    click_col >= left_edge && click_col < right_edge
}

#[derive(Clone, Copy)]
enum ScrollDirection {
    Up,
    Down,
}

fn scroll_dialog(app: &mut App, direction: ScrollDirection) {
    let code = match direction {
        ScrollDirection::Up => KeyCode::Up,
        ScrollDirection::Down => KeyCode::Down,
    };
    let key = KeyEvent::new(code, KeyModifiers::NONE);
    match app.focus {
        Focus::CharacterDialog => {
            dialogs::character::handle_character_dialog_key(key, app);
        }
        Focus::PersonaDialog => {
            dialogs::persona::handle_persona_dialog_key(key, app);
        }
        Focus::SystemPromptDialog => {
            dialogs::system_prompt::handle_system_prompt_dialog_key(key, app);
        }
        Focus::BranchDialog => {
            dialogs::branch::handle_branch_dialog_key(key, app);
        }
        Focus::WorldbookDialog => {
            dialogs::worldbook::handle_worldbook_dialog_key(key, app);
        }
        Focus::WorldbookEditorDialog => {
            dialogs::worldbook::handle_worldbook_editor_key(key, app);
        }
        Focus::PresetPickerDialog => {
            dialogs::preset::handle_preset_dialog_key(key, app);
        }
        Focus::BaseThemePickerDialog => {
            handle_base_theme_picker_key(key, app);
        }
        _ => {}
    }
}

pub(super) fn move_textarea_cursor_to_mouse(
    textarea: &mut TextArea,
    widget_area: Rect,
    screen_col: u16,
    screen_row: u16,
) {
    let inner_row = screen_row.saturating_sub(widget_area.y + 1);
    let inner_col = screen_col.saturating_sub(widget_area.x + 1);
    textarea.move_cursor(CursorMove::Jump(inner_row, inner_col));
}

pub(super) fn is_dialog_focus(focus: Focus) -> bool {
    !matches!(focus, Focus::Input | Focus::Chat | Focus::Sidebar)
}

fn handle_base_theme_picker_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Up if app.base_theme_picker_selected > 0 => {
            app.base_theme_picker_selected -= 1;
        }
        KeyCode::Down => {
            let count = app.base_theme_picker_names.len();
            if count > 0 && app.base_theme_picker_selected + 1 < count {
                app.base_theme_picker_selected += 1;
            }
        }
        KeyCode::Enter => {
            let chosen = app
                .base_theme_picker_names
                .get(app.base_theme_picker_selected)
                .cloned()
                .unwrap_or_default();
            if let Some(ref mut dialog) = app.theme_dialog {
                dialog.set_value(0, 0, chosen);
            }
            app.focus = Focus::ThemeDialog;
            live_apply_theme_dialog(app);
        }
        KeyCode::Esc => {
            app.focus = Focus::ThemeDialog;
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod paste_tests {
    use super::paste_as_file_reference;
    use tempfile::TempDir;

    #[test]
    fn non_file_paste_returns_none() {
        let cfg = libllm::config::FilesConfig::default();
        assert!(paste_as_file_reference("not a path", &cfg).is_none());
    }

    #[test]
    fn file_paste_returns_at_path() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("a.md");
        std::fs::write(&p, "hello").unwrap();
        let cfg = libllm::config::FilesConfig::default();
        let out = paste_as_file_reference(p.to_str().unwrap(), &cfg).unwrap();
        assert!(out.starts_with("@"));
        assert!(out.contains("a.md"));
    }

    #[test]
    fn binary_paste_returns_none() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("bin");
        std::fs::write(&p, [0x89u8, 0x50, 0x4E, 0x47, 0, 0, 0, 0]).unwrap();
        let cfg = libllm::config::FilesConfig::default();
        assert!(paste_as_file_reference(p.to_str().unwrap(), &cfg).is_none());
    }

    #[test]
    fn disabled_config_returns_none() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("a.md");
        std::fs::write(&p, "hello").unwrap();
        let cfg = libllm::config::FilesConfig {
            enabled: false,
            per_file_bytes: 524_288,
            per_message_bytes: 4_194_304,
            ..libllm::config::FilesConfig::default()
        };
        assert!(paste_as_file_reference(p.to_str().unwrap(), &cfg).is_none());
    }

    #[test]
    fn oversize_paste_returns_none() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("big.md");
        std::fs::write(&p, vec![b'x'; 2000]).unwrap();
        let cfg = libllm::config::FilesConfig {
            enabled: true,
            per_file_bytes: 1000,
            per_message_bytes: 4_194_304,
            ..libllm::config::FilesConfig::default()
        };
        assert!(paste_as_file_reference(p.to_str().unwrap(), &cfg).is_none());
    }

    #[test]
    fn paste_wraps_path_with_spaces_in_quotes() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("Lecture 29 notes.md");
        std::fs::write(&p, "body").unwrap();
        let cfg = libllm::config::FilesConfig::default();
        let out = paste_as_file_reference(p.to_str().unwrap(), &cfg)
            .expect("spaced path should paste");
        assert!(out.starts_with(r#"@""#));
        assert!(out.ends_with('"'));
        assert!(out.contains("Lecture 29 notes.md"));
    }

    #[test]
    fn paste_leaves_non_spaced_path_bare() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("plain.md");
        std::fs::write(&p, "body").unwrap();
        let cfg = libllm::config::FilesConfig::default();
        let out = paste_as_file_reference(p.to_str().unwrap(), &cfg).unwrap();
        assert!(out.starts_with('@'));
        assert!(!out.starts_with(r#"@""#));
        assert!(!out.ends_with('"'));
    }
}
