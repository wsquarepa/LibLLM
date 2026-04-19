//! Persona picker and editor dialog for managing user profiles.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::tui::{Action, App, DeleteContext, Focus};

pub(in crate::tui) fn render_persona_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let visible_indices = super::filter_indices(&app.persona_names, &app.dialog_search);
    let unfiltered_total = app.persona_names.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let filtered_selected = visible_indices
        .iter()
        .position(|&i| i == app.persona_selected)
        .unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| {
            let name = &app.persona_names[i];
            let slug = app.persona_slugs.get(i).map(String::as_str).unwrap_or("");
            let active_marker = if app.session.persona.as_deref() == Some(slug) {
                " *"
            } else {
                ""
            };
            ListItem::new(format!("{name}{active_marker}"))
        })
        .collect();

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: filtered_selected,
            items,
            title_base: " Personas ",
            search: Some(&app.dialog_search),
            unfiltered_total: Some(unfiltered_total),
        },
    );

    let hints = if app.dialog_search.active {
        vec![Line::from("Enter: apply  Esc: cancel  type to filter")]
    } else {
        vec![
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: select  Right: edit  a: add  Del: delete  Ctrl+F: search  Esc: cancel"),
            Line::from("Drop .txt to import"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(in crate::tui) fn handle_persona_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.persona_slugs.is_empty() && !app.dialog_search.active {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_persona(app);
            }
            KeyCode::Esc => {
                app.focus = Focus::Input;
            }
            _ => {}
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::LIST_DIALOG_TALL_PADDING);
    let action = super::handle_paged_list_key(
        &mut app.persona_selected,
        &app.persona_names,
        visible,
        key,
        Some(&mut app.dialog_search),
    );
    if matches!(action, super::PagedListAction::Consumed | super::PagedListAction::EnteredSearch | super::PagedListAction::ExitedSearch) {
        return None;
    }

    match key.code {
        KeyCode::Enter => {
            let slug = app.persona_slugs[app.persona_selected].clone();
            match app.db.as_ref().and_then(|db| db.load_persona(&slug).ok()) {
                Some(pf) => {
                    let display_name = pf.name.clone();
                    app.active_persona_name = Some(pf.name);
                    app.active_persona_desc = Some(pf.persona);
                    app.session.persona = Some(slug.clone());
                    app.invalidate_chat_cache();
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);

                    app.config.default_persona = Some(slug.clone());
                    let mut cfg = libllm::config::load();
                    cfg.default_persona = Some(slug.clone());
                    if let Err(e) = libllm::config::save(&cfg) {
                        tracing::warn!(result = "error", error = %e, "config.default_persona");
                    }

                    app.set_status(
                        format!("Persona set to '{display_name}'."),
                        super::super::StatusLevel::Info,
                    );
                }
                None => {
                    app.set_status(
                        format!("Failed to load persona '{slug}'."),
                        super::super::StatusLevel::Error,
                    );
                }
            }
            app.focus = Focus::Input;
        }
        KeyCode::Right => {
            let slug = app.persona_slugs[app.persona_selected].clone();
            open_persona_editor(app, &slug);
        }
        KeyCode::Char('a') => {
            create_and_edit_persona(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.persona_names[app.persona_selected].clone();
            let slug = app.persona_slugs[app.persona_selected].clone();
            app.delete_confirm_filename = name;
            app.delete_confirm_selected = 0;
            app.delete_context = DeleteContext::Persona { slug };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn open_persona_editor(app: &mut App, slug: &str) {
    let pf = app.db.as_ref().and_then(|db| db.load_persona(slug).ok());
    let values = match pf {
        Some(pf) => vec![pf.name, pf.persona],
        None => vec![slug.to_owned(), String::new()],
    };

    app.persona_editor_slug = slug.to_owned();
    app.persona_editor = Some(super::open_persona_editor(values));
    app.focus = Focus::PersonaEditorDialog;
}

fn create_and_edit_persona(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.persona_names.iter().cloned().collect();
    let new_name = super::generate_unique_name("persona", &existing);
    let persona = libllm::persona::PersonaFile {
        name: new_name.clone(),
        persona: String::new(),
    };
    let slug = libllm::character::slugify(&new_name);
    if let Err(e) = app.db.as_ref().map(|db| db.insert_persona(&slug, &persona)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
        app.set_status(
            format!("Failed to create persona: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.persona_names.push(new_name);
    app.persona_slugs.push(slug.clone());
    app.persona_selected = app.persona_slugs.len() - 1;
    open_persona_editor(app, &slug);
}

pub(in crate::tui) fn handle_persona_paste(
    path: &std::path::Path,
    ext: &str,
    app: &mut App,
) -> bool {
    if ext != "txt" {
        app.set_status(
            "Persona import supports .txt files only.".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return true;
    }

    match path.metadata() {
        Ok(meta) if meta.len() > super::MAX_TXT_IMPORT_BYTES => {
            app.set_status(
                "File too large (max 1 MB).".to_owned(),
                super::super::StatusLevel::Error,
            );
            return true;
        }
        Err(e) => {
            app.set_status(
                format!("Cannot read file: {e}"),
                super::super::StatusLevel::Error,
            );
            return true;
        }
        _ => {}
    }

    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => {
            app.set_status(
                "Invalid filename.".to_owned(),
                super::super::StatusLevel::Error,
            );
            return true;
        }
    };

    let name = match super::sanitize_import_name(stem) {
        Some(n) => n,
        None => {
            app.set_status(
                "Filename produces an empty name after sanitization.".to_owned(),
                super::super::StatusLevel::Error,
            );
            return true;
        }
    };

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            app.set_status(format!("Read error: {e}"), super::super::StatusLevel::Error);
            return true;
        }
    };

    let persona = libllm::persona::PersonaFile {
        name: name.clone(),
        persona: content,
    };
    let slug = libllm::character::slugify(&name);
    match app.db.as_ref().map(|db| db.insert_persona(&slug, &persona)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
        Ok(()) => {
            let personas = app.db.as_ref().and_then(|db| db.list_personas().ok()).unwrap_or_default();
            app.persona_names = personas.iter().map(|(_, n)| n.clone()).collect();
            app.persona_slugs = personas.into_iter().map(|(s, _)| s).collect();
            app.persona_selected = 0;
            app.set_status(
                format!("Imported persona: {name}"),
                super::super::StatusLevel::Info,
            );
        }
        Err(e) => {
            app.set_status(format!("Save error: {e}"), super::super::StatusLevel::Error);
        }
    }
    true
}
