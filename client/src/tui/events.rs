//! Top-level event dispatch for keyboard, mouse, and paste events.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use tokio::sync::mpsc;
use tui_textarea::{CursorMove, TextArea};

use libllm::client::StreamToken;

use super::types::*;
use super::{clipboard, commands, dialogs, input, render};
use super::dialog_handler::{
    cancel_generation, configure_textarea, handle_field_dialog_key, live_apply_theme_dialog,
    DialogKind,
};

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
            app.textarea.input(raw_event);
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

pub(super) fn process_action(action: Action, app: &mut App, token_tx: mpsc::Sender<StreamToken>) {
    match action {
        Action::Quit => {
            app.should_quit = true;
        }
        Action::SendMessage(text) => {
            app.nav_cursor = None;
            commands::start_streaming(app, &text, token_tx);
        }
        Action::EditMessage { node_id, content } => {
            if let Some(new_root) = app.session.tree.duplicate_subtree(node_id)
                && app.session.tree.set_message_content(new_root, content) {
                    app.session.tree.switch_to(new_root);
                    app.invalidate_chat_cache();
                    app.nav_cursor = Some(new_root);
                    app.focus = Focus::Chat;
                    app.mark_session_dirty(SaveTrigger::Debounced, false);
                }
        }
        Action::SlashCommand(cmd, arg) => {
            commands::handle_slash_command(&cmd, &arg, app, token_tx);
        }
    }
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
                    && selected_idx != Some(index) {
                        app.sidebar_state.select(Some(index));
                        input::load_sidebar_selection(app);
                    }
            } else if chat.contains(pos) {
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
                    && let Ok((tw, th)) = crossterm::terminal::size() {
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
            if chat.contains(pos) {
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
            if chat.contains(pos) {
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
