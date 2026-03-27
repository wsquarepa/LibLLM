use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let width = (area.width as f32 * 0.7) as u16;
    let height = (area.height as f32 * 0.6) as u16;
    let dialog = centered_rect(width, height, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let border = Block::default()
        .borders(Borders::ALL)
        .title(" Edit Message (Esc to cancel, Enter to send) ")
        .border_style(Style::default().fg(Color::Yellow));
    f.render_widget(border, dialog);

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
        let hint = Paragraph::new(Line::from(Span::styled(
            "Enter: send edited message  Alt+Enter: newline  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, hint_area);
    }
}

pub(in crate::tui) fn handle_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if let Some(ref mut editor) = app.edit_editor {
        match key.code {
            KeyCode::Esc => {
                app.edit_editor = None;
                app.focus = Focus::Input;
                app.status_message = "Edit cancelled.".to_owned();
            }
            KeyCode::Enter
                if !key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::ALT) =>
            {
                let content = editor.lines().join("\n").trim().to_owned();
                app.edit_editor = None;
                app.focus = Focus::Input;
                if content.is_empty() {
                    app.status_message = "Edit cancelled (empty message).".to_owned();
                } else {
                    return Some(Action::EditMessage(content));
                }
            }
            _ => {
                editor.input(key);
            }
        }
    }
    None
}
