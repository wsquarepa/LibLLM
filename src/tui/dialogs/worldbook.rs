use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_worldbook_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.worldbook_list.len();
    let dialog = centered_rect(50, count as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.worldbook_list.iter().enumerate() {
        let is_selected = i == app.worldbook_selected;
        let is_active = app.session.worldbooks.contains(name);
        let checkbox = if is_active { "[x]" } else { "[ ]" };
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checkbox} {name}"),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: toggle  Esc: close",
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
            if app.session.worldbooks.contains(&name) {
                app.session.worldbooks.retain(|n| n != &name);
                app.status_message = format!("Disabled: {name}");
            } else {
                app.session.worldbooks.push(name.clone());
                app.status_message = format!("Enabled: {name}");
            }
            let _ = app.session.maybe_save(&app.save_mode);
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}
