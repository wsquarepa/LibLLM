//! Character card picker and editor dialog with session greeting injection.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use libllm::session::{self, Message, Role};
use crate::tui::business::refresh_sidebar;
use crate::tui::{Action, App, DeleteContext, Focus};

pub(in crate::tui) fn render_character_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.character_names.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let items: Vec<ListItem<'_>> = app
        .character_names
        .iter()
        .map(|name| ListItem::new(name.clone()))
        .collect();

    super::render_paged_list(
        f,
        dialog,
        app.character_selected,
        items,
        " Select Character ",
        &app.theme,
    );

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: select  Right: edit  a: add  Del: delete  Esc: cancel"),
            Line::from("Drop .png/.json to import"),
        ],
    );
}

pub(in crate::tui) fn handle_character_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.character_names.is_empty() {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_character(app);
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
        &mut app.character_selected,
        app.character_names.len(),
        visible,
        key,
    ) == super::PagedListAction::Consumed
    {
        return None;
    }

    match key.code {
        KeyCode::Enter => {
            if !app.flush_session_before_transition() {
                return None;
            }
            let slug = app.character_slugs[app.character_selected].clone();
            let load_result = app.db.as_ref().and_then(|db| db.load_character(&slug).ok());
            match load_result {
                Some(card) => {
                    app.discard_pending_session_save();
                    app.session.tree.clear();
                    app.session.worldbooks.clear();
                    let cfg = libllm::config::load();
                    let tpl_name = cfg.template_preset.as_deref().unwrap_or("Default");
                    let tpl = libllm::preset::resolve_template_preset(tpl_name);
                    app.session.system_prompt =
                        Some(libllm::character::build_system_prompt(&card, Some(&tpl)));
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
                    let new_id = session::generate_session_id();
                    app.save_mode.set_id(new_id);
                    app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
                    app.set_status(
                        format!("Loaded character: {}", card.name),
                        super::super::StatusLevel::Info,
                    );
                    app.focus = Focus::Input;
                    refresh_sidebar(app);
                }
                None => {
                    app.set_status("Character not found.".to_owned(), super::super::StatusLevel::Error);
                    app.focus = Focus::Input;
                }
            }
        }
        KeyCode::Right => {
            let slug = app.character_slugs[app.character_selected].clone();
            match app.db.as_ref().and_then(|db| db.load_character(&slug).ok()) {
                Some(card) => {
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
                    app.character_editor = Some(super::open_character_editor(values));
                    app.character_editor_slug = slug;
                    app.focus = Focus::CharacterEditorDialog;
                }
                None => {
                    app.set_status("Character not found.".to_owned(), super::super::StatusLevel::Error);
                }
            }
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.character_names[app.character_selected].clone();
            let slug = app.character_slugs[app.character_selected].clone();
            app.delete_confirm_filename = name;
            app.delete_confirm_selected = 0;
            app.delete_context = DeleteContext::Character { slug };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Char('a') => {
            create_and_edit_character(app);
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn create_and_edit_character(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.character_names.iter().cloned().collect();
    let new_name = super::generate_unique_name("character", &existing);
    let card = libllm::character::CharacterCard {
        name: new_name.clone(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: Vec::new(),
    };
    let slug = libllm::character::slugify(&new_name);
    if let Err(e) = app.db.as_ref().map(|db| db.insert_character(&slug, &card)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
        app.set_status(
            format!("Failed to create character: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.character_names.push(new_name);
    app.character_slugs.push(slug.clone());
    app.character_selected = app.character_names.len() - 1;

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
    app.character_editor = Some(super::open_character_editor(values));
    app.character_editor_slug = slug;
    app.focus = Focus::CharacterEditorDialog;
}

pub(in crate::tui) fn handle_character_paste(
    path: &std::path::Path,
    ext: &str,
    app: &mut App,
) -> bool {
    if ext != "png" && ext != "json" {
        app.set_status(
            "Character import supports .png and .json files only.".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return true;
    }

    match libllm::character::import_card(path) {
        Ok(card) => {
            if card.name.chars().count() > super::MAX_NAME_LENGTH {
                app.set_status(
                    format!(
                        "Character name exceeds {} characters: \"{}\"",
                        super::MAX_NAME_LENGTH,
                        card.name,
                    ),
                    super::super::StatusLevel::Error,
                );
                return true;
            }
            let name = card.name.clone();
            let slug = libllm::character::slugify(&name);
            match app.db.as_ref().map(|db| db.insert_character(&slug, &card)).unwrap_or_else(|| Err(anyhow::anyhow!("no database"))) {
                Ok(()) => {
                    let chars = app.db.as_ref().and_then(|db| db.list_characters().ok()).unwrap_or_default();
                    app.character_names = chars.iter().map(|(_, n)| n.clone()).collect();
                    app.character_slugs = chars.into_iter().map(|(s, _)| s).collect();
                    app.character_selected = 0;
                    app.set_status(
                        format!("Imported character: {name}"),
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
