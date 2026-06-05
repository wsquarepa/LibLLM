//! Inline message editor dialog for modifying existing chat messages.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::{Action, App, Focus};

pub(crate) fn render_edit_dialog(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let width = (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16;
    let height = (area.height as f32 * super::DIALOG_HEIGHT_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    f.render_widget(dialog_block(" Edit Message ", Color::Yellow), dialog);

    if let Some(ref editor) = app.edit_editor {
        let editor_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(4),
            height: dialog.height.saturating_sub(2),
        };
        app.edit_scroll_top =
            crate::events::update_scroll_top(app.edit_scroll_top, editor, editor_area);
        f.render_widget(editor, editor_area);
    }

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from("Alt+Enter or Ctrl+S: save edit  Esc: cancel")],
    );
}

fn is_commit_key(key: &KeyEvent) -> bool {
    (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S')))
}

pub(crate) fn handle_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let editor = app.edit_editor.as_mut()?;

    let is_confirm = is_commit_key(&key);

    if key.code == KeyCode::Esc {
        let current_content = editor.lines().join("\n");
        if current_content == app.edit_original_content {
            app.edit_editor = None;
            app.raw_edit_node = None;
            app.focus = Focus::Chat;
        } else {
            app.unsaved_warning = Some(crate::dialogs::unsaved_warning::UnsavedWarningState::new(
                Focus::EditDialog,
            ));
            app.focus = Focus::UnsavedWarningDialog;
        }
        return None;
    }

    if is_confirm {
        let content = editor.lines().join("\n").trim().to_owned();
        let node_id = app.raw_edit_node.take();
        app.edit_editor = None;
        app.focus = Focus::Chat;

        if content.is_empty() {
            return None;
        }

        return node_id.map(|id| Action::EditMessage {
            node_id: id,
            content,
        });
    }

    let (consumed, warning) = crate::clipboard::handle_clipboard_key(&key, editor);
    if !consumed {
        crate::dialog_handler::input_with_eof_jump(editor, key);
    }
    if let Some(msg) = warning {
        app.set_status(msg, crate::StatusLevel::Warning);
    }
    None
}

pub(crate) fn commit_edit_dialog(app: &mut App) {
    let Some(ref editor) = app.edit_editor else {
        crate::dialog_handler::return_to_input(app);
        return;
    };
    let content = editor.lines().join("\n").trim().to_owned();
    let node_id = app.raw_edit_node.take();
    app.edit_editor = None;
    app.focus = Focus::Chat;
    if content.is_empty() {
        return;
    }
    if let Some(id) = node_id {
        crate::events::handle_edit_message(app, id, content);
    }
}

pub(crate) fn discard_edit_dialog(app: &mut App) {
    app.edit_editor = None;
    app.raw_edit_node = None;
    app.focus = Focus::Chat;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn alt_enter_is_commit() {
        assert!(is_commit_key(&key(KeyCode::Enter, KeyModifiers::ALT)));
    }

    #[test]
    fn ctrl_s_is_commit() {
        assert!(is_commit_key(&key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn ctrl_shift_s_is_commit() {
        assert!(is_commit_key(&key(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )));
    }

    #[test]
    fn plain_enter_is_not_commit() {
        assert!(!is_commit_key(&key(KeyCode::Enter, KeyModifiers::NONE)));
    }

    #[test]
    fn plain_s_is_not_commit() {
        assert!(!is_commit_key(&key(KeyCode::Char('s'), KeyModifiers::NONE)));
    }
}
