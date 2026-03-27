use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_system_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let width = (area.width as f32 * 0.7) as u16;
    let height = (area.height as f32 * 0.6) as u16;
    let dialog = centered_rect(width, height, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let border = Block::default()
        .borders(Borders::ALL)
        .title(" System Prompt (Esc to save & close) ")
        .border_style(Style::default().fg(Color::Yellow));
    f.render_widget(border, dialog);

    if let Some(ref editor) = app.system_editor {
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
            "Esc: save & close",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, hint_area);
    }
}

pub(in crate::tui) fn handle_system_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if let Some(ref mut editor) = app.system_editor {
        match key.code {
            KeyCode::Esc => {
                let content = editor.lines().join("\n");
                if content.trim().is_empty() {
                    app.session.system_prompt = None;
                } else {
                    app.session.system_prompt = Some(content);
                }
                app.system_editor = None;
                app.focus = Focus::Input;
                app.status_message = "System prompt updated.".to_owned();
                let _ = app.session.maybe_save(&app.save_mode);
            }
            _ => {
                editor.input(key);
            }
        }
    }
    None
}
