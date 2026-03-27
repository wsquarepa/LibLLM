use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{FieldDialog, centered_rect};
use crate::tui::{Action, App, Focus};

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys",
    "Secondary Keys",
    "Content",
    "Selective",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];

const ENTRY_EDITOR_MULTILINE: &[usize] = &[2];

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
        "  Up/Down: navigate  Enter: cycle  Right: edit  Esc: close",
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
        KeyCode::Right => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            let wb_path = crate::worldinfo::resolve_worldbook_path(
                &crate::config::worldinfo_dir(), &name,
            );
            match crate::worldinfo::load_worldbook(&wb_path, app.save_mode.key()) {
                Ok(wb) => {
                    app.worldbook_editor_entries = wb.entries;
                    app.worldbook_editor_name = wb.name;
                    app.worldbook_editor_selected = 0;
                    app.focus = Focus::WorldbookEditorDialog;
                }
                Err(e) => {
                    app.status_message = format!("Error: {e}");
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

pub(in crate::tui) fn render_worldbook_editor(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.worldbook_editor_entries.len();
    let dialog = centered_rect(60, count as u16 + 6, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, entry) in app.worldbook_editor_entries.iter().enumerate() {
        let is_selected = i == app.worldbook_editor_selected;
        let marker = if is_selected { "> " } else { "  " };
        let enabled = if entry.enabled { "+" } else { "-" };
        let label = if entry.keys.is_empty() {
            format!("[{enabled}] (no keys)")
        } else {
            let keys_str = entry.keys.join(", ");
            let truncated = if keys_str.len() > 40 {
                format!("{}...", &keys_str[..40])
            } else {
                keys_str
            };
            format!("[{enabled}] {truncated}")
        };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if entry.enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{label}"),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Right: edit entry  Esc: save & close",
        Style::default().fg(Color::DarkGray),
    )));

    let title = format!(" {} ({} entries) ", app.worldbook_editor_name, count);
    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_worldbook_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook_editor_entries.is_empty() {
        if key.code == KeyCode::Esc {
            save_worldbook_editor(app);
            app.focus = Focus::WorldbookDialog;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.worldbook_editor_selected = app.worldbook_editor_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.worldbook_editor_selected =
                (app.worldbook_editor_selected + 1).min(app.worldbook_editor_entries.len() - 1);
        }
        KeyCode::Right | KeyCode::Enter => {
            let idx = app.worldbook_editor_selected;
            let entry = &app.worldbook_editor_entries[idx];
            let values = vec![
                entry.keys.join(", "),
                entry.secondary_keys.join(", "),
                entry.content.clone(),
                entry.selective.to_string(),
                entry.constant.to_string(),
                entry.enabled.to_string(),
                entry.order.to_string(),
                entry.depth.to_string(),
                entry.case_sensitive.to_string(),
            ];
            app.worldbook_entry_editor = Some(FieldDialog::new(
                " Edit Entry ",
                ENTRY_EDITOR_FIELDS,
                values,
                ENTRY_EDITOR_MULTILINE,
            ).with_size(70, 60));
            app.worldbook_entry_editor_index = idx;
            app.focus = Focus::WorldbookEntryEditorDialog;
        }
        KeyCode::Esc => {
            save_worldbook_editor(app);
            app.focus = Focus::WorldbookDialog;
        }
        _ => {}
    }
    None
}

fn save_worldbook_editor(app: &mut App) {
    let wb = crate::worldinfo::WorldBook {
        name: app.worldbook_editor_name.clone(),
        entries: app.worldbook_editor_entries.clone(),
    };
    match crate::worldinfo::save_worldbook(&wb, &crate::config::worldinfo_dir(), app.save_mode.key()) {
        Ok(_) => app.status_message = format!("Saved worldbook: {}", wb.name),
        Err(e) => app.status_message = format!("Failed to save worldbook: {e}"),
    }
}
