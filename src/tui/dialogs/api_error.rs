use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_api_error_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(f, 60, 8, area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Could not connect to API server",
            Style::default().fg(Color::Red),
        )),
        Line::from(""),
        Line::from(format!("  {}", app.api_error)),
        Line::from(""),
        Line::from(Span::styled(
            "  You can browse existing chats but cannot send messages.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Enter or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" API Error ", Color::Red));

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_api_error_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

pub(in crate::tui) fn render_loading_dialog(f: &mut ratatui::Frame, area: Rect) {
    let dialog = clear_centered(f, 40, 5, area);

    let lines = vec![
        Line::from(""),
        Line::from("  Connecting to API server..."),
        Line::from(""),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Loading ", Color::Cyan));

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_loading_key(key: KeyEvent) -> Option<Action> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }
    None
}
