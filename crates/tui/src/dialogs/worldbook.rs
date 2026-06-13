//! Worldbook picker and entry editor dialog with session/global toggle.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::dialog_handler::return_to_input;
use crate::{Action, App, DeleteContext, Focus};

pub(crate) struct WorldbookUi<'a> {
    pub list: Vec<String>,
    pub list_selected: usize,
    pub editor_entries: Vec<libllm_core::worldinfo::Entry>,
    pub editor_original_entries: Vec<libllm_core::worldinfo::Entry>,
    pub editor_name: String,
    pub editor_original_name: String,
    pub editor_name_selected: bool,
    pub editor_name_editing: bool,
    pub editor_selected: usize,
    pub entry_editor: Option<super::FieldDialog<'a>>,
    pub entry_editor_index: usize,
}

fn is_save_shortcut(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S'))
}

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

pub(crate) fn render_worldbook_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let visible_indices = super::filter_indices(&app.worldbook.list, &app.dialog_search);
    let unfiltered_total = app.worldbook.list.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let filtered_selected =
        super::filtered_selection_position(&visible_indices, app.worldbook.list_selected)
            .unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| {
            let name = &app.worldbook.list[i];
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

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: filtered_selected,
            items,
            title_base: " Worldbooks ",
            search: Some(&app.dialog_search),
            unfiltered_total: Some(unfiltered_total),
        },
    );

    let hints = if app.dialog_search.active {
        vec![Line::from("Enter: apply  Esc: cancel  type to filter")]
    } else {
        vec![
            Line::from("[G] Global  [S] Session  [ ] Off"),
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from(
                "Enter: cycle  Right: edit  a: add  Del: delete  Ctrl+F: search  Esc: close",
            ),
            Line::from("Drop .json to import"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(crate) fn handle_worldbook_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook.list.is_empty() && !app.dialog_search.active {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_worldbook(app);
            }
            KeyCode::Esc => {
                return_to_input(app);
            }
            _ => {}
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::LIST_DIALOG_TALL_PADDING);
    let action = super::handle_paged_list_key(
        &mut app.worldbook.list_selected,
        &app.worldbook.list,
        visible,
        key,
        Some(&mut app.dialog_search),
    );
    if matches!(
        action,
        super::PagedListAction::Consumed
            | super::PagedListAction::EnteredSearch
            | super::PagedListAction::ExitedSearch
    ) {
        return None;
    }

    let visible_indices = super::filter_indices(&app.worldbook.list, &app.dialog_search);
    let Some(selected) = super::visible_selection(&visible_indices, app.worldbook.list_selected)
    else {
        if key.code == KeyCode::Char('a') {
            create_and_edit_worldbook(app);
        } else if key.code == KeyCode::Esc {
            return_to_input(app);
        }
        return None;
    };

    match key.code {
        KeyCode::Enter | KeyCode::Char(' ') => {
            let name = app.worldbook.list[selected].clone();
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
                    if let Err(e) = libllm_config::save(&app.config) {
                        app.set_status(
                            format!("Failed to save config: {e}"),
                            super::super::StatusLevel::Error,
                        );
                    }
                }
                WorldbookState::Global => {
                    app.config.worldbooks.retain(|n| n != &name);
                    app.invalidate_worldbook_cache();
                    if let Err(e) = libllm_config::save(&app.config) {
                        app.set_status(
                            format!("Failed to save config: {e}"),
                            super::super::StatusLevel::Error,
                        );
                    }
                }
            }
        }
        KeyCode::Right => {
            let name = app.worldbook.list[selected].clone();
            let slug = libllm_core::character::slugify(&name);
            match app.db.as_ref().and_then(|db| db.load_worldbook(&slug).ok()) {
                Some(wb) => {
                    app.worldbook.editor_original_name = wb.name.clone();
                    app.worldbook.editor_original_entries = wb.entries.clone();
                    app.worldbook.editor_entries = wb.entries;
                    app.worldbook.editor_name = wb.name;
                    app.worldbook.editor_name_selected = true;
                    app.worldbook.editor_name_editing = false;
                    app.worldbook.editor_selected = 0;
                    app.focus = Focus::WorldbookEditorDialog;
                }
                None => {
                    app.set_status(
                        "Worldbook not found.".to_owned(),
                        super::super::StatusLevel::Error,
                    );
                }
            }
        }
        KeyCode::Char('a') => {
            create_and_edit_worldbook(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.worldbook.list[selected].clone();
            app.delete_confirm.filename = name.clone();
            app.delete_confirm.selected = 0;
            app.delete_confirm.context = DeleteContext::Worldbook { name };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Esc => {
            return_to_input(app);
        }
        _ => {}
    }
    None
}

pub(crate) fn render_worldbook_editor(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let entry_labels: Vec<String> = app
        .worldbook
        .editor_entries
        .iter()
        .map(|entry| {
            let enabled = if entry.enabled { "+" } else { "-" };
            let keys_str = if entry.keys.is_empty() {
                "(no keys)".to_owned()
            } else {
                entry.keys.join(", ")
            };
            format!("[{enabled}] {keys_str}")
        })
        .collect();
    let visible_indices = super::filter_indices(&entry_labels, &app.dialog_search);
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING + 2);
    let dialog = clear_centered(f, super::FIELD_DIALOG_DEFAULT_WIDTH, height, area);

    let title = format!(" Worldbook ({} entries) ", entry_labels.len());
    f.render_widget(ratatui::widgets::Clear, dialog);
    let search_max = dialog.width.saturating_sub(2);
    let block =
        dialog_block(title, app.theme.border_focused).title_bottom(super::search_title_line(
            &app.dialog_search,
            app.theme.border_focused,
            &app.theme,
            search_max,
        ));
    f.render_widget(block, dialog);

    let name_selected = app.worldbook.editor_name_selected && !app.worldbook.editor_name_editing;
    let name_editing = app.worldbook.editor_name_editing;
    let name_marker = if name_selected || name_editing {
        "> "
    } else {
        "  "
    };
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
        format!("{name_marker}Name: {}_", app.worldbook.editor_name)
    } else {
        format!("{name_marker}Name: {}", app.worldbook.editor_name)
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

    let list_area = Rect {
        x: dialog.x + 1,
        y: dialog.y + 3,
        width: dialog.width.saturating_sub(2),
        height: dialog.height.saturating_sub(4),
    };

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| {
            let entry = &app.worldbook.editor_entries[i];
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

    let effective_selected = if app.worldbook.editor_name_selected {
        usize::MAX
    } else {
        visible_indices
            .iter()
            .position(|&i| i == app.worldbook.editor_selected)
            .unwrap_or(0)
    };

    super::render_paged_list_inline(f, list_area, effective_selected, items, &app.theme);

    let hints = if app.dialog_search.active {
        vec![Line::from("Enter: apply  Esc: cancel  type to filter")]
    } else {
        vec![
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from(
                "Right/Enter: edit  a: add  Del: delete  Ctrl+F: search  Ctrl+S: save  Esc: close",
            ),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(crate) fn handle_worldbook_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if is_save_shortcut(&key) {
        app.worldbook.editor_name_editing = false;
        app.dialog_search.deactivate_and_clear();
        save_worldbook_editor(app);
        app.focus = Focus::WorldbookDialog;
        return None;
    }
    let is_ctrl_f = key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL);
    if app.dialog_search.active || is_ctrl_f {
        let labels: Vec<String> = app
            .worldbook
            .editor_entries
            .iter()
            .map(|entry| {
                let enabled = if entry.enabled { "+" } else { "-" };
                let keys_str = if entry.keys.is_empty() {
                    "(no keys)".to_owned()
                } else {
                    entry.keys.join(", ")
                };
                format!("[{enabled}] {keys_str}")
            })
            .collect();
        let visible = super::page_size(
            app.last_terminal_height,
            super::LIST_DIALOG_TALL_PADDING + 2,
        );
        if is_ctrl_f && !app.dialog_search.active {
            app.worldbook.editor_name_selected = false;
        }
        let action = super::handle_paged_list_key(
            &mut app.worldbook.editor_selected,
            &labels,
            visible,
            key,
            Some(&mut app.dialog_search),
        );
        if matches!(
            action,
            super::PagedListAction::Consumed
                | super::PagedListAction::EnteredSearch
                | super::PagedListAction::ExitedSearch
        ) {
            return None;
        }
    }

    if app.worldbook.editor_name_editing {
        match key.code {
            KeyCode::Char(c) => {
                if app.worldbook.editor_name.chars().count() < super::MAX_NAME_LENGTH {
                    app.worldbook.editor_name.push(c);
                } else {
                    app.input_reject_flash = Some(std::time::Instant::now());
                }
            }
            KeyCode::Backspace => {
                app.worldbook.editor_name.pop();
            }
            KeyCode::Enter | KeyCode::Esc => {
                app.worldbook.editor_name_editing = false;
            }
            _ => {}
        }
        return None;
    }

    if app.worldbook.editor_name_selected {
        match key.code {
            KeyCode::Down if !app.worldbook.editor_entries.is_empty() => {
                app.worldbook.editor_name_selected = false;
                app.worldbook.editor_selected = 0;
            }
            KeyCode::Right | KeyCode::Enter => {
                app.worldbook.editor_name_editing = true;
            }
            KeyCode::Char('a') => {
                app.worldbook.editor_name_selected = false;
                add_new_entry(app);
            }
            KeyCode::Esc => close_worldbook_editor(app),
            _ => {}
        }
        return None;
    }

    if app.worldbook.editor_entries.is_empty() {
        match key.code {
            KeyCode::Up => {
                app.worldbook.editor_name_selected = true;
            }
            KeyCode::Esc => close_worldbook_editor(app),
            KeyCode::Char('a') => {
                add_new_entry(app);
            }
            _ => {}
        }
        return None;
    }

    let labels: Vec<String> = app
        .worldbook
        .editor_entries
        .iter()
        .map(|entry| {
            let enabled = if entry.enabled { "+" } else { "-" };
            let keys_str = if entry.keys.is_empty() {
                "(no keys)".to_owned()
            } else {
                entry.keys.join(", ")
            };
            format!("[{enabled}] {keys_str}")
        })
        .collect();

    match key.code {
        KeyCode::Up => {
            if app.worldbook.editor_selected == 0 && !app.dialog_search.is_filtering() {
                app.worldbook.editor_name_selected = true;
            } else {
                let visible = super::page_size(
                    app.last_terminal_height,
                    super::LIST_DIALOG_TALL_PADDING + 2,
                );
                super::handle_paged_list_key(
                    &mut app.worldbook.editor_selected,
                    &labels,
                    visible,
                    key,
                    Some(&mut app.dialog_search),
                );
            }
        }
        KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => {
            let visible = super::page_size(
                app.last_terminal_height,
                super::LIST_DIALOG_TALL_PADDING + 2,
            );
            super::handle_paged_list_key(
                &mut app.worldbook.editor_selected,
                &labels,
                visible,
                key,
                Some(&mut app.dialog_search),
            );
        }
        KeyCode::Right | KeyCode::Enter => {
            let idx = app.worldbook.editor_selected;
            let entry = &app.worldbook.editor_entries[idx];
            open_entry_editor(app, idx, entry_to_values(entry), entry.selective);
        }
        KeyCode::Char('a') => {
            add_new_entry(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let idx = app.worldbook.editor_selected;
            let entry = &app.worldbook.editor_entries[idx];
            let content_lines = entry.content.lines().count();
            let keys_desc = if entry.keys.is_empty() {
                "(no keys)".to_owned()
            } else {
                entry.keys.join(", ")
            };
            app.delete_confirm.filename = format!("{keys_desc} ({content_lines} lines)");
            app.delete_confirm.selected = 0;
            app.focus = Focus::WorldbookEntryDeleteDialog;
        }
        KeyCode::Esc => close_worldbook_editor(app),
        _ => {}
    }
    None
}

fn create_and_edit_worldbook(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.worldbook.list.iter().cloned().collect();
    let new_name = super::generate_unique_name("worldbook", &existing);
    let wb = libllm_core::worldinfo::WorldBook {
        name: new_name.clone(),
        entries: Vec::new(),
    };
    let slug = libllm_core::character::slugify(&new_name);
    if let Err(e) = app
        .db
        .as_ref()
        .map(|db| db.insert_worldbook(&slug, &wb).map_err(anyhow::Error::from))
        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    {
        app.set_status(
            format!("Failed to create worldbook: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.worldbook.list.push(new_name.clone());
    app.worldbook.list_selected = app.worldbook.list.len() - 1;
    app.worldbook.editor_entries = Vec::new();
    app.worldbook.editor_original_name = new_name.clone();
    app.worldbook.editor_original_entries = Vec::new();
    app.worldbook.editor_name = new_name;
    app.worldbook.editor_selected = 0;
    app.worldbook.editor_name_selected = true;
    app.focus = Focus::WorldbookEditorDialog;
}

fn add_new_entry(app: &mut App) {
    let new_entry = libllm_core::worldinfo::Entry {
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
    app.worldbook.editor_entries.push(new_entry);
    let idx = app.worldbook.editor_entries.len() - 1;
    app.worldbook.editor_selected = idx;
    let entry = &app.worldbook.editor_entries[idx];
    open_entry_editor(app, idx, entry_to_values(entry), entry.selective);
}

fn open_entry_editor(app: &mut App, idx: usize, values: Vec<String>, selective: bool) {
    app.worldbook.entry_editor = Some(if selective {
        super::open_entry_editor(values)
    } else {
        super::open_entry_editor_non_selective(values)
    });
    app.worldbook.entry_editor_index = idx;
    app.focus = Focus::WorldbookEntryEditorDialog;
}

fn entry_to_values(entry: &libllm_core::worldinfo::Entry) -> Vec<String> {
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
    existing: &libllm_core::worldinfo::Entry,
) -> libllm_core::worldinfo::Entry {
    let parse_keys = |s: &str| -> Vec<String> {
        s.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    };
    libllm_core::worldinfo::Entry {
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

pub(crate) fn render_entry_delete_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    super::delete_confirm::render_confirm_dialog(
        f,
        area,
        &format!("Delete {}?", app.delete_confirm.filename),
        app.delete_confirm.selected,
    );
}

pub(crate) fn handle_entry_delete_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match super::delete_confirm::handle_confirm_key(key, &mut app.delete_confirm.selected) {
        super::delete_confirm::ConfirmResult::Confirmed => {
            let idx = app.worldbook.editor_selected;
            app.worldbook.editor_entries.remove(idx);
            if app.worldbook.editor_selected >= app.worldbook.editor_entries.len()
                && app.worldbook.editor_selected > 0
            {
                app.worldbook.editor_selected -= 1;
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

fn worldbook_editor_is_dirty_inner(
    original_name: &str,
    current_name: &str,
    original_entries: &[libllm_core::worldinfo::Entry],
    current_entries: &[libllm_core::worldinfo::Entry],
) -> bool {
    original_name != current_name || original_entries != current_entries
}

fn worldbook_editor_is_dirty(app: &App) -> bool {
    worldbook_editor_is_dirty_inner(
        &app.worldbook.editor_original_name,
        &app.worldbook.editor_name,
        &app.worldbook.editor_original_entries,
        &app.worldbook.editor_entries,
    )
}

fn close_worldbook_editor(app: &mut App) {
    if worldbook_editor_is_dirty(app) {
        app.worldbook.editor_name_editing = false;
        app.dialog_search.deactivate_and_clear();
        app.unsaved_warning = Some(crate::dialogs::unsaved_warning::UnsavedWarningState::new(
            Focus::WorldbookEditorDialog,
        ));
        app.focus = Focus::UnsavedWarningDialog;
    } else {
        app.worldbook.editor_name_editing = false;
        app.dialog_search.deactivate_and_clear();
        app.focus = Focus::WorldbookDialog;
    }
}

pub(crate) fn commit_editor_and_close(app: &mut App) {
    save_worldbook_editor(app);
    app.focus = Focus::WorldbookDialog;
}

fn save_worldbook_editor(app: &mut App) {
    let original = app.worldbook.editor_original_name.clone();
    let new_name = app.worldbook.editor_name.clone();

    if original == new_name && app.worldbook.editor_entries == app.worldbook.editor_original_entries
    {
        app.set_status(
            "No changes found.".to_owned(),
            super::super::StatusLevel::Info,
        );
        return;
    }

    if original != new_name && app.worldbook.list.iter().any(|n| n == &new_name) {
        app.set_status(
            format!("Name '{new_name}' is already in use."),
            super::super::StatusLevel::Error,
        );
        return;
    }

    let wb = libllm_core::worldinfo::WorldBook {
        name: new_name.clone(),
        entries: app.worldbook.editor_entries.clone(),
    };
    let slug = libllm_core::character::slugify(&new_name);
    let old_slug = libllm_core::character::slugify(&original);
    let is_rename = !original.is_empty() && original != new_name;
    let save_result = if is_rename {
        app.db
            .as_ref()
            .map(|db| db.insert_worldbook(&slug, &wb).map_err(anyhow::Error::from))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    } else {
        app.db
            .as_ref()
            .map(|db| {
                if db.load_worldbook(&slug).is_ok() {
                    db.update_worldbook(&slug, &wb)
                } else {
                    db.insert_worldbook(&slug, &wb)
                }
            })
            .map(|r| r.map_err(anyhow::Error::from))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    };
    match save_result {
        Ok(()) => {
            if is_rename {
                if let Some(pos) = app.session.worldbooks.iter().position(|n| n == &original) {
                    app.session.worldbooks[pos] = new_name.clone();
                }
                if let Some(pos) = app.config.worldbooks.iter().position(|n| n == &original) {
                    app.config.worldbooks[pos] = new_name.clone();
                    let _ = libllm_config::save(&app.config);
                }
                if let Some(db) = app.db.as_ref() {
                    let _ = db.delete_worldbook(&old_slug);
                }
            }
            app.invalidate_worldbook_cache();
            let books = app
                .db
                .as_ref()
                .and_then(|db| db.list_worldbooks().ok())
                .unwrap_or_default();
            app.worldbook.list = books.into_iter().map(|(_, n)| n).collect();
            app.worldbook.list_selected = app
                .worldbook
                .list
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

pub(crate) fn handle_worldbook_paste(path: &std::path::Path, ext: &str, app: &mut App) -> bool {
    if ext != "json" {
        app.set_status(
            "Worldbook import supports .json files only.".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return true;
    }

    let fallback_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    match std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!(e))
        .and_then(|s| {
            libllm_core::worldinfo::parse_worldbook_json(&s, &fallback_name)
                .map_err(|e| anyhow::anyhow!(e))
        }) {
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
            let slug = libllm_core::character::slugify(&name);
            match app
                .db
                .as_ref()
                .map(|db| db.insert_worldbook(&slug, &wb).map_err(anyhow::Error::from))
                .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
            {
                Ok(()) => {
                    let books = app
                        .db
                        .as_ref()
                        .and_then(|db| db.list_worldbooks().ok())
                        .unwrap_or_default();
                    app.worldbook.list = books.into_iter().map(|(_, n)| n).collect();
                    app.worldbook.list_selected = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn make_entry(keys: Vec<&str>) -> libllm_core::worldinfo::Entry {
        libllm_core::worldinfo::Entry {
            keys: keys.into_iter().map(String::from).collect(),
            secondary_keys: Vec::new(),
            selective: false,
            content: String::new(),
            constant: false,
            enabled: true,
            order: 10,
            depth: 4,
            case_sensitive: false,
        }
    }

    #[test]
    fn worldbook_editor_dirty_when_name_changes() {
        assert!(worldbook_editor_is_dirty_inner(
            "original",
            "renamed",
            &[],
            &[],
        ));
    }

    #[test]
    fn worldbook_editor_clean_when_name_and_entries_match() {
        let entry = make_entry(vec!["key"]);
        assert!(!worldbook_editor_is_dirty_inner(
            "same",
            "same",
            std::slice::from_ref(&entry),
            std::slice::from_ref(&entry),
        ));
    }

    #[test]
    fn worldbook_editor_dirty_when_entries_differ() {
        let entry_a = make_entry(vec!["key-a"]);
        let entry_b = make_entry(vec!["key-b"]);
        assert!(worldbook_editor_is_dirty_inner(
            "same",
            "same",
            &[entry_a],
            &[entry_b],
        ));
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_s_is_save_shortcut() {
        assert!(is_save_shortcut(&key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn ctrl_shift_s_is_save_shortcut() {
        assert!(is_save_shortcut(&key(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn plain_s_is_not_save_shortcut() {
        assert!(!is_save_shortcut(&key(
            KeyCode::Char('s'),
            KeyModifiers::NONE
        )));
    }
}
