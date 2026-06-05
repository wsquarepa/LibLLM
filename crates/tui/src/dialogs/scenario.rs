//! Scenario edit dialog: single multiline textarea for editing the session scenario.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;
use tui_textarea::TextArea;

use super::{
    DIALOG_HEIGHT_RATIO, DIALOG_WIDTH_RATIO, clear_centered, dialog_block,
    render_hints_below_dialog,
};
use crate::types::{App, Focus, StatusLevel};
use crate::{Action, clipboard, dialog_handler, events};

pub(crate) fn open(app: &mut App) {
    let initial = app
        .chat_settings_dialog
        .as_ref()
        .and_then(|dlg| dlg.provisional_scenario())
        .map(str::to_owned)
        .or_else(|| app.session.scenario.clone())
        .unwrap_or_default();
    let lines: Vec<String> = if initial.is_empty() {
        vec![String::new()]
    } else {
        initial.lines().map(String::from).collect()
    };
    let mut editor = TextArea::from(lines);
    dialog_handler::configure_textarea_at_start(&mut editor);
    app.scenario_editor = Some(editor);
    app.scenario_scroll_top = 0;
    app.focus = Focus::ScenarioEditorDialog;
}

fn editor_area(dialog: Rect) -> Rect {
    Rect {
        x: dialog.x + 2,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(4),
        height: dialog.height.saturating_sub(2),
    }
}

fn dialog_rect(area: Rect) -> Rect {
    let width = (area.width as f32 * DIALOG_WIDTH_RATIO) as u16;
    let height = (area.height as f32 * DIALOG_HEIGHT_RATIO) as u16;
    crate::render::centered_rect(width, height, area)
}

pub(crate) fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(ref editor) = app.scenario_editor else {
        return;
    };
    let width = (area.width as f32 * DIALOG_WIDTH_RATIO) as u16;
    let height = (area.height as f32 * DIALOG_HEIGHT_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    f.render_widget(dialog_block(" Edit Scenario ", Color::Yellow), dialog);

    let editor_rect = editor_area(dialog);
    app.scenario_scroll_top =
        events::update_scroll_top(app.scenario_scroll_top, editor, editor_rect);
    f.render_widget(editor, editor_rect);

    render_hints_below_dialog(f, dialog, area, &[Line::from("Esc: close")]);
}

pub(crate) fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if key.code == KeyCode::Esc {
        stage_provisional_and_close(app);
        return None;
    }

    let editor = app.scenario_editor.as_mut()?;
    let (consumed, warning) = clipboard::handle_clipboard_key(&key, editor);
    if !consumed {
        dialog_handler::input_with_eof_jump(editor, key);
    }
    if let Some(msg) = warning {
        app.set_status(msg, StatusLevel::Warning);
    }
    None
}

pub(crate) fn insert_text(app: &mut App, text: &str) {
    if let Some(ref mut editor) = app.scenario_editor {
        editor.insert_str(text);
    }
}

pub(crate) fn scroll_by(app: &mut App, rows: i16) -> bool {
    if let Some(ref mut editor) = app.scenario_editor {
        editor.scroll((rows, 0));
        return true;
    }
    false
}

pub(crate) fn handle_mouse_click(app: &mut App, screen_col: u16, screen_row: u16) {
    let Ok((tw, th)) = crossterm::terminal::size() else {
        return;
    };
    let terminal_area = Rect::new(0, 0, tw, th);
    let dialog = dialog_rect(terminal_area);
    let inside = dialog.contains(ratatui::layout::Position::new(screen_col, screen_row));

    let Some(ref mut editor) = app.scenario_editor else {
        return;
    };

    if inside {
        let editor_rect = editor_area(dialog);
        let scroll_top = app.scenario_scroll_top;
        editor.cancel_selection();
        events::move_textarea_cursor_to_mouse(
            editor,
            editor_rect,
            scroll_top,
            screen_col,
            screen_row,
        );
    } else {
        stage_provisional_and_close(app);
    }
}

fn stage_provisional_and_close(app: &mut App) {
    let content = app
        .scenario_editor
        .take()
        .map(|e| e.lines().join("\n"))
        .unwrap_or_default();
    let trimmed = content.trim();
    let provisional = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    };
    if let Some(dlg) = app.chat_settings_dialog.as_mut() {
        dlg.set_provisional_scenario(provisional);
    }
    app.scenario_scroll_top = 0;
    app.invalidate_chat_caches();
    app.focus = Focus::ChatSettingsDialog;
}

pub(crate) fn handle_mouse_drag(app: &mut App, screen_col: u16, screen_row: u16) {
    let Some(ref mut editor) = app.scenario_editor else {
        return;
    };
    let Ok((tw, th)) = crossterm::terminal::size() else {
        return;
    };
    let terminal_area = Rect::new(0, 0, tw, th);
    let dialog = dialog_rect(terminal_area);
    let editor_rect = editor_area(dialog);
    let scroll_top = app.scenario_scroll_top;
    if editor.selection_range().is_none() {
        editor.start_selection();
    }
    events::move_textarea_cursor_to_mouse(editor, editor_rect, scroll_top, screen_col, screen_row);
}
