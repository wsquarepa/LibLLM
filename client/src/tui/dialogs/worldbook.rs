//! Worldbook picker and entry editor dialog with session/global toggle.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, DeleteContext, Focus};

enum WorldbookState {
    Off,
    Session,
    Global,
}

fn worldbook_state(app: &App, name: &str) -> WorldbookState {
    if app.config.worldbooks.iter().any(|n| n == name) {
        WorldbookState::Global
    } else if app.session.worldbooks.iter().any(|n| n == name) {
        WorldbookState::Session
    } else {
        WorldbookState::Off
    }
}

pub(in crate::tui) fn render_worldbook_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.worldbook_list.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING, false);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let items: Vec<ListItem<'_>> = app
        .worldbook_list
        .iter()
        .map(|name| {
            let state = worldbook_state(app, name);
            let (checkbox, color) = match state {
                WorldbookState::Global => ("[G]", Color::Green),
                WorldbookState::Session => ("[S]", Color::Cyan),
                WorldbookState::Off => ("[ ]", Color::Reset),
            };
            let line = Line::from(Span::styled(
                format!("{checkbox} {name}"),
                Style::default().fg(color),
            ));
            ListItem::new(line)
        })
        .collect();

    super::render_paged_list(f, dialog, app.worldbook_selected, items, " Worldbooks ", &app.theme, None, None);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("[G] Global  [S] Session  [ ] Off"),
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: cycle  Right: edit  a: add  Del: delete  Esc: close"),
            Line::from("Drop .json to import"),
        ],
    );
}

pub(in crate::tui) fn handle_worldbook_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook_list.is_empty() {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_worldbook(app);
            }
            KeyCode::Esc => {
                app.focus = Focus::Input;
            }
            _ => {}
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::LIST_DIALOG_TALL_PADDING);
    if super::handle_paged_list_key(
        &mut app.worldbook_selected,
        &app.worldbook_list,
        visible,
        key,
        None,
    ) == super::PagedListAction::Consumed
    {
        return None;
    }

    match key.code {
        KeyCode::Enter | KeyCode::Char(' ') => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            match worldbook_state(app, &name) {
                WorldbookState::Off => {
                    app.session.worldbooks.push(name.clone());
                    app.invalidate_worldbook_cache();
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
                }
                WorldbookState::Session => {
                    app.session.worldbooks.retain(|n| n != &name);
                    app.config.worldbooks.push(name.clone());
                    app.invalidate_worldbook_cache();
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
                    if let Err(e) = libllm::config::save(&app.config) {
                        app.set_status(
                            format!("Failed to save config: {e}"),
                            super::super::StatusLevel::Error,
                        );
                    }
                }
                WorldbookState::Global => {
                    app.config.worldbooks.retain(|n| n != &name);
                    app.invalidate_worldbook_cache();
                    if let Err(e) = libllm::config::save(&app.config) {
                        app.set_status(
                            format!("Failed to save config: {e}"),
                            super::super::StatusLevel::Error,
                        );
                    }
                }
            }
        }
        KeyCode::Right => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            let slug = libllm::character::slugify(&name);
            match app.db.as_ref().and_then(|db| db.load_worldbook(&slug).ok()) {
                Some(wb) => {
                    app.worldbook_editor_original_name = wb.name.clone();
                    app.worldbook_editor_original_entries = wb.entries.clone();
                    app.worldbook_editor_entries = wb.entries;
                    app.worldbook_editor_name = wb.name;
                    app.worldbook_editor_name_selected = true;
                    app.worldbook_editor_name_editing = false;
                    app.worldbook_editor_selected = 0;
                    app.focus = Focus::WorldbookEditorDialog;
                }
                None => {
                    app.set_status("Worldbook not found.".to_owned(), super::super::StatusLevel::Error);
                }
            }
        }
        KeyCode::Char('a') => {
            create_and_edit_worldbook(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            app.delete_confirm_filename = name.clone();
            app.delete_confirm_selected = 0;
            app.delete_context = DeleteContext::Worldbook { name };
            app.focus = Focus::DeleteConfirmDialog;
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
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING + 2, false);
    let dialog = clear_centered(f, super::FIELD_DIALOG_DEFAULT_WIDTH, height, area);

    let title = format!(" Worldbook ({count} entries) ");
    f.render_widget(ratatui::widgets::Clear, dialog);
    f.render_widget(dialog_block(title, app.theme.border_focused), dialog);

    let name_selected = app.worldbook_editor_name_selected && !app.worldbook_editor_name_editing;
    let name_editing = app.worldbook_editor_name_editing;
    let name_marker = if name_selected || name_editing { "> " } else { "  " };
    let name_flashing = name_editing && super::is_flash_active(app.input_reject_flash);
    let name_style = if name_flashing {
        Style::default()
            .fg(app.theme.status_warning_bg)
            .add_modifier(Modifier::BOLD)
    } else if name_selected || name_editing {
        Style::default()
            .fg(app.theme.sidebar_highlight_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.theme.border_focused)
    };
    let name_display = if name_editing {
        format!("{name_marker}Name: {}_", app.worldbook_editor_name)
    } else {
        format!("{name_marker}Name: {}", app.worldbook_editor_name)
    };
    let name_row = Rect {
        x: dialog.x + 1,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(2),
        height: 1,
    };
    f.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(Span::styled(name_display, name_style))),
        name_row,
    );

    // Entries area is the rest of the inner dialog, below the name row and spacer.
    let list_area = Rect {
        x: dialog.x + 1,
        y: dialog.y + 3,
        width: dialog.width.saturating_sub(2),
        height: dialog.height.saturating_sub(4),
    };

    let items: Vec<ListItem<'_>> = app
        .worldbook_editor_entries
        .iter()
        .map(|entry| {
            let enabled = if entry.enabled { "+" } else { "-" };
            let label = if entry.keys.is_empty() {
                format!("[{enabled}] (no keys)")
            } else {
                let keys_str = entry.keys.join(", ");
                let truncated = if keys_str.len() > 40 {
                    let end = keys_str[..40]
                        .char_indices()
                        .last()
                        .map_or(0, |(i, c)| i + c.len_utf8());
                    format!("{}...", &keys_str[..end])
                } else {
                    keys_str
                };
                format!("[{enabled}] {truncated}")
            };
            let row_style = if entry.enabled {
                Style::default()
            } else {
                Style::default().fg(app.theme.dimmed)
            };
            ListItem::new(Line::from(Span::styled(label, row_style)))
        })
        .collect();

    // Name row focus is signaled by usize::MAX so no entry row highlights.
    let effective_selected = if app.worldbook_editor_name_selected {
        usize::MAX
    } else {
        app.worldbook_editor_selected
    };

    super::render_paged_list_inline(f, list_area, effective_selected, items, &app.theme, None);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Right/Enter: edit  a: add  Del: delete  Esc: save & close"),
        ],
    );
}

