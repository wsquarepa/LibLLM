//! Dialog-level key event routing and generation cancellation logic.

use crossterm::event::KeyEvent;
use ratatui::style::Style;
use tui_textarea::TextArea;

use libllm::session::{Message, Role};

use super::types::*;
use super::{business, dialogs, maintenance};

pub(super) fn cancel_generation(app: &mut App) {
    if let Some(handle) = app.streaming_task.take() {
        handle.abort();
    }

    if app.is_continuation {
        if !app.streaming_buffer.is_empty() {
            let head = app.session.tree.head().unwrap();
            let existing = app.session.tree.node(head).unwrap().message.content.clone();
            let combined = format!("{}{}", existing, app.streaming_buffer);
            app.session.tree.set_message_content(head, combined);
        }
        app.is_continuation = false;
    } else if !app.streaming_buffer.is_empty() {
        let content = std::mem::take(&mut app.streaming_buffer);
        let head = app.session.tree.head().unwrap();
        app.session
            .tree
            .push(Some(head), Message::new(Role::Assistant, content));
    }

    app.streaming_buffer.clear();
    app.is_streaming = false;
    app.mark_session_dirty(SaveTrigger::StreamDone, true);
    app.invalidate_chat_cache();
    app.auto_scroll = true;
}

pub(super) fn open_edit_dialog_with(app: &mut App, content: &str) {
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let mut editor = TextArea::from(if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    });
    configure_textarea_at_end(&mut editor);
    app.edit_editor = Some(editor);
    app.edit_original_content = content.lines().collect::<Vec<_>>().join("\n");
    app.focus = Focus::EditDialog;
}

pub(super) fn configure_textarea(ta: &mut TextArea<'_>) {
    ta.set_cursor_line_style(Style::default());
    ta.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);
}

pub(super) fn configure_textarea_at_end(ta: &mut TextArea<'_>) {
    configure_textarea(ta);
    ta.move_cursor(tui_textarea::CursorMove::Bottom);
    ta.move_cursor(tui_textarea::CursorMove::End);
}

pub(super) enum DialogKind {
    Config,
    Theme,
    PresetEditor,
    PersonaEditor,
    CharacterEditor,
    SystemPromptEditor,
    WorldbookEntryEditor,
}

