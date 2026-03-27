use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{FieldDialog, centered_rect};
use crate::tui::{Action, App, Focus};

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys [OR]",
    "Content",
    "Selective",
    "Keys [AND]",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];

const ENTRY_EDITOR_MULTILINE: &[usize] = &[1];
const ENTRY_EDITOR_PLACEHOLDER_FIELDS: &[usize] = &[0, 3];

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
    let dialog = centered_rect(60, count as u16 + 7, area);
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
        "Up/Down: navigate  Right: edit  a: add  Del: delete",
        Style::default().fg(Color::DarkGray),
    )).alignment(ratatui::layout::Alignment::Center));
    lines.push(Line::from(Span::styled(
        "Esc: save & close",
        Style::default().fg(Color::DarkGray),
    )).alignment(ratatui::layout::Alignment::Center));

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
            open_entry_editor(app, idx, entry_to_values(entry), entry.selective);
        }
        KeyCode::Char('a') => {
            let new_entry = crate::worldinfo::Entry {
                keys: Vec::new(),
                secondary_keys: Vec::new(),
                selective: false,
                content: String::new(),
                constant: false,
                enabled: true,
                order: 10,
                depth: 4,
                case_sensitive: false,
            };
            app.worldbook_editor_entries.push(new_entry);
            let idx = app.worldbook_editor_entries.len() - 1;
            app.worldbook_editor_selected = idx;
            let entry = &app.worldbook_editor_entries[idx];
            open_entry_editor(app, idx, entry_to_values(entry), entry.selective);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let idx = app.worldbook_editor_selected;
            let entry = &app.worldbook_editor_entries[idx];
            let content_lines = entry.content.lines().count();
            let keys_desc = if entry.keys.is_empty() {
                "(no keys)".to_owned()
            } else {
                entry.keys.join(", ")
            };
            app.delete_confirm_filename = format!("{keys_desc} ({content_lines} lines)");
            app.delete_confirm_selected = 0;
            app.focus = Focus::WorldbookEntryDeleteDialog;
        }
        KeyCode::Esc => {
            save_worldbook_editor(app);
            app.focus = Focus::WorldbookDialog;
        }
        _ => {}
    }
    None
}

fn open_entry_editor(app: &mut App, idx: usize, values: Vec<String>, selective: bool) {
    let mut dialog = FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    ).with_size(70, 60).with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS);
    if !selective {
        dialog.hidden_fields = vec![3];
    }
    app.worldbook_entry_editor = Some(dialog);
    app.worldbook_entry_editor_index = idx;
    app.focus = Focus::WorldbookEntryEditorDialog;
}

fn entry_to_values(entry: &crate::worldinfo::Entry) -> Vec<String> {
    vec![
        entry.keys.join(", "),
        entry.content.clone(),
        entry.selective.to_string(),
        entry.secondary_keys.join(", "),
        entry.constant.to_string(),
        entry.enabled.to_string(),
        entry.order.to_string(),
        entry.depth.to_string(),
        entry.case_sensitive.to_string(),
    ]
}

pub fn values_to_entry(values: &[String], existing: &mut crate::worldinfo::Entry) {
    let parse_keys = |s: &str| -> Vec<String> {
        s.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
    };
    existing.keys = parse_keys(&values[0]);
    existing.content = values[1].clone();
    existing.selective = values[2].eq_ignore_ascii_case("true");
    existing.secondary_keys = parse_keys(&values[3]);
    existing.constant = values[4].eq_ignore_ascii_case("true");
    existing.enabled = values[5].eq_ignore_ascii_case("true");
    existing.order = values[6].parse().unwrap_or(existing.order);
    existing.depth = values[7].parse().unwrap_or(existing.depth);
    existing.case_sensitive = values[8].eq_ignore_ascii_case("true");
}

pub(in crate::tui) fn render_entry_delete_dialog(
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
            "  Delete {}?",
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
    ];

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirm Delete ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_entry_delete_key(
    key: KeyEvent,
    app: &mut App,
) -> Option<Action> {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            app.delete_confirm_selected = 1 - app.delete_confirm_selected;
        }
        KeyCode::Enter => {
            if app.delete_confirm_selected == 1 {
                let idx = app.worldbook_editor_selected;
                app.worldbook_editor_entries.remove(idx);
                if app.worldbook_editor_selected >= app.worldbook_editor_entries.len()
                    && app.worldbook_editor_selected > 0
                {
                    app.worldbook_editor_selected -= 1;
                }
            }
            app.focus = Focus::WorldbookEditorDialog;
        }
        KeyCode::Esc => {
            app.focus = Focus::WorldbookEditorDialog;
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