pub(in crate::tui) fn handle_worldbook_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook_editor_name_editing {
        match key.code {
            KeyCode::Char(c) => {
                if app.worldbook_editor_name.chars().count() < super::MAX_NAME_LENGTH {
                    app.worldbook_editor_name.push(c);
                } else {
                    app.input_reject_flash = Some(std::time::Instant::now());
                }
            }
            KeyCode::Backspace => {
                app.worldbook_editor_name.pop();
            }
            KeyCode::Enter | KeyCode::Esc => {
                app.worldbook_editor_name_editing = false;
            }
            _ => {}
        }
        return None;
    }

    if app.worldbook_editor_name_selected {
        match key.code {
            KeyCode::Down if !app.worldbook_editor_entries.is_empty() => {
                app.worldbook_editor_name_selected = false;
                app.worldbook_editor_selected = 0;
            }
            KeyCode::Right | KeyCode::Enter => {
                app.worldbook_editor_name_editing = true;
            }
            KeyCode::Char('a') => {
                app.worldbook_editor_name_selected = false;
                add_new_entry(app);
            }
            KeyCode::Esc => {
                save_worldbook_editor(app);
                app.focus = Focus::WorldbookDialog;
            }
            _ => {}
        }
        return None;
    }

    if app.worldbook_editor_entries.is_empty() {
        match key.code {
            KeyCode::Up => {
                app.worldbook_editor_name_selected = true;
            }
            KeyCode::Esc => {
                save_worldbook_editor(app);
                app.focus = Focus::WorldbookDialog;
            }
            KeyCode::Char('a') => {
                add_new_entry(app);
            }
            _ => {}
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            if app.worldbook_editor_selected == 0 {
                app.worldbook_editor_name_selected = true;
            } else {
                let visible = super::page_size(
                    app.last_terminal_height,
                    super::LIST_DIALOG_TALL_PADDING + 2,
                );
                let labels: Vec<String> = app
                    .worldbook_editor_entries
                    .iter()
                    .map(|entry| entry.keys.join(", "))
                    .collect();
                super::handle_paged_list_key(
                    &mut app.worldbook_editor_selected,
                    &labels,
                    visible,
                    key,
                    None,
                );
            }
        }
        KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => {
            let visible = super::page_size(
                app.last_terminal_height,
                super::LIST_DIALOG_TALL_PADDING + 2,
            );
            let labels: Vec<String> = app
                .worldbook_editor_entries
                .iter()
                .map(|entry| entry.keys.join(", "))
                .collect();
            super::handle_paged_list_key(
                &mut app.worldbook_editor_selected,
                &labels,
                visible,
                key,
                None,
            );
        }
        KeyCode::Right | KeyCode::Enter => {
            let idx = app.worldbook_editor_selected;
            let entry = &app.worldbook_editor_entries[idx];
            open_entry_editor(app, idx, entry_to_values(entry), entry.selective);
        }
        KeyCode::Char('a') => {
            add_new_entry(app);
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

fn create_and_edit_worldbook(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.worldbook_list.iter().cloned().collect();
    let new_name = super::generate_unique_name("worldbook", &existing);
    let wb = libllm::worldinfo::WorldBook {
        name: new_name.clone(),
        entries: Vec::new(),
    };
    let slug = libllm::character::slugify(&new_name);
    if let Err(e) = app.db.as_ref().map(|db| db.insert_worldbook(&slug, &wb)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
        app.set_status(
            format!("Failed to create worldbook: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.worldbook_list.push(new_name.clone());
    app.worldbook_selected = app.worldbook_list.len() - 1;
    app.worldbook_editor_entries = Vec::new();
    app.worldbook_editor_original_name = new_name.clone();
    app.worldbook_editor_original_entries = Vec::new();
    app.worldbook_editor_name = new_name;
    app.worldbook_editor_selected = 0;
    app.worldbook_editor_name_selected = true;
    app.focus = Focus::WorldbookEditorDialog;
}

fn add_new_entry(app: &mut App) {
    let new_entry = libllm::worldinfo::Entry {
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

fn open_entry_editor(app: &mut App, idx: usize, values: Vec<String>, selective: bool) {
    app.worldbook_entry_editor = Some(if selective {
        super::open_entry_editor(values)
    } else {
        super::open_entry_editor_non_selective(values)
    });
    app.worldbook_entry_editor_index = idx;
    app.focus = Focus::WorldbookEntryEditorDialog;
}

fn entry_to_values(entry: &libllm::worldinfo::Entry) -> Vec<String> {
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

pub fn values_to_entry(
    values: &[String],
    existing: &libllm::worldinfo::Entry,
) -> libllm::worldinfo::Entry {
    let parse_keys = |s: &str| -> Vec<String> {
        s.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    };
    libllm::worldinfo::Entry {
        keys: parse_keys(&values[0]),
        content: values[1].clone(),
        selective: values[2].eq_ignore_ascii_case("true"),
        secondary_keys: parse_keys(&values[3]),
        constant: values[4].eq_ignore_ascii_case("true"),
        enabled: values[5].eq_ignore_ascii_case("true"),
        order: values[6].parse().unwrap_or(existing.order),
        depth: values[7].parse().unwrap_or(existing.depth),
        case_sensitive: values[8].eq_ignore_ascii_case("true"),
    }
}

pub(in crate::tui) fn render_entry_delete_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    super::delete_confirm::render_confirm_dialog(
        f,
        area,
        &format!("Delete {}?", app.delete_confirm_filename),
        app.delete_confirm_selected,
    );
}

pub(in crate::tui) fn handle_entry_delete_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match super::delete_confirm::handle_confirm_key(key, &mut app.delete_confirm_selected) {
        super::delete_confirm::ConfirmResult::Confirmed => {
            let idx = app.worldbook_editor_selected;
            app.worldbook_editor_entries.remove(idx);
            if app.worldbook_editor_selected >= app.worldbook_editor_entries.len()
                && app.worldbook_editor_selected > 0
            {
                app.worldbook_editor_selected -= 1;
            }
            app.focus = Focus::WorldbookEditorDialog;
        }
        super::delete_confirm::ConfirmResult::Cancelled => {
            app.focus = Focus::WorldbookEditorDialog;
        }
        super::delete_confirm::ConfirmResult::Pending => {}
    }
    None
}

fn save_worldbook_editor(app: &mut App) {
    let original = app.worldbook_editor_original_name.clone();
    let new_name = app.worldbook_editor_name.clone();

    if original == new_name && app.worldbook_editor_entries == app.worldbook_editor_original_entries
    {
        app.set_status(
            "No changes found.".to_owned(),
            super::super::StatusLevel::Info,
        );
        return;
    }

    if original != new_name && app.worldbook_list.iter().any(|n| n == &new_name) {
        app.set_status(
            format!("Name '{new_name}' is already in use."),
            super::super::StatusLevel::Error,
        );
        return;
    }

    let wb = libllm::worldinfo::WorldBook {
        name: new_name.clone(),
        entries: app.worldbook_editor_entries.clone(),
    };
    let slug = libllm::character::slugify(&new_name);
    let old_slug = libllm::character::slugify(&original);
    let is_rename = !original.is_empty() && original != new_name;
    let save_result = if is_rename {
        app.db.as_ref()
            .map(|db| db.insert_worldbook(&slug, &wb))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    } else {
        app.db.as_ref().map(|db| {
            if db.load_worldbook(&slug).is_ok() {
                db.update_worldbook(&slug, &wb)
            } else {
                db.insert_worldbook(&slug, &wb)
            }
        }).unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    };
    match save_result {
        Ok(()) => {
            if is_rename {
                if let Some(pos) = app.session.worldbooks.iter().position(|n| n == &original) {
                    app.session.worldbooks[pos] = new_name.clone();
                }
                if let Some(pos) = app.config.worldbooks.iter().position(|n| n == &original) {
                    app.config.worldbooks[pos] = new_name.clone();
                    let _ = libllm::config::save(&app.config);
                }
                if let Some(db) = app.db.as_ref() {
                    let _ = db.delete_worldbook(&old_slug);
                }
            }
            app.invalidate_worldbook_cache();
            let books = app.db.as_ref().and_then(|db| db.list_worldbooks().ok()).unwrap_or_default();
            app.worldbook_list = books.into_iter().map(|(_, n)| n).collect();
            app.worldbook_selected = app
                .worldbook_list
                .iter()
                .position(|n| n == &new_name)
                .unwrap_or(0);
            app.set_status(
                format!("Saved worldbook: {}", wb.name),
                super::super::StatusLevel::Info,
            )
        }
        Err(e) => app.set_status(
            format!("Failed to save worldbook: {e}"),
            super::super::StatusLevel::Error,
        ),
    }
}

pub(in crate::tui) fn handle_worldbook_paste(
    path: &std::path::Path,
    ext: &str,
    app: &mut App,
) -> bool {
    if ext != "json" {
        app.set_status(
            "Worldbook import supports .json files only.".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return true;
    }

    let fallback_name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    match std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!(e))
        .and_then(|s| libllm::worldinfo::parse_worldbook_json(&s, &fallback_name)) {
        Ok(wb) => {
            if wb.name.chars().count() > super::MAX_NAME_LENGTH {
                app.set_status(
                    format!(
                        "Worldbook name exceeds {} characters: \"{}\"",
                        super::MAX_NAME_LENGTH,
                        wb.name,
                    ),
                    super::super::StatusLevel::Error,
                );
                return true;
            }
            let name = wb.name.clone();
            let slug = libllm::character::slugify(&name);
            match app.db.as_ref().map(|db| db.insert_worldbook(&slug, &wb)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
                Ok(()) => {
                    let books = app.db.as_ref().and_then(|db| db.list_worldbooks().ok()).unwrap_or_default();
                    app.worldbook_list = books.into_iter().map(|(_, n)| n).collect();
                    app.worldbook_selected = 0;
                    app.invalidate_worldbook_cache();
                    app.set_status(
                        format!("Imported worldbook: {name}"),
                        super::super::StatusLevel::Info,
                    );
                }
                Err(e) => {
                    app.set_status(format!("Save error: {e}"), super::super::StatusLevel::Error);
                }
            }
        }
        Err(e) => {
            app.set_status(
                format!("Import error: {e}"),
                super::super::StatusLevel::Error,
            );
        }
    }
    true
}
