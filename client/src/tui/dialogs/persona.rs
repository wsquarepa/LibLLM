use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, DeleteContext, Focus};

pub(in crate::tui) fn render_persona_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.persona_list.len();
    let dialog = clear_centered(
        f,
        super::LIST_DIALOG_WIDTH,
        count as u16 + super::LIST_DIALOG_TALL_PADDING,
        area,
    );

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.persona_list.iter().enumerate() {
        let is_selected = i == app.persona_selected;
        let marker = if is_selected { "> " } else { "  " };
        let active_marker = if app.session.persona.as_deref() == Some(name.as_str()) {
            " *"
        } else {
            ""
        };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{name}{active_marker}"),
            style,
        )));
    }

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Personas ", Color::Yellow));

    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("Up/Down: navigate  Enter: select  Right: edit"),
            Line::from("a: add new  Del: delete  Esc: cancel"),
            Line::from("Drop .txt to import"),
        ],
    );
}

pub(in crate::tui) fn handle_persona_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.persona_list.is_empty() {
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

    match key.code {
        KeyCode::Up => {
            super::move_selection_up(&mut app.persona_selected);
        }
        KeyCode::Down => {
            super::move_selection_down(&mut app.persona_selected, app.persona_list.len());
        }
        KeyCode::Enter => {
            let file_name = app.persona_list[app.persona_selected].clone();
            match app.db.as_ref().and_then(|db| db.load_persona(&file_name).ok()) {
                Some(pf) => {
                    app.active_persona_name = Some(pf.name);
                    app.active_persona_desc = Some(pf.persona);
                    app.session.persona = Some(file_name.clone());
                    app.invalidate_chat_cache();
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);

                    app.config.default_persona = Some(file_name.clone());
                    let mut cfg = libllm::config::load();
                    cfg.default_persona = Some(file_name.clone());
                    if let Err(e) = libllm::config::save(&cfg) {
                        libllm::debug_log::log_kv(
                            "config.default_persona",
                            &[
                                libllm::debug_log::field("result", "error"),
                                libllm::debug_log::field("error", &e),
                            ],
                        );
                    }

                    app.set_status(
                        format!("Persona set to '{file_name}'."),
                        super::super::StatusLevel::Info,
                    );
                }
                None => {
                    app.set_status(
                        format!("Failed to load persona '{file_name}'."),
                        super::super::StatusLevel::Error,
                    );
                }
            }
            app.focus = Focus::Input;
        }
        KeyCode::Right => {
            let file_name = app.persona_list[app.persona_selected].clone();
            open_persona_editor(app, &file_name);
        }
        KeyCode::Char('a') => {
            create_and_edit_persona(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.persona_list[app.persona_selected].clone();
            app.delete_confirm_filename = name.clone();
            app.delete_confirm_selected = 0;
            app.delete_context = DeleteContext::Persona { name };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn open_persona_editor(app: &mut App, file_name: &str) {
    let pf = app.db.as_ref().and_then(|db| db.load_persona(file_name).ok());
    let values = match pf {
        Some(pf) => vec![pf.name, pf.persona],
        None => vec![file_name.to_owned(), String::new()],
    };

    app.persona_editor_file_name = file_name.to_owned();
    app.persona_editor = Some(super::open_persona_editor(values));
    app.focus = Focus::PersonaEditorDialog;
}

fn create_and_edit_persona(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.persona_list.iter().cloned().collect();
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
    app.persona_list.push(new_name.clone());
    app.persona_selected = app.persona_list.len() - 1;
    open_persona_editor(app, &new_name);
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
            app.persona_list = personas.into_iter().map(|(_, n)| n).collect();
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
