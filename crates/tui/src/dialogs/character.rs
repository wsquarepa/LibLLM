//! Character card picker and editor dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::business::refresh_sidebar;
use crate::dialog_handler::return_to_input;
use crate::{Action, App, DeleteContext, Focus};
use libllm_core::session::{self, Message, Role};

pub(crate) fn render_character_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let visible_indices = super::filter_indices(&app.character.names, &app.dialog_search);
    let unfiltered_total = app.character.names.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let filtered_selected =
        super::filtered_selection_position(&visible_indices, app.character.selected).unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| {
            let mark = if app.character.picks.get(i).copied().unwrap_or(false) {
                "[x] "
            } else {
                "[ ] "
            };
            ListItem::new(format!("{mark}{}", app.character.names[i]))
        })
        .collect();

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: filtered_selected,
            items,
            title_base: " Select Character ",
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
                "Space: toggle  Enter: confirm  Right: edit  a: add  Del: delete  Ctrl+F: search  Esc: cancel",
            ),
            Line::from("Drop .png/.json to import"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(crate) fn handle_character_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.character.names.is_empty() && !app.dialog_search.active {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_character(app);
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
        &mut app.character.selected,
        &app.character.names,
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

    let visible_indices = super::filter_indices(&app.character.names, &app.dialog_search);
    let Some(selected) = super::visible_selection(&visible_indices, app.character.selected) else {
        if key.code == KeyCode::Esc {
            return_to_input(app);
        }
        return None;
    };

    match key.code {
        KeyCode::Char(' ') => {
            if let Some(pick) = app.character.picks.get_mut(selected) {
                *pick = !*pick;
            }
            return None;
        }
        KeyCode::Enter => {
            let picked: Vec<usize> = visible_indices
                .iter()
                .copied()
                .filter(|&i| app.character.picks.get(i).copied().unwrap_or(false))
                .collect();

            if picked.len() >= 2 {
                let slugs: Vec<String> = picked
                    .iter()
                    .map(|&i| app.character.slugs[i].clone())
                    .collect();
                let names_by_slug: std::collections::HashMap<String, String> = picked
                    .iter()
                    .map(|&i| {
                        (
                            app.character.slugs[i].clone(),
                            app.character.names[i].clone(),
                        )
                    })
                    .collect();
                if let Err(e) = libllm_core::group_chat::validate_group_chat_args(
                    &slugs,
                    &Default::default(),
                    &names_by_slug,
                ) {
                    app.set_status(e.to_string(), super::super::StatusLevel::Error);
                    return None;
                }

                if !app.flush_session_before_transition() {
                    return None;
                }

                let cards: Vec<libllm_core::character::CharacterCard> = slugs
                    .iter()
                    .filter_map(|s| app.db.as_ref().and_then(|db| db.load_character(s).ok()))
                    .collect();
                if cards.len() != slugs.len() {
                    app.set_status(
                        "One or more selected characters could not be loaded.".to_owned(),
                        super::super::StatusLevel::Error,
                    );
                    return_to_input(app);
                    return None;
                }

                app.discard_pending_session_save();
                app.session.tree.clear();
                app.session.worldbooks.clear();
                app.session.system_prompt = None;
                app.session.author_note = None;
                app.active_card_author_note = None;
                app.session.character = None;
                app.session.characters = slugs
                    .iter()
                    .map(|s| libllm_core::group_chat::CharacterAttachment::new(s.clone()))
                    .collect();
                app.session.chat_mode = libllm_core::group_chat::ChatMode::default();
                app.session.scenario = None;

                crate::business::rebuild_character_cards_cache(app);
                app.invalidate_chat_caches();
                app.invalidate_worldbook_cache();
                app.chat_scroll = 0;
                app.auto_scroll = true;
                let new_id = session::generate_session_id();
                app.save_mode.set_id(new_id);
                app.group_chat.creation_pending = true;
                refresh_sidebar(app);
                return_to_input(app);
                return Some(Action::OpenChatSettings);
            }

            let single_index = if picked.len() == 1 {
                picked[0]
            } else {
                selected
            };

            if !app.flush_session_before_transition() {
                return None;
            }
            let slug = app.character.slugs[single_index].clone();
            let load_result = app.db.as_ref().and_then(|db| db.load_character(&slug).ok());
            match load_result {
                Some(card) => {
                    app.discard_pending_session_save();
                    app.session.tree.clear();
                    app.session.worldbooks.clear();
                    let cfg = libllm_config::load();
                    let tpl_name = cfg.template_preset.as_deref().unwrap_or("Default");
                    let tpl = libllm_core::preset::resolve_template_preset(
                        tpl_name,
                        &libllm_config::template_presets_dir(),
                    );
                    app.session.system_prompt = Some(libllm_core::character::build_system_prompt(
                        &card,
                        Some(&tpl),
                    ));
                    app.session.character = Some(card.name.clone());
                    app.session.characters =
                        vec![libllm_core::group_chat::CharacterAttachment::new(slug)];
                    app.session.scenario =
                        libllm_core::group_chat::inherit_card_scenario(&card.scenario);
                    app.active_card_author_note = card.author_note.clone();
                    app.invalidate_chat_caches();
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
                    refresh_sidebar(app);
                    return_to_input(app);
                }
                None => {
                    app.set_status(
                        "Character not found.".to_owned(),
                        super::super::StatusLevel::Error,
                    );
                    return_to_input(app);
                }
            }
        }
        KeyCode::Right => {
            let slug = app.character.slugs[selected].clone();
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
                        card.author_note
                            .as_ref()
                            .map(|n| n.text.clone())
                            .unwrap_or_default(),
                        card.author_note
                            .as_ref()
                            .map(|n| n.depth.to_string())
                            .unwrap_or_else(|| libllm_core::author_note::DEFAULT_DEPTH.to_string()),
                        if card.author_note.as_ref().is_some_and(|n| n.at_top) {
                            "true".to_owned()
                        } else {
                            "false".to_owned()
                        },
                    ];
                    app.character.editor = Some(super::open_character_editor(values));
                    app.character.editor_slug = slug;
                    app.focus = Focus::CharacterEditorDialog;
                }
                None => {
                    app.set_status(
                        "Character not found.".to_owned(),
                        super::super::StatusLevel::Error,
                    );
                }
            }
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.character.names[selected].clone();
            let slug = app.character.slugs[selected].clone();
            app.delete_confirm.filename = name;
            app.delete_confirm.selected = 0;
            app.delete_confirm.context = DeleteContext::Character { slug };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Char('a') => {
            create_and_edit_character(app);
        }
        KeyCode::Esc => {
            return_to_input(app);
        }
        _ => {}
    }
    None
}

