//! Persona picker and editor dialog for managing user profiles.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::dialog_handler::return_to_input;
use crate::{Action, App, DeleteContext, Focus};

pub(crate) fn render_persona_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let visible_indices = super::filter_indices(&app.persona.names, &app.dialog_search);
    let unfiltered_total = app.persona.names.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let filtered_selected =
        super::filtered_selection_position(&visible_indices, app.persona.selected).unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| {
            let name = &app.persona.names[i];
            let slug = app.persona.slugs.get(i).map(String::as_str).unwrap_or("");
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
            Line::from(
                "Enter: select  Right: edit  a: add  Del: delete  Ctrl+F: search  Esc: cancel",
            ),
            Line::from("Drop .txt to import"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(crate) fn handle_persona_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.persona.slugs.is_empty() && !app.dialog_search.active {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_persona(app);
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
        &mut app.persona.selected,
        &app.persona.names,
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

    let visible_indices = super::filter_indices(&app.persona.names, &app.dialog_search);
    let Some(selected) = super::visible_selection(&visible_indices, app.persona.selected) else {
        if key.code == KeyCode::Char('a') {
            create_and_edit_persona(app);
        } else if key.code == KeyCode::Esc {
            return_to_input(app);
        }
        return None;
    };

    match key.code {
        KeyCode::Enter => {
            let slug = app.persona.slugs[selected].clone();
            match app.db.as_ref().and_then(|db| db.load_persona(&slug).ok()) {
                Some(pf) => {
                    let display_name = pf.name.clone();
                    app.persona.active_name = Some(pf.name);
                    app.persona.active_desc = Some(pf.persona);
                    app.session.persona = Some(slug.clone());
                    app.invalidate_chat_caches();
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);

                    app.config.default_persona = Some(slug.clone());
                    let mut cfg = libllm_config::load();
                    cfg.default_persona = Some(slug.clone());
                    if let Err(e) = libllm_config::save(&cfg) {
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
            return_to_input(app);
        }
        KeyCode::Right => {
            let slug = app.persona.slugs[selected].clone();
            open_persona_editor(app, &slug);
        }
        KeyCode::Char('a') => {
            create_and_edit_persona(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.persona.names[selected].clone();
            let slug = app.persona.slugs[selected].clone();
            app.delete_confirm.filename = name;
            app.delete_confirm.selected = 0;
            app.delete_confirm.context = DeleteContext::Persona { slug };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Esc => {
            return_to_input(app);
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

    app.persona.editor_slug = slug.to_owned();
    app.persona.editor = Some(super::open_persona_editor(values));
    app.focus = Focus::PersonaEditorDialog;
}

fn create_and_edit_persona(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.persona.names.iter().cloned().collect();
    let new_name = super::generate_unique_name("persona", &existing);
    let persona = libllm_core::persona::PersonaFile {
        name: new_name.clone(),
        persona: String::new(),
    };
    let slug = libllm_core::character::slugify(&new_name);
    if let Err(e) = app
        .db
        .as_ref()
        .map(|db| {
            db.insert_persona(&slug, &persona)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    {
        app.set_status(
            format!("Failed to create persona: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.persona.names.push(new_name);
    app.persona.slugs.push(slug.clone());
    app.persona.selected = app.persona.slugs.len() - 1;
    open_persona_editor(app, &slug);
}

pub(crate) fn handle_persona_paste(path: &std::path::Path, ext: &str, app: &mut App) -> bool {
    let Some((name, content)) = super::import_txt_file(path, ext, "Persona", app) else {
        return true;
    };

    let persona = libllm_core::persona::PersonaFile {
        name: name.clone(),
        persona: content,
    };
    let slug = libllm_core::character::slugify(&name);
    match app
        .db
        .as_ref()
        .map(|db| {
            db.insert_persona(&slug, &persona)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    {
        Ok(()) => {
            let personas = app
                .db
                .as_ref()
                .and_then(|db| db.list_personas().ok())
                .unwrap_or_default();
            app.persona.names = personas.iter().map(|(_, n)| n.clone()).collect();
            app.persona.slugs = personas.into_iter().map(|(s, _)| s).collect();
            app.persona.selected = 0;
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
