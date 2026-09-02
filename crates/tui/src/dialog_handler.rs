//! Dialog-level key event routing and generation cancellation logic.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Style;
use tui_textarea::TextArea;

use libllm_core::session::{Message, Role};

use super::types::*;
use super::{business, dialogs, maintenance};

#[expect(
    clippy::expect_used,
    reason = "a stream is only cancelled after a user message gave the tree a head node"
)]
pub(super) fn cancel_generation(app: &mut App) {
    if let Some(handle) = app.streaming.task.take() {
        handle.abort();
    }

    if app.streaming.is_continuation {
        if !app.streaming.buffer.is_empty() {
            let Some(head) = app.session.tree.head() else {
                tracing::debug!("tui.cancel_generation.no_head");
                return;
            };
            let combined = match app.streaming.prefill.take() {
                Some(prefill) => format!("{}{}", prefill, app.streaming.buffer.trim_start()),
                None => {
                    let existing = app
                        .session
                        .tree
                        .node(head)
                        .expect("head id resolves to message node")
                        .message
                        .content
                        .clone();
                    format!("{}{}", existing, app.streaming.buffer)
                }
            };
            let combined = libllm_core::regex_rules::apply(
                &app.compiled_regex,
                libllm_core::regex_rules::Scope::PromptRecv,
                Role::Assistant,
                &combined,
            )
            .into_owned();
            app.session.tree.set_message_content(head, combined);
            let current_seconds = app
                .session
                .tree
                .node(head)
                .and_then(|node| node.message.thought_seconds);
            let measured_seconds = libllm_core::thought::measured_thought_seconds(
                app.streaming.started_at,
                app.streaming.first_think_closed_at,
            );
            let final_seconds = libllm_core::thought::resolve_thought_seconds(
                &app.session
                    .tree
                    .node(head)
                    .expect("head id resolves to message node")
                    .message
                    .content,
                current_seconds,
                measured_seconds,
                app.reasoning_preset.as_ref(),
            );
            app.session
                .tree
                .set_message_thought_seconds(head, final_seconds);
        }
        app.streaming.is_continuation = false;
    } else if !app.streaming.buffer.is_empty() {
        let raw = std::mem::take(&mut app.streaming.buffer);
        let Some(head) = app.session.tree.head() else {
            tracing::debug!("tui.cancel_generation.no_head");
            return;
        };
        let stored_content =
            libllm_core::thought::normalize_assistant_content(&raw, app.reasoning_preset.as_ref())
                .into_owned();
        let stored_content = libllm_core::regex_rules::apply(
            &app.compiled_regex,
            libllm_core::regex_rules::Scope::PromptRecv,
            Role::Assistant,
            &stored_content,
        )
        .into_owned();
        let measured_seconds = libllm_core::thought::measured_thought_seconds(
            app.streaming.started_at,
            app.streaming.first_think_closed_at,
        );
        let thought_seconds = libllm_core::thought::resolve_thought_seconds(
            &stored_content,
            None,
            measured_seconds,
            app.reasoning_preset.as_ref(),
        );
        app.session.tree.push(
            Some(head),
            Message::new(Role::Assistant, stored_content).with_thought_seconds(thought_seconds),
        );
    }

    app.streaming.buffer.clear();
    app.streaming.prefill = None;
    app.streaming.active = false;
    app.streaming.started_at = None;
    app.streaming.first_think_closed_at = None;
    app.mark_session_dirty(SaveTrigger::StreamDone, true);
    app.invalidate_chat_caches();
    app.auto_scroll = true;
}

pub(super) fn open_edit_dialog_with(app: &mut App, content: &str) {
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let mut editor = TextArea::from(if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    });
    configure_textarea_at_start(&mut editor);
    app.edit.editor = Some(editor);
    app.edit.scroll_top = 0;
    app.edit.original_content = content.lines().collect::<Vec<_>>().join("\n");
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

pub(super) fn configure_textarea_at_start(ta: &mut TextArea<'_>) {
    configure_textarea(ta);
    ta.move_cursor(tui_textarea::CursorMove::Top);
    ta.move_cursor(tui_textarea::CursorMove::Head);
}

