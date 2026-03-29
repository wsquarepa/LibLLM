use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let width = (area.width as f32 * 0.7) as u16;
    let height = (area.height as f32 * 0.6) as u16;
    let dialog = clear_centered(f, width, height, area);

    f.render_widget(dialog_block(" Edit Message ", Color::Yellow), dialog);

    if let Some(ref editor) = app.edit_editor {
        let editor_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(4),
            height: dialog.height.saturating_sub(3),
        };
        f.render_widget(editor, editor_area);

        let hint_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + dialog.height - 2,
            width: dialog.width.saturating_sub(4),
            height: 1,
        };
        let hint_widget = Paragraph::new(Line::from(Span::styled(
            "Alt+Enter: save edit  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint_widget, hint_area);
    }
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
