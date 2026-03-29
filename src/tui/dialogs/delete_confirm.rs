use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus, business};

pub(in crate::tui) enum ConfirmResult {
    Confirmed,
    Cancelled,
    Pending,
}

pub(in crate::tui) fn render_confirm_dialog(
    f: &mut ratatui::Frame,
    area: Rect,
    prompt: &str,
    hint: Option<&str>,
    selected: usize,
) {
    let height = if hint.is_some() { 7 } else { 6 };
    let dialog = centered_rect(50, height, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let cancel_style = if selected == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let delete_style = if selected == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(format!("  {prompt}")),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(" Delete ", delete_style),
        ]),
        Line::from(""),
    ];

    if let Some(hint_text) = hint {
        lines.push(Line::from(Span::styled(
            format!("  {hint_text}"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirm Delete ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_confirm_key(key: KeyEvent, selected: &mut usize) -> ConfirmResult {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            *selected = 1 - *selected;
            ConfirmResult::Pending
        }
        KeyCode::Enter => {
            if *selected == 1 {
                ConfirmResult::Confirmed
            } else {
                ConfirmResult::Cancelled
            }
        }
        KeyCode::Esc => ConfirmResult::Cancelled,
        _ => ConfirmResult::Pending,
    }
}

pub(in crate::tui) fn render_delete_confirm_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    render_confirm_dialog(
        f,
        area,
        &format!("Delete \"{}\"?", app.delete_confirm_filename),
        Some("Left/Right: navigate  Enter: confirm  Esc: cancel"),
        app.delete_confirm_selected,
    );
}

pub(in crate::tui) fn handle_delete_confirm_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match handle_confirm_key(key, &mut app.delete_confirm_selected) {
        ConfirmResult::Confirmed => {
            delete_selected_session(app);
            app.focus = Focus::Sidebar;
        }
        ConfirmResult::Cancelled => {
            app.focus = Focus::Sidebar;
        }
        ConfirmResult::Pending => {}
    }
    None
}

fn delete_selected_session(app: &mut App) {
    let Some(selected) = app.sidebar_state.selected() else {
        return;
    };
    let entry = &app.sidebar_sessions[selected];
    if entry.is_new_chat {
        return;
    }

    let path = entry.path.clone();
    let filename = entry.filename.clone();
    let is_current = app.save_mode.path().is_some_and(|p| p == path);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    crate::index::warn_if_save_fails(
        crate::index::remove_session(&path),
        "failed to remove session index entry",
    );

    if is_current {
        app.discard_pending_session_save();
        *app.session = crate::session::Session::default();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path = crate::config::sessions_dir().join(crate::session::generate_session_name());
        app.save_mode.set_path(new_path);
    }

    business::refresh_sidebar(app);
    app.set_status(
        format!("Deleted: {filename}"),
        super::super::StatusLevel::Info,
    );
}
