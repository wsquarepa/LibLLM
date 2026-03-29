use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{FieldDialog, centered_rect};
use crate::session::{self, Message, Role};
use crate::tui::business::refresh_sidebar;
use crate::tui::{Action, App, Focus};

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
];

const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7];

pub(in crate::tui) fn render_character_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.character_names.len();
    let dialog = centered_rect(50, count as u16 + 5, area);
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
        lines.push(Line::from(Span::styled(format!("{marker}{name}"), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: select  Right: edit  Esc: cancel",
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
            if !app.flush_session_before_transition() {
                return None;
            }
            let slug = app.character_slugs[app.character_selected].clone();
            let card_path =
                crate::character::resolve_card_path(&crate::config::characters_dir(), &slug);
            match crate::character::load_card(&card_path, app.save_mode.key()) {
                Ok(card) => {
                    app.discard_pending_session_save();
                    app.session.tree.clear();
                    app.session.system_prompt = Some(crate::character::build_system_prompt(&card));
                    app.session.character = Some(card.name.clone());
                    app.invalidate_chat_cache();
                    app.invalidate_worldbook_cache();
                    if !card.first_mes.is_empty() {
                        app.session
                            .tree
                            .push(None, Message::new(Role::Assistant, card.first_mes));
                    }
                    app.chat_scroll = 0;
                    app.auto_scroll = true;
                    let new_path =
                        crate::config::sessions_dir().join(session::generate_session_name());
                    app.save_mode.set_path(new_path);
                    app.mark_session_dirty_debounced();
                    app.set_status(
                        format!("Loaded character: {}", card.name),
                        super::super::StatusLevel::Info,
                    );
                    app.focus = Focus::Input;
                    refresh_sidebar(app);
                }
                Err(e) => {
                    app.set_status(format!("Error: {e}"), super::super::StatusLevel::Error);
                    app.focus = Focus::Input;
                }
            }
        }
        KeyCode::Right => {
            let slug = app.character_slugs[app.character_selected].clone();
            let card_path =
                crate::character::resolve_card_path(&crate::config::characters_dir(), &slug);
            match crate::character::load_card(&card_path, app.save_mode.key()) {
                Ok(card) => {
                    let values = vec![
                        card.name,
                        card.description,
                        card.personality,
                        card.scenario,
                        card.first_mes,
                        card.mes_example,
                        card.system_prompt,
                        card.post_history_instructions,
                    ];
                    app.character_editor = Some(
                        FieldDialog::new(
                            " Edit Character ",
                            CHARACTER_EDITOR_FIELDS,
                            values,
                            CHARACTER_EDITOR_MULTILINE,
                        )
                        .with_size(70, 60),
                    );
                    app.character_editor_slug = slug;
                    app.focus = Focus::CharacterEditorDialog;
                }
                Err(e) => {
                    app.set_status(format!("Error: {e}"), super::super::StatusLevel::Error);
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
