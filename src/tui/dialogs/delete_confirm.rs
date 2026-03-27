use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{business, Action, App, Focus};

pub(in crate::tui) fn render_delete_confirm_dialog(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
) {
    let dialog = centered_rect(50, 7, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let cancel_style = if app.delete_confirm_selected == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let delete_style = if app.delete_confirm_selected == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let lines = vec![
        Line::from(""),
        Line::from(format!(
            "  Delete \"{}\"?",
            app.delete_confirm_filename
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(" Delete ", delete_style),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Left/Right: navigate  Enter: confirm  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirm Delete ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_delete_confirm_key(
    key: KeyEvent,
    app: &mut App,
) -> Option<Action> {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            app.delete_confirm_selected = 1 - app.delete_confirm_selected;
        }
        KeyCode::Enter => {
            if app.delete_confirm_selected == 1 {
                delete_selected_session(app);
            }
            app.focus = Focus::Sidebar;
        }
        KeyCode::Esc => {
            app.focus = Focus::Sidebar;
        }
        _ => {}
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
        app.status_message = format!("Error deleting: {e}");
        return;
    }

    if is_current {
        *app.session = crate::session::Session::default();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path =
            crate::config::sessions_dir().join(crate::session::generate_session_name());
        app.save_mode.set_path(new_path);
    }

    business::refresh_sidebar(app);
    app.status_message = format!("Deleted: {filename}");
}
