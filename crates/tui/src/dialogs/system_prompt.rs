//! System prompt picker and editor dialog for selecting and editing prompts.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::dialog_handler::return_to_input;
use crate::{Action, App, DeleteContext, Focus};

pub(crate) fn render_system_prompt_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let visible_indices = super::filter_indices(&app.system_prompt.list, &app.dialog_search);
    let unfiltered_total = app.system_prompt.list.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let filtered_selected =
        super::filtered_selection_position(&visible_indices, app.system_prompt.selected)
            .unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| ListItem::new(app.system_prompt.list[i].clone()))
        .collect();

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: filtered_selected,
            items,
            title_base: " System Prompts ",
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

pub(crate) fn handle_system_prompt_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.system_prompt.list.is_empty() && !app.dialog_search.active {
        if key.code == KeyCode::Esc {
            return_to_input(app);
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::LIST_DIALOG_TALL_PADDING);
    let action = super::handle_paged_list_key(
        &mut app.system_prompt.selected,
        &app.system_prompt.list,
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

    let visible_indices = super::filter_indices(&app.system_prompt.list, &app.dialog_search);
    let Some(selected) = super::visible_selection(&visible_indices, app.system_prompt.selected)
    else {
        if key.code == KeyCode::Esc {
            return_to_input(app);
        }
        return None;
    };

    match key.code {
        KeyCode::Enter => {
            let name = app.system_prompt.list[selected].clone();
            let content = app
                .db
                .as_ref()
                .and_then(|db| db.load_prompt(&name).ok())
                .map(|p| p.content);

            app.session.system_prompt = content;
            app.invalidate_prompt_cache();
            app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
            app.set_status(
                format!("System prompt set to '{name}'."),
                super::super::StatusLevel::Info,
            );
            return_to_input(app);
        }
        KeyCode::Right => {
            let name = app.system_prompt.list[selected].clone();
            open_prompt_editor(app, &name);
        }
        KeyCode::Char('a') => {
            let existing: std::collections::HashSet<String> =
                app.system_prompt.list.iter().cloned().collect();
            let new_name = super::generate_unique_name("custom", &existing);
            let prompt = libllm_core::system_prompt::SystemPromptFile {
                name: new_name.clone(),
                content: String::new(),
            };
            let slug = libllm_core::character::slugify(&new_name);
            if let Err(e) = app
                .db
                .as_ref()
                .map(|db| {
                    db.insert_prompt(&slug, &prompt, false)
                        .map_err(anyhow::Error::from)
                })
                .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
            {
                app.set_status(
                    format!("Failed to create prompt: {e}"),
                    super::super::StatusLevel::Error,
                );
                return None;
            }
            app.system_prompt.list.push(new_name.clone());
            app.system_prompt.selected = app.system_prompt.list.len() - 1;
            open_prompt_editor(app, &new_name);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.system_prompt.list[selected].clone();
            if name == libllm_core::system_prompt::BUILTIN_ASSISTANT
                || name == libllm_core::system_prompt::BUILTIN_ROLEPLAY
            {
                app.set_status(
                    "Cannot delete built-in prompts.".to_owned(),
                    super::super::StatusLevel::Warning,
                );
            } else {
                app.delete_confirm.filename = name.clone();
                app.delete_confirm.selected = 0;
                app.delete_confirm.context = DeleteContext::SystemPrompt { name };
                app.focus = Focus::DeleteConfirmDialog;
            }
        }
        KeyCode::Esc => {
            return_to_input(app);
        }
        _ => {}
    }
    None
}

fn open_prompt_editor(app: &mut App, name: &str) {
    let content = app
        .db
        .as_ref()
        .and_then(|db| db.load_prompt(name).ok())
        .map(|p| p.content)
        .unwrap_or_default();

    let values = vec![name.to_owned(), content];
    let is_builtin = name == libllm_core::system_prompt::BUILTIN_ASSISTANT
        || name == libllm_core::system_prompt::BUILTIN_ROLEPLAY;

    let mut dialog = super::open_system_prompt_editor(values);
    if is_builtin {
        dialog = dialog.with_locked_fields(vec![0]);
    }

    app.system_prompt.editor = Some(dialog);
    app.system_prompt.editor_prompt_name = name.to_owned();
    app.system_prompt.editor_read_only = false;
    app.system_prompt.editor_return_focus = Focus::SystemPromptDialog;
    app.focus = Focus::SystemPromptEditorDialog;
}

pub(crate) fn handle_system_prompt_paste(path: &std::path::Path, ext: &str, app: &mut App) -> bool {
    let Some((name, content)) = super::import_txt_file(path, ext, "System prompt", app) else {
        return true;
    };

    let prompt = libllm_core::system_prompt::SystemPromptFile {
        name: name.clone(),
        content,
    };
    let slug = libllm_core::character::slugify(&name);
    match app
        .db
        .as_ref()
        .map(|db| {
            db.insert_prompt(&slug, &prompt, false)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    {
        Ok(()) => {
            let prompts = app
                .db
                .as_ref()
                .and_then(|db| db.list_prompts().ok())
                .unwrap_or_default();
            app.system_prompt.list = prompts.into_iter().map(|e| e.name).collect();
            app.system_prompt.selected = 0;
            app.set_status(
                format!("Imported system prompt: {name}"),
                super::super::StatusLevel::Info,
            );
        }
        Err(e) => {
            app.set_status(format!("Save error: {e}"), super::super::StatusLevel::Error);
        }
    }
    true
}