/// Dispatch `key` to `ta.input()`. When Up/Down would have had no effect --
/// because the caret is already on the first/last visual row -- jump to
/// start/end of line instead, so single-line content behaves like a
/// single-line field and multi-line content lands cleanly at either edge.
pub(super) fn input_with_eof_jump(ta: &mut TextArea<'_>, key: KeyEvent) {
    let before = ta.cursor();
    ta.input(key);
    if ta.cursor() != before {
        return;
    }
    match key.code {
        KeyCode::Down => ta.move_cursor(tui_textarea::CursorMove::End),
        KeyCode::Up => ta.move_cursor(tui_textarea::CursorMove::Head),
        _ => {}
    }
}

#[derive(Clone, Copy)]
pub(super) enum DialogKind {
    Config,
    Theme,
    PresetEditor,
    PersonaEditor,
    AuthorNoteEditor,
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
        let dialog = app.config_dialog.as_mut()?;
        if dialog.current_tab() == crate::dialogs::danger_tab::DANGER_TAB_INDEX {
            use crate::dialogs::danger_tab::{DangerTabResult, handle_danger_tab_key};
            let mut sel = app.danger.selected;
            let result = handle_danger_tab_key(key, &mut sel);
            app.danger.selected = sel;
            match result {
                DangerTabResult::Pending => return None,
                DangerTabResult::OpenConfirm(op) => {
                    if matches!(op, crate::types::DangerOp::DestroyAll) {
                        let challenge = crate::dialogs::danger_typed_confirm::generate_challenge();
                        let snapshot_path = std::env::temp_dir()
                            .join(format!("libllm-{}.tar.zst", uuid::Uuid::new_v4()));
                        app.danger.typed_confirm = Some(TypedConfirmState {
                            challenge,
                            input: String::new(),
                            cursor_pos: 0,
                            op,
                            snapshot_path,
                            focus_idx: 0,
                        });
                        app.focus = Focus::DangerTypedConfirmDialog;
                    } else {
                        app.danger.confirm_op = Some(op);
                        app.danger.confirm_selected = Some(0);
                        app.focus = Focus::DangerConfirmDialog;
                    }
                    return None;
                }
                DangerTabResult::Passthrough => {}
            }
        }
        let action = dialog.handle_key(key);
        if let Some(msg) = dialog.clipboard_warning.take() {
            app.set_status(msg, StatusLevel::Warning);
        }
        match action {
            dialogs::TabbedFieldAction::Continue => {}
            dialogs::TabbedFieldAction::Close | dialogs::TabbedFieldAction::SaveAndClose => {
                commit_config_dialog(app);
            }
            dialogs::TabbedFieldAction::RequestUnsavedWarning => {
                app.unsaved_warning = Some(
                    crate::dialogs::unsaved_warning::UnsavedWarningState::new(Focus::ConfigDialog),
                );
                app.focus = Focus::UnsavedWarningDialog;
            }
            dialogs::TabbedFieldAction::OpenSelector {
                section: 0,
                field: 1,
            } => {
                crate::dialogs::auth::open_auth_dialog(app);
            }
            dialogs::TabbedFieldAction::OpenSelector {
                section: 0,
                field: 2,
            } => {
                crate::dialogs::preset::open_preset_picker(
                    app,
                    crate::dialogs::preset::PresetKind::Template,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector {
                section: 0,
                field: 3,
            } => {
                crate::dialogs::preset::open_preset_picker(
                    app,
                    crate::dialogs::preset::PresetKind::Instruct,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector {
                section: 0,
                field: 4,
            } => {
                crate::dialogs::preset::open_preset_picker(
                    app,
                    crate::dialogs::preset::PresetKind::Reasoning,
                );
            }
            dialogs::TabbedFieldAction::OpenSelector { .. } => {}
            dialogs::TabbedFieldAction::InvokeAction { .. } => {}
        }
        return None;
    }

    if matches!(kind, DialogKind::Theme) {
        let dialog = app.theme_ui.dialog.as_mut()?;
        let action = dialog.handle_key(key);
        let value_changed = dialog.take_value_changed();
        if value_changed {
            live_apply_theme_dialog(app);
        }
        match action {
            dialogs::TabbedFieldAction::Continue => {}
            dialogs::TabbedFieldAction::Close | dialogs::TabbedFieldAction::SaveAndClose => {
                commit_theme_dialog(app);
            }
            dialogs::TabbedFieldAction::RequestUnsavedWarning => {
                app.unsaved_warning = Some(
                    crate::dialogs::unsaved_warning::UnsavedWarningState::new(Focus::ThemeDialog),
                );
                app.focus = Focus::UnsavedWarningDialog;
            }
            dialogs::TabbedFieldAction::OpenSelector {
                section: 0,
                field: 0,
            } => {
                open_base_theme_picker(app);
            }
            dialogs::TabbedFieldAction::OpenSelector { .. } => {}
            dialogs::TabbedFieldAction::InvokeAction {
                section: 0,
                field: 2,
            } => {
                app.delete_confirm.filename = "all color overrides".to_owned();
                app.delete_confirm.selected = 1;
                app.delete_confirm.context = DeleteContext::ThemeResetColors;
                app.focus = Focus::DeleteConfirmDialog;
            }
            dialogs::TabbedFieldAction::InvokeAction {
                section: 0,
                field: 3,
            } => {
                if let Some(dialog) = app.theme_ui.dialog.as_mut() {
                    for section in dialog.sections_mut() {
                        section.values = section.original_values.clone();
                    }
                }
                app.config = libllm_config::load();
                app.theme = crate::theme::resolve_theme(&app.config);
                app.invalidate_chat_render_cache();
                app.theme_ui.dialog = None;
                return_to_input(app);
            }
            dialogs::TabbedFieldAction::InvokeAction { .. } => {}
        }
        return None;
    }

    let dialog = match kind {
        DialogKind::Config => unreachable!(),
        DialogKind::Theme => unreachable!(),
        DialogKind::PresetEditor => app.preset.editor.as_mut(),
        DialogKind::PersonaEditor => app.persona.editor.as_mut(),
        DialogKind::AuthorNoteEditor => app.author_note_editor.as_mut(),
        DialogKind::CharacterEditor => app.character.editor.as_mut(),
        DialogKind::SystemPromptEditor => app.system_prompt.editor.as_mut(),
        DialogKind::WorldbookEntryEditor => app.worldbook.entry_editor.as_mut(),
    };

    let dialog = dialog?;

    let result = dialog.handle_key(key);

    if let Some(msg) = dialog.clipboard_warning.take() {
        app.set_status(msg, StatusLevel::Warning);
    }

    if matches!(kind, DialogKind::WorldbookEntryEditor)
        && let Some(ref mut d) = app.worldbook.entry_editor
    {
        let selective = d
            .values
            .get(2)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        d.hidden_fields = if selective { Vec::new() } else { vec![3] };
    }

    match result {
        dialogs::FieldDialogAction::Continue => None,
        dialogs::FieldDialogAction::OpenSelector(_field_index) => None,
        dialogs::FieldDialogAction::Close => {
            discard_field_dialog(app, kind);
            None
        }
        dialogs::FieldDialogAction::SaveAndClose => commit_field_dialog(app, kind),
        dialogs::FieldDialogAction::RequestUnsavedWarning => {
            app.unsaved_warning = Some(crate::dialogs::unsaved_warning::UnsavedWarningState::new(
                field_dialog_focus(kind),
            ));
            app.focus = Focus::UnsavedWarningDialog;
            None
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "each DialogKind arm runs only while its editor is open, so the editor Option is Some"
)]
pub(super) fn commit_field_dialog(app: &mut App, kind: DialogKind) -> Option<Action> {
    match kind {
        DialogKind::Config => unreachable!(),
        DialogKind::Theme => unreachable!(),
        DialogKind::PresetEditor => {
            if !app
                .preset
                .editor
                .as_ref()
                .expect("preset editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
            } else {
                let editor = app
                    .preset
                    .editor
                    .as_ref()
                    .expect("preset editor is present while its dialog is focused");
                let original_name = app.preset.editor_original_name.clone();
                let edited_preset_name = editor.values[0].trim().to_owned();
                match dialogs::preset::save_preset_from_editor(
                    app.preset.editor_kind,
                    &editor.values,
                    &original_name,
                ) {
                    Ok(()) => {
                        app.set_status("Preset saved.".to_owned(), StatusLevel::Info);
                        dialogs::preset::refresh_preset_list(app);
                        if matches!(
                            app.preset.editor_kind,
                            dialogs::preset::PresetKind::Instruct
                        ) && app.instruct_preset.name == original_name
                        {
                            let resolve_name = if edited_preset_name.is_empty() {
                                &original_name
                            } else {
                                &edited_preset_name
                            };
                            app.instruct_preset = libllm_core::preset::resolve_instruct_preset(
                                resolve_name,
                                &libllm_config::instruct_presets_dir(),
                            );
                            app.stop_tokens = app.instruct_preset.stop_tokens();
                        }
                    }
                    Err(e) => {
                        app.set_status(format!("Failed to save preset: {e}"), StatusLevel::Error);
                    }
                }
            }
            app.preset.editor = None;
            app.focus = Focus::PresetPickerDialog;
            None
        }
        DialogKind::PersonaEditor => {
            let is_cli_locked = app.cli_overrides.persona.is_some();
            if is_cli_locked {
                app.persona.editor = None;
                return_to_input(app);
            } else if !app
                .persona
                .editor
                .as_ref()
                .expect("persona editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                app.persona.editor = None;
                app.focus = Focus::PersonaDialog;
            } else {
                let values = &app
                    .persona
                    .editor
                    .as_ref()
                    .expect("persona editor is present while its dialog is focused")
                    .values;
                let old_slug = app.persona.editor_slug.clone();
                let persona = libllm_core::persona::PersonaFile {
                    name: values[0].clone(),
                    persona: values[1].clone(),
                };

                let new_slug = libllm_core::character::slugify(&persona.name);
                if new_slug != old_slug && app.persona.slugs.iter().any(|s| s == &new_slug) {
                    app.set_status(
                        format!("Name '{}' is already in use.", persona.name),
                        StatusLevel::Error,
                    );
                    return None;
                }

                if !old_slug.is_empty()
                    && new_slug != old_slug
                    && let Some(ref db) = app.db
                {
                    let _ = db.delete_persona(&old_slug);
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
                    .map(|r| r.map_err(anyhow::Error::from))
                    .unwrap_or_else(|| Err(anyhow::anyhow!("no database")))
                {
                    Ok(_) => {
                        if app.session.persona.as_deref() == Some(old_slug.as_str()) {
                            app.invalidate_prompt_cache();
                            app.invalidate_chat_render_cache();
                            app.persona.active_name = Some(persona.name.clone());
                            app.persona.active_desc = Some(persona.persona.clone());
                            app.session.persona = Some(new_slug.clone());
                        }
                        app.persona.editor_slug = new_slug;
                        app.set_status(
                            format!("Persona '{}' saved.", persona.name),
                            StatusLevel::Info,
                        );
                    }
                    Err(e) => {
                        app.set_status(format!("Failed to save persona: {e}"), StatusLevel::Error);
                    }
                }
                app.persona.editor = None;
                maintenance::reload_persona_picker(app);
                app.focus = Focus::PersonaDialog;
            }
            None
        }
        DialogKind::AuthorNoteEditor => {
            if !app
                .author_note_editor
                .as_ref()
                .expect("author_note editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                app.author_note_editor = None;
                return_to_input(app);
                return None;
            }

            let dialog = app.author_note_editor.take()?;
            let values = &dialog.values;
            let text = values.first().cloned().unwrap_or_default();
            let depth_str = values
                .get(1)
                .cloned()
                .unwrap_or_else(|| libllm_core::author_note::DEFAULT_DEPTH.to_string());
            let at_top_str = values.get(2).cloned().unwrap_or_else(|| "false".to_owned());

            let depth = depth_str
                .trim()
                .parse::<u32>()
                .unwrap_or(libllm_core::author_note::DEFAULT_DEPTH);
            let at_top = at_top_str == "true";

            app.session.author_note =
                libllm_core::author_note::AuthorNote::from_row_parts(Some(text), depth, at_top);
            app.mark_session_dirty(SaveTrigger::Debounced, false);
            app.invalidate_chat_caches();
            return_to_input(app);
            None
        }
        DialogKind::SystemPromptEditor => {
            if app.system_prompt.editor_read_only {
                app.system_prompt.editor = None;
                app.system_prompt.editor_read_only = false;
                app.focus = app.system_prompt.editor_return_focus;
                return None;
            }

            if !app
                .system_prompt
                .editor
                .as_ref()
                .expect("system_prompt editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                app.system_prompt.editor = None;
                app.focus = app.system_prompt.editor_return_focus;
                return None;
            }

            let values = &app
                .system_prompt
                .editor
                .as_ref()
                .expect("system_prompt editor is present while its dialog is focused")
                .values;
            let new_name = values[0].clone();
            let content = values[1].clone();
            let original_name = app.system_prompt.editor_prompt_name.clone();

            if original_name != new_name && app.system_prompt.list.iter().any(|n| n == &new_name) {
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
            app.invalidate_prompt_cache();
            app.mark_session_dirty(SaveTrigger::Debounced, false);

            if !original_name.is_empty() {
                let prompt = libllm_core::system_prompt::SystemPromptFile {
                    name: new_name.clone(),
                    content,
                };
                let new_slug = libllm_core::character::slugify(&new_name);
                let old_slug = libllm_core::character::slugify(&original_name);
                let save_result = app
                            .db
                            .as_ref()
                            .map(|db| {
                                if original_name == new_name || old_slug == new_slug {
                                    db.update_prompt(&new_slug, &prompt)
                                        .map_err(anyhow::Error::from)
                                } else if db.load_prompt(&new_slug).is_ok() {
                                    anyhow::bail!(
                                        "name '{}' conflicts with an existing prompt after slug normalization",
                                        new_name
                                    )
                                } else {
                                    db.rename_prompt(&old_slug, &new_slug, &prompt)
                                        .map_err(anyhow::Error::from)
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
                        app.system_prompt.list = prompts.into_iter().map(|e| e.name).collect();
                        app.set_status(
                            format!("System prompt '{}' saved.", new_name),
                            StatusLevel::Info,
                        );
                    }
                    Err(e) => {
                        app.set_status(format!("Failed to save prompt: {e}"), StatusLevel::Error);
                    }
                }
            }

            app.system_prompt.editor = None;
            app.focus = app.system_prompt.editor_return_focus;
            None
        }
        DialogKind::CharacterEditor => {
            if !app
                .character
                .editor
                .as_ref()
                .expect("character editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                app.character.editor = None;
                app.focus = Focus::CharacterDialog;
                return None;
            }

            let values = &app
                .character
                .editor
                .as_ref()
                .expect("character editor is present while its dialog is focused")
                .values;
            let new_slug = libllm_core::character::slugify(&values[0]);
            if new_slug != app.character.editor_slug
                && app.character.slugs.iter().any(|s| s == &new_slug)
            {
                app.set_status(
                    format!("Name '{}' is already in use.", values[0]),
                    StatusLevel::Error,
                );
                return None;
            }

            let note_depth = values
                .get(9)
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(libllm_core::author_note::DEFAULT_DEPTH);
            let note_at_top = values.get(10).is_some_and(|s| s == "true");
            let card = libllm_core::character::CharacterCard {
                name: values[0].clone(),
                description: values[1].clone(),
                personality: values[2].clone(),
                scenario: values[3].clone(),
                first_mes: values[4].clone(),
                mes_example: values[5].clone(),
                system_prompt: values[6].clone(),
                post_history_instructions: values[7].clone(),
                alternate_greetings: Vec::new(),
                author_note: libllm_core::author_note::AuthorNote::from_row_parts(
                    values.get(8).cloned(),
                    note_depth,
                    note_at_top,
                ),
            };
            let old_slug = app.character.editor_slug.clone();
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
                .map(|r| r.map_err(anyhow::Error::from))
                .unwrap_or_else(|| Err(anyhow::anyhow!("no database")));
            match save_result {
                Ok(()) => {
                    let chars = app
                        .db
                        .as_ref()
                        .and_then(|db| db.list_characters().ok())
                        .unwrap_or_default();
                    app.character.names = chars.iter().map(|(_, name)| name.clone()).collect();
                    app.character.slugs = chars.into_iter().map(|(slug, _)| slug).collect();
                    app.character.selected = app
                        .character
                        .slugs
                        .iter()
                        .position(|existing| existing == &new_slug)
                        .unwrap_or(0)
                        .min(app.character.slugs.len().saturating_sub(1));
                    app.character.editor_slug = new_slug.clone();
                    app.set_status(format!("Saved character: {}", card.name), StatusLevel::Info);
                    let is_active = app.session.character.as_deref().is_some_and(|name| {
                        libllm_core::character::slugify(name) == app.character.editor_slug
                    });
                    if is_active {
                        let cfg = libllm_config::load();
                        let tpl_name = cfg.template_preset.as_deref().unwrap_or("Default");
                        let tpl = libllm_core::preset::resolve_template_preset(
                            tpl_name,
                            &libllm_config::template_presets_dir(),
                        );
                        app.session.system_prompt = Some(
                            libllm_core::character::build_system_prompt(&card, Some(&tpl)),
                        );
                        app.session.character = Some(card.name.clone());
                        app.active_card_author_note = card.author_note.clone();
                        app.invalidate_chat_caches();
                    }
                }
                Err(e) => {
                    app.set_status(format!("Failed to save character: {e}"), StatusLevel::Error)
                }
            }
            app.character.editor = None;
            app.focus = Focus::CharacterDialog;
            None
        }
        DialogKind::WorldbookEntryEditor => {
            if !app
                .worldbook
                .entry_editor
                .as_ref()
                .expect("worldbook_entry editor is present while its dialog is focused")
                .has_changes()
            {
                app.set_status("No changes found.".to_owned(), StatusLevel::Info);
            } else {
                let values = &app
                    .worldbook
                    .entry_editor
                    .as_ref()
                    .expect("worldbook_entry editor is present while its dialog is focused")
                    .values;
                let idx = app.worldbook.entry_editor_index;
                if idx < app.worldbook.editor_entries.len() {
                    app.worldbook.editor_entries[idx] = dialogs::worldbook::values_to_entry(
                        values,
                        &app.worldbook.editor_entries[idx],
                    );
                }
            }
            app.worldbook.entry_editor = None;
            app.focus = Focus::WorldbookEditorDialog;
            None
        }
    }
}

fn field_dialog_focus(kind: DialogKind) -> Focus {
    match kind {
        DialogKind::Config => Focus::ConfigDialog,
        DialogKind::Theme => Focus::ThemeDialog,
        DialogKind::PresetEditor => Focus::PresetEditorDialog,
        DialogKind::PersonaEditor => Focus::PersonaEditorDialog,
        DialogKind::AuthorNoteEditor => Focus::AuthorNoteEditorDialog,
        DialogKind::CharacterEditor => Focus::CharacterEditorDialog,
        DialogKind::SystemPromptEditor => Focus::SystemPromptEditorDialog,
        DialogKind::WorldbookEntryEditor => Focus::WorldbookEntryEditorDialog,
    }
}

pub(super) fn discard_field_dialog(app: &mut App, kind: DialogKind) {
    match kind {
        DialogKind::Config => discard_config_dialog(app),
        DialogKind::Theme => discard_theme_dialog(app),
        DialogKind::PresetEditor => {
            app.preset.editor = None;
            app.focus = Focus::PresetPickerDialog;
        }
        DialogKind::PersonaEditor => {
            app.persona.editor = None;
            if app.cli_overrides.persona.is_some() {
                return_to_input(app);
            } else {
                app.focus = Focus::PersonaDialog;
            }
        }
        DialogKind::AuthorNoteEditor => {
            app.author_note_editor = None;
            return_to_input(app);
        }
        DialogKind::CharacterEditor => {
            app.character.editor = None;
            app.focus = Focus::CharacterDialog;
        }
        DialogKind::SystemPromptEditor => {
            app.system_prompt.editor = None;
            app.system_prompt.editor_read_only = false;
            app.focus = app.system_prompt.editor_return_focus;
        }
        DialogKind::WorldbookEntryEditor => {
            app.worldbook.entry_editor = None;
            app.focus = Focus::WorldbookEditorDialog;
        }
    }
}

pub(super) fn commit_config_dialog(app: &mut App) {
    let (has_changes, sections) = {
        let Some(dialog) = app.config_dialog.as_ref() else {
            return;
        };
        let has_changes = dialog.has_changes();
        let sections: Vec<Vec<String>> =
            dialog.sections().iter().map(|s| s.values.clone()).collect();
        (has_changes, sections)
    };
    if !has_changes {
        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
    } else {
        let existing = libllm_config::load();
        if let Err(e) =
            business::apply_tabbed_config_fields(&sections, existing, &app.cli_overrides)
        {
            app.set_status(format!("Failed to save config: {e}"), StatusLevel::Error);
        } else {
            business::apply_config(app);
            app.set_status("Config saved.".to_owned(), StatusLevel::Info);
        }
    }
    app.config_dialog = None;
    return_to_input(app);
}

pub(super) fn discard_config_dialog(app: &mut App) {
    app.config_dialog = None;
    return_to_input(app);
}

pub(super) fn commit_theme_dialog(app: &mut App) {
    let sections: Vec<Vec<String>> = match app.theme_ui.dialog.as_ref() {
        Some(d) => d.sections().iter().map(|s| s.values.clone()).collect(),
        None => return,
    };
    let existing = libllm_config::load();
    if let Err(e) = business::apply_theme_color_sections(&sections, existing) {
        app.set_status(format!("Failed to save theme: {e}"), StatusLevel::Error);
    } else {
        app.config = libllm_config::load();
        app.theme = crate::theme::resolve_theme(&app.config);
        app.invalidate_chat_render_cache();
    }
    app.theme_ui.dialog = None;
    return_to_input(app);
}

pub(super) fn discard_theme_dialog(app: &mut App) {
    app.config = libllm_config::load();
    app.theme = crate::theme::resolve_theme(&app.config);
    app.invalidate_chat_render_cache();
    app.theme_ui.dialog = None;
    return_to_input(app);
}

pub(crate) fn live_apply_theme_dialog(app: &mut App) {
    let Some(dialog) = app.theme_ui.dialog.as_ref() else {
        return;
    };
    let sections: Vec<Vec<String>> = dialog.sections().iter().map(|s| s.values.clone()).collect();
    let base_theme = sections[0][0].clone();
    let mut preview = app.config.clone();
    preview.theme = Some(base_theme);
    let overrides = business::build_theme_color_overrides(&sections);
    preview.theme_colors = Some(overrides);
    let new_theme = crate::theme::resolve_theme(&preview);
    if new_theme != app.theme {
        app.theme = new_theme;
        app.invalidate_chat_render_cache();
    }
}

pub(crate) fn return_to_input(app: &mut App) {
    if let Some(pending) = app.pending_template_prompt.take() {
        let dismissed = app
            .db
            .as_ref()
            .and_then(|db| db.is_template_dismissed(&pending.server_template_hash).ok())
            .unwrap_or(false);
        if dismissed {
            app.focus = Focus::Input;
        } else {
            app.template_prompt_state = Some(pending);
            app.focus = Focus::TemplatePromptDialog;
        }
    } else {
        app.focus = Focus::Input;
    }
}

pub(super) fn open_base_theme_picker(app: &mut App) {
    let names: Vec<String> = crate::theme::Theme::available_themes()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let current = app
        .theme_ui
        .dialog
        .as_ref()
        .map(|d| d.sections()[0].values[0].clone())
        .unwrap_or_default();
    let selected = names.iter().position(|n| *n == current).unwrap_or(0);
    app.theme_ui.base_picker_names = names;
    app.theme_ui.base_picker_selected = selected;
    app.focus = Focus::BaseThemePickerDialog;
}