pub(super) fn handle_field_dialog_key(
    key: KeyEvent,
    app: &mut App,
    kind: DialogKind,
) -> Option<Action> {
    if matches!(kind, DialogKind::Config) {
        let Some(dialog) = app.config_dialog.as_mut() else {
            return None;
        };
        let action = dialog.handle_key(key);
        if let Some(msg) = dialog.clipboard_warning.take() {
            app.set_status(msg, StatusLevel::Warning);
        }
        match action {
            dialogs::TabbedFieldAction::Continue => {}
            dialogs::TabbedFieldAction::Close => {
                let (has_changes, sections) = {
                    let dialog = app.config_dialog.as_ref().unwrap();
                    let has_changes = dialog.has_changes();
                    let sections: Vec<Vec<String>> = dialog
                        .sections()
                        .iter()
                        .map(|s| s.values.clone())
                        .collect();
                    (has_changes, sections)
                };
                if !has_changes {
                    app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                    app.config_dialog = None;
                } else {
                    let existing = libllm::config::load();
                    if let Err(e) = business::apply_tabbed_config_fields(
                        &sections,
                        existing,
                        &app.cli_overrides,
                    ) {
                        app.set_status(
                            format!("Failed to save config: {e}"),
                            StatusLevel::Error,
                        );
                    } else {
                        business::apply_config(app);
                        app.set_status("Config saved.".to_owned(), StatusLevel::Info);
                    }
                    app.config_dialog = None;
                }
            }
            dialogs::TabbedFieldAction::OpenSelector { section: 0, field: 1 } => {
                crate::tui::dialogs::preset::open_preset_picker(
                    app,
                    crate::tui::dialogs::preset::PresetKind::Template,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector { section: 0, field: 2 } => {
                crate::tui::dialogs::preset::open_preset_picker(
                    app,
                    crate::tui::dialogs::preset::PresetKind::Instruct,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector { section: 0, field: 3 } => {
                crate::tui::dialogs::preset::open_preset_picker(
                    app,
                    crate::tui::dialogs::preset::PresetKind::Reasoning,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector { .. } => {}
            dialogs::TabbedFieldAction::InvokeAction { .. } => {}
        }
        return None;
    }

    if matches!(kind, DialogKind::Theme) {
        let Some(dialog) = app.theme_dialog.as_mut() else {
            return None;
        };
        let action = dialog.handle_key(key);
        live_apply_theme_dialog(app);
        match action {
            dialogs::TabbedFieldAction::Continue => {}
            dialogs::TabbedFieldAction::Close => {
                let sections: Vec<Vec<String>> = app
                    .theme_dialog
                    .as_ref()
                    .unwrap()
                    .sections()
                    .iter()
                    .map(|s| s.values.clone())
                    .collect();
                let existing = libllm::config::load();
                if let Err(e) = business::apply_theme_color_sections(&sections, existing) {
                    app.set_status(
                        format!("Failed to save theme: {e}"),
                        StatusLevel::Error,
                    );
                } else {
                    app.config = libllm::config::load();
                    app.theme = crate::tui::theme::resolve_theme(&app.config);
                    app.invalidate_chat_cache();
                }
                app.theme_dialog = None;
            }
            dialogs::TabbedFieldAction::OpenSelector { section: 0, field: 0 } => {
                open_base_theme_picker(app);
            }
            dialogs::TabbedFieldAction::OpenSelector { .. } => {}
            dialogs::TabbedFieldAction::InvokeAction { section: 0, field: 2 } => {
                app.delete_confirm_filename = "all color overrides".to_owned();
                app.delete_confirm_selected = 1;
                app.delete_context = DeleteContext::ThemeResetColors;
                app.focus = Focus::DeleteConfirmDialog;
            }
            dialogs::TabbedFieldAction::InvokeAction { section: 0, field: 3 } => {
                if let Some(dialog) = app.theme_dialog.as_mut() {
                    for section in dialog.sections_mut() {
                        section.values = section.original_values.clone();
                    }
                }
                app.config = libllm::config::load();
                app.theme = crate::tui::theme::resolve_theme(&app.config);
                app.invalidate_chat_cache();
                app.theme_dialog = None;
                app.focus = Focus::Input;
            }
            dialogs::TabbedFieldAction::InvokeAction { .. } => {}
        }
        return None;
    }

    let dialog = match kind {
        DialogKind::Config => unreachable!(),
        DialogKind::Theme => unreachable!(),
        DialogKind::PresetEditor => app.preset_editor.as_mut(),
        DialogKind::PersonaEditor => app.persona_editor.as_mut(),
        DialogKind::CharacterEditor => app.character_editor.as_mut(),
        DialogKind::SystemPromptEditor => app.system_prompt_editor.as_mut(),
        DialogKind::WorldbookEntryEditor => app.worldbook_entry_editor.as_mut(),
    };

    let Some(dialog) = dialog else {
        return None;
    };

    let result = dialog.handle_key(key);

    if let Some(msg) = dialog.clipboard_warning.take() {
        app.set_status(msg, StatusLevel::Warning);
    }

    if matches!(kind, DialogKind::WorldbookEntryEditor) {
        if let Some(ref mut d) = app.worldbook_entry_editor {
            let selective = d
                .values
                .get(2)
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));
            d.hidden_fields = if selective { Vec::new() } else { vec![3] };
        }
    }

    match result {
        dialogs::FieldDialogAction::Continue => None,
        dialogs::FieldDialogAction::OpenSelector(_field_index) => None,
        dialogs::FieldDialogAction::Close => {
            match kind {
                DialogKind::Config => unreachable!(),
                DialogKind::Theme => unreachable!(),
                DialogKind::PresetEditor => {
                    if !app.preset_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                    } else {
                        let editor = app.preset_editor.as_ref().unwrap();
                        let original_name = app.preset_editor_original_name.clone();
                        let edited_preset_name = editor.values[0].trim().to_owned();
                        match dialogs::preset::save_preset_from_editor(
                            app.preset_editor_kind,
                            &editor.values,
                            &original_name,
                        ) {
                            Ok(()) => {
                                app.set_status("Preset saved.".to_owned(), StatusLevel::Info);
                                dialogs::preset::refresh_preset_list(app);
                                if matches!(
                                    app.preset_editor_kind,
                                    dialogs::preset::PresetKind::Instruct
                                ) && app.instruct_preset.name == original_name
                                {
                                    let resolve_name = if edited_preset_name.is_empty() {
                                        &original_name
                                    } else {
                                        &edited_preset_name
                                    };
                                    app.instruct_preset =
                                        libllm::preset::resolve_instruct_preset(resolve_name);
                                    app.stop_tokens = app.instruct_preset.stop_tokens();
                                }
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save preset: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                    }
                    app.preset_editor = None;
                    app.focus = Focus::PresetPickerDialog;
                    return None;
                }
                DialogKind::PersonaEditor => {
                    let is_cli_locked = app.cli_overrides.persona.is_some();
                    if is_cli_locked {
                        app.persona_editor = None;
                        app.focus = Focus::Input;
                    } else if !app.persona_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.persona_editor = None;
                        app.focus = Focus::PersonaDialog;
                    } else {
                        let values = &app.persona_editor.as_ref().unwrap().values;
                        let file_name = app.persona_editor_file_name.clone();
                        let persona = libllm::persona::PersonaFile {
                            name: values[0].clone(),
                            persona: values[1].clone(),
                        };

                        if file_name != persona.name
                            && app.persona_list.iter().any(|n| n == &persona.name)
                        {
                            app.set_status(
                                format!("Name '{}' is already in use.", persona.name),
                                StatusLevel::Error,
                            );
                            return None;
                        }

                        let new_slug = libllm::character::slugify(&persona.name);
                        if !file_name.is_empty() && file_name != persona.name {
                            let old_slug = libllm::character::slugify(&file_name);
                            if let Some(ref db) = app.db {
                                let _ = db.delete_persona(&old_slug);
                            }
                        }
                        match app
                            .db
                            .as_ref()
                            .map(|db| {
                                if db.load_persona(&new_slug).is_ok() {
                                    db.update_persona(&new_slug, &persona)
                                } else {
                                    db.insert_persona(&new_slug, &persona)
                                }
                            })
                            .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
                        {
                            Ok(_) => {
                                app.invalidate_chat_cache();
                                if app.session.persona.as_deref() == Some(&file_name)
                                    || app.session.persona.as_deref()
                                        == Some(persona.name.as_str())
                                {
                                    app.active_persona_name = Some(persona.name.clone());
                                    app.active_persona_desc = Some(persona.persona.clone());
                                    app.session.persona = Some(persona.name.clone());
                                }
                                app.set_status(
                                    format!("Persona '{}' saved.", persona.name),
                                    StatusLevel::Info,
                                );
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save persona: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                        app.persona_editor = None;
                        maintenance::reload_persona_picker(app);
                        app.focus = Focus::PersonaDialog;
                    }
                    return None;
                }
                DialogKind::SystemPromptEditor => {
                    if app.system_editor_read_only {
                        app.system_prompt_editor = None;
                        app.system_editor_read_only = false;
                        app.focus = app.system_editor_return_focus;
                        return None;
                    }

                    if !app.system_prompt_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.system_prompt_editor = None;
                        app.focus = app.system_editor_return_focus;
                        return None;
                    }

                    let values = &app.system_prompt_editor.as_ref().unwrap().values;
                    let new_name = values[0].clone();
                    let content = values[1].clone();
                    let original_name = app.system_editor_prompt_name.clone();

                    if original_name != new_name
                        && app.system_prompt_list.iter().any(|n| n == &new_name)
                    {
                        app.set_status(
                            format!("Name '{new_name}' is already in use."),
                            StatusLevel::Error,
                        );
                        return None;
                    }

                    let value = if content.trim().is_empty() {
                        None
                    } else {
                        Some(content.clone())
                    };
                    app.session.system_prompt = value;
                    app.invalidate_chat_cache();
                    app.mark_session_dirty(SaveTrigger::Debounced, false);

                    if !original_name.is_empty() {
                        let prompt = libllm::system_prompt::SystemPromptFile {
                            name: new_name.clone(),
                            content,
                        };
                        let new_slug = libllm::character::slugify(&new_name);
                        let save_result = app
                            .db
                            .as_ref()
                            .map(|db| {
                                if original_name != new_name {
                                    let old_slug = libllm::character::slugify(&original_name);
                                    let _ = db.delete_prompt(&old_slug);
                                }
                                if db.load_prompt(&new_slug).is_ok() {
                                    db.update_prompt(&new_slug, &prompt)
                                } else {
                                    db.insert_prompt(&new_slug, &prompt, false)
                                }
                            })
                            .unwrap_or_else(|| Err(anyhow::anyhow!("no database")));
                        match save_result {
                            Ok(()) => {
                                let prompts = app
                                    .db
                                    .as_ref()
                                    .and_then(|db| db.list_prompts().ok())
                                    .unwrap_or_default();
                                app.system_prompt_list =
                                    prompts.into_iter().map(|e| e.name).collect();
                                app.set_status(
                                    format!("System prompt '{}' saved.", new_name),
                                    StatusLevel::Info,
                                );
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save prompt: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                    }

                    app.system_prompt_editor = None;
                    app.focus = app.system_editor_return_focus;
                    return None;
                }
                DialogKind::CharacterEditor => {
                    if !app.character_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.character_editor = None;
                        app.focus = Focus::CharacterDialog;
                        return None;
                    }

                    let values = &app.character_editor.as_ref().unwrap().values;
                    let new_slug = libllm::character::slugify(&values[0]);
                    if new_slug != app.character_editor_slug
                        && app.character_slugs.iter().any(|s| s == &new_slug)
                    {
                        app.set_status(
                            format!("Name '{}' is already in use.", values[0]),
                            StatusLevel::Error,
                        );
                        return None;
                    }

                    let card = libllm::character::CharacterCard {
                        name: values[0].clone(),
                        description: values[1].clone(),
                        personality: values[2].clone(),
                        scenario: values[3].clone(),
                        first_mes: values[4].clone(),
                        mes_example: values[5].clone(),
                        system_prompt: values[6].clone(),
                        post_history_instructions: values[7].clone(),
                        alternate_greetings: Vec::new(),
                    };
                    let old_slug = app.character_editor_slug.clone();
                    let save_result = app
                        .db
                        .as_ref()
                        .map(|db| {
                            if new_slug != old_slug {
                                let _ = db.delete_character(&old_slug);
                            }
                            if db.load_character(&new_slug).is_ok() {
                                db.update_character(&new_slug, &card)
                            } else {
                                db.insert_character(&new_slug, &card)
                            }
                        })
                        .unwrap_or_else(|| Err(anyhow::anyhow!("no database")));
                    match save_result {
                        Ok(()) => {
                            let chars = app
                                .db
                                .as_ref()
                                .and_then(|db| db.list_characters().ok())
                                .unwrap_or_default();
                            app.character_names =
                                chars.iter().map(|(_, name)| name.clone()).collect();
                            app.character_slugs =
                                chars.into_iter().map(|(slug, _)| slug).collect();
                            app.character_selected = app
                                .character_slugs
                                .iter()
                                .position(|existing| existing == &new_slug)
                                .unwrap_or(0)
                                .min(app.character_slugs.len().saturating_sub(1));
                            app.character_editor_slug = new_slug.clone();
                            app.set_status(
                                format!("Saved character: {}", card.name),
                                StatusLevel::Info,
                            );
                            let is_active =
                                app.session.character.as_deref().is_some_and(|name| {
                                    libllm::character::slugify(name) == app.character_editor_slug
                                });
                            if is_active {
                                let cfg = libllm::config::load();
                                let tpl_name =
                                    cfg.template_preset.as_deref().unwrap_or("Default");
                                let tpl = libllm::preset::resolve_template_preset(tpl_name);
                                app.session.system_prompt = Some(
                                    libllm::character::build_system_prompt(&card, Some(&tpl)),
                                );
                                app.session.character = Some(card.name.clone());
                                app.invalidate_chat_cache();
                            }
                        }
                        Err(e) => app.set_status(
                            format!("Failed to save character: {e}"),
                            StatusLevel::Error,
                        ),
                    }
                    app.character_editor = None;
                    app.focus = Focus::CharacterDialog;
                    return None;
                }
                DialogKind::WorldbookEntryEditor => {
                    if !app.worldbook_entry_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                    } else {
                        let values = &app.worldbook_entry_editor.as_ref().unwrap().values;
                        let idx = app.worldbook_entry_editor_index;
                        if idx < app.worldbook_editor_entries.len() {
                            app.worldbook_editor_entries[idx] =
                                dialogs::worldbook::values_to_entry(
                                    values,
                                    &app.worldbook_editor_entries[idx],
                                );
                        }
                    }
                    app.worldbook_entry_editor = None;
                    app.focus = Focus::WorldbookEditorDialog;
                    return None;
                }
            }
            None
        }
    }
}

fn non_empty_opt(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

fn live_apply_theme_dialog(app: &mut App) {
    let Some(dialog) = app.theme_dialog.as_ref() else {
        return;
    };
    let sections: Vec<Vec<String>> = dialog
        .sections()
        .iter()
        .map(|s| s.values.clone())
        .collect();
    let base_theme = sections[0][0].clone();
    let mut preview = app.config.clone();
    preview.theme = Some(base_theme);
    preview.theme_colors = Some(libllm::config::ThemeColorOverrides {
        user_message: non_empty_opt(&sections[1][0]),
        assistant_message_fg: non_empty_opt(&sections[1][1]),
        assistant_message_bg: non_empty_opt(&sections[1][2]),
        system_message: non_empty_opt(&sections[1][3]),
        dialogue: non_empty_opt(&sections[1][4]),
        border_focused: non_empty_opt(&sections[2][0]),
        border_unfocused: non_empty_opt(&sections[2][1]),
        status_bar_fg: non_empty_opt(&sections[2][2]),
        status_bar_bg: non_empty_opt(&sections[2][3]),
        status_error_fg: non_empty_opt(&sections[2][4]),
        status_error_bg: non_empty_opt(&sections[2][5]),
        status_info_fg: non_empty_opt(&sections[2][6]),
        status_info_bg: non_empty_opt(&sections[2][7]),
        status_warning_fg: non_empty_opt(&sections[2][8]),
        status_warning_bg: non_empty_opt(&sections[2][9]),
        nav_cursor_fg: non_empty_opt(&sections[3][0]),
        nav_cursor_bg: non_empty_opt(&sections[3][1]),
        hover_bg: non_empty_opt(&sections[3][2]),
        sidebar_highlight_fg: non_empty_opt(&sections[3][3]),
        sidebar_highlight_bg: non_empty_opt(&sections[3][4]),
        dimmed: non_empty_opt(&sections[3][5]),
        command_picker_fg: non_empty_opt(&sections[3][6]),
        command_picker_bg: non_empty_opt(&sections[3][7]),
        streaming_indicator: non_empty_opt(&sections[4][0]),
        api_unavailable: non_empty_opt(&sections[4][1]),
        summary_indicator: non_empty_opt(&sections[4][2]),
    });
    app.theme = crate::tui::theme::resolve_theme(&preview);
    app.invalidate_chat_cache();
}

pub(super) fn open_base_theme_picker(app: &mut App) {
    let names: Vec<String> = crate::tui::theme::Theme::available_themes()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let current = app
        .theme_dialog
        .as_ref()
        .map(|d| d.sections()[0].values[0].clone())
        .unwrap_or_default();
    let selected = names.iter().position(|n| *n == current).unwrap_or(0);
    app.base_theme_picker_names = names;
    app.base_theme_picker_selected = selected;
    app.focus = Focus::BaseThemePickerDialog;
}