fn create_and_edit_character(app: &mut App) {
    let existing: std::collections::HashSet<String> = app.character.names.iter().cloned().collect();
    let new_name = super::generate_unique_name("character", &existing);
    let card = libllm_core::character::CharacterCard {
        name: new_name.clone(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: Vec::new(),
        author_note: None,
    };
    let slug = libllm_core::character::slugify(&new_name);
    if let Err(e) = app
        .db
        .as_ref()
        .map(|db| {
            db.insert_character(&slug, &card)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
    {
        app.set_status(
            format!("Failed to create character: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    app.character.names.push(new_name);
    app.character.slugs.push(slug.clone());
    app.character.selected = app.character.names.len() - 1;

    let values = vec![
        card.name,
        card.description,
        card.personality,
        card.scenario,
        card.first_mes,
        card.mes_example,
        card.system_prompt,
        card.post_history_instructions,
        String::new(),
        libllm_core::author_note::DEFAULT_DEPTH.to_string(),
        "false".to_owned(),
    ];
    app.character.editor = Some(super::open_character_editor(values));
    app.character.editor_slug = slug;
    app.focus = Focus::CharacterEditorDialog;
}

pub(crate) fn handle_character_paste(path: &std::path::Path, ext: &str, app: &mut App) -> bool {
    if ext != "png" && ext != "json" {
        app.set_status(
            "Character import supports .png and .json files only.".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return true;
    }

    match libllm_core::character::import_card(path) {
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
            let slug = libllm_core::character::slugify(&name);
            match app
                .db
                .as_ref()
                .map(|db| {
                    db.insert_character(&slug, &card)
                        .map_err(anyhow::Error::from)
                })
                .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
            {
                Ok(()) => {
                    let chars = app
                        .db
                        .as_ref()
                        .and_then(|db| db.list_characters().ok())
                        .unwrap_or_default();
                    app.character.names = chars.iter().map(|(_, n)| n.clone()).collect();
                    app.character.slugs = chars.into_iter().map(|(s, _)| s).collect();
                    app.character.selected = 0;
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
