use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
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
        f.render_widget(editor, editor_area);
    }

    render_hints_below_dialog(f, dialog, area, &[
        Line::from("Alt+Enter: save edit  Esc: cancel"),
    ]);
}

pub(in crate::tui) fn handle_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let Some(ref mut editor) = app.edit_editor else {
        return None;
    };

    let is_confirm = key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT);

    if key.code == KeyCode::Esc {
        app.edit_editor = None;
        app.raw_edit_node = None;
        app.focus = Focus::Chat;
        return None;
    }

    if is_confirm {
        let content = editor.lines().join("\n").trim().to_owned();
        let node_id = app.raw_edit_node.take();
        app.edit_editor = None;

        if content.is_empty() {
            app.focus = Focus::Chat;
            return None;
        }

        return match node_id {
            Some(id) => Some(Action::EditMessage {
                node_id: id,
                content,
            }),
            None => {
                app.focus = Focus::Chat;
                None
            }
        };
    }

    editor.input(key);
    None
}
