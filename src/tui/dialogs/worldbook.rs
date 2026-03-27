use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus};

enum WorldbookState {
    Off,
    Session,
    Global,
}

fn worldbook_state(app: &App, name: &str) -> WorldbookState {
    if app.config.worldbooks.contains(&name.to_owned()) {
        WorldbookState::Global
    } else if app.session.worldbooks.contains(&name.to_owned()) {
        WorldbookState::Session
    } else {
        WorldbookState::Off
    }
}

pub(in crate::tui) fn render_worldbook_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.worldbook_list.len();
    let dialog = centered_rect(50, count as u16 + 6, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.worldbook_list.iter().enumerate() {
        let is_selected = i == app.worldbook_selected;
        let state = worldbook_state(app, name);
        let (checkbox, color) = match state {
            WorldbookState::Global => ("[G]", Color::Green),
            WorldbookState::Session => ("[S]", Color::Cyan),
            WorldbookState::Off => ("[ ]", Color::Reset),
        };
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checkbox} {name}"),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [G] Global  [S] Session  [ ] Off",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: cycle  Esc: close",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Worldbooks ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_worldbook_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook_list.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.worldbook_selected = app.worldbook_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.worldbook_selected =
                (app.worldbook_selected + 1).min(app.worldbook_list.len() - 1);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            match worldbook_state(app, &name) {
                WorldbookState::Off => {
                    app.session.worldbooks.push(name.clone());
                    let _ = app.session.maybe_save(&app.save_mode);
                    app.status_message = format!("Session: {name}");
                }
                WorldbookState::Session => {
                    app.session.worldbooks.retain(|n| n != &name);
                    app.config.worldbooks.push(name.clone());
                    let _ = app.session.maybe_save(&app.save_mode);
                    let _ = crate::config::save(&app.config);
                    app.status_message = format!("Global: {name}");
                }
                WorldbookState::Global => {
                    app.config.worldbooks.retain(|n| n != &name);
                    let _ = crate::config::save(&app.config);
                    app.status_message = format!("Disabled: {name}");
                }
            }
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}
