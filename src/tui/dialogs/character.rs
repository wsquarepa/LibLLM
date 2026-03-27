use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::session::{self, Message, Role};
use crate::tui::business::refresh_sidebar;
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_character_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.character_names.len();
    let dialog = centered_rect(50, count as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.character_names.iter().enumerate() {
        let is_selected = i == app.character_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{name}"),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: select  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Character ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_character_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.character_names.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.character_selected = app.character_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.character_selected =
                (app.character_selected + 1).min(app.character_names.len() - 1);
        }
        KeyCode::Enter => {
            let slug = app.character_slugs[app.character_selected].clone();
            let card_path = crate::config::characters_dir().join(format!("{slug}.json"));
            match crate::character::load_card(&card_path) {
                Ok(card) => {
                    app.session.tree.clear();
                    app.session.system_prompt =
                        Some(crate::character::build_system_prompt(&card));
                    app.session.character = Some(card.name.clone());
                    if !card.first_mes.is_empty() {
                        app.session.tree.push(
                            None,
                            Message::new(Role::Assistant, card.first_mes),
                        );
                    }
                    app.chat_scroll = 0;
                    app.auto_scroll = true;
                    let new_path = crate::config::sessions_dir()
                        .join(session::generate_session_name());
                    app.save_mode.set_path(new_path);
                    app.status_message = format!("Loaded character: {}", card.name);
                    app.focus = Focus::Input;
                    refresh_sidebar(app);
                }
                Err(e) => {
                    app.status_message = format!("Error: {e}");
                    app.focus = Focus::Input;
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
