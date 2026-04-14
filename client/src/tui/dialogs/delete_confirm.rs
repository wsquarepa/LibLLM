//! Confirmation dialog for destructive delete operations on sessions and content.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, DeleteContext, Focus, business, maintenance};

pub(in crate::tui) enum ConfirmResult {
    Confirmed,
    Cancelled,
    Pending,
}

pub(in crate::tui) fn render_confirm_dialog(
    f: &mut ratatui::Frame,
    area: Rect,
    prompt: &str,
    selected: usize,
) -> Rect {
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, 6, area);

    let cancel_style = if selected == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let delete_style = if selected == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let lines = vec![
        Line::from(""),
        Line::from(format!("  {prompt}")),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(" Delete ", delete_style),
        ]),
        Line::from(""),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Confirm Delete ", Color::Yellow));

    f.render_widget(paragraph, dialog);
    dialog
}

pub(in crate::tui) fn handle_confirm_key(key: KeyEvent, selected: &mut usize) -> ConfirmResult {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            *selected = 1 - *selected;
            ConfirmResult::Pending
        }
        KeyCode::Enter => {
            if *selected == 1 {
                ConfirmResult::Confirmed
            } else {
                ConfirmResult::Cancelled
            }
        }
        KeyCode::Esc => ConfirmResult::Cancelled,
        _ => ConfirmResult::Pending,
    }
}

pub(in crate::tui) fn render_delete_confirm_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = render_confirm_dialog(
        f,
        area,
        &format!("Delete \"{}\"?", app.delete_confirm_filename),
        app.delete_confirm_selected,
    );
    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Left/Right: navigate  Enter: confirm  Esc: cancel",
        )],
    );
}

pub(in crate::tui) fn handle_delete_confirm_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match handle_confirm_key(key, &mut app.delete_confirm_selected) {
        ConfirmResult::Confirmed => {
            let context = std::mem::replace(&mut app.delete_context, DeleteContext::Session);
            match context {
                DeleteContext::Session => {
                    delete_selected_session(app);
                    app.focus = Focus::Sidebar;
                }
                DeleteContext::Character { slug } => {
                    delete_character(app, &slug);
                    app.focus = Focus::CharacterDialog;
                }
                DeleteContext::Persona { name } => {
                    delete_persona(app, &name);
                    app.focus = Focus::PersonaDialog;
                }
                DeleteContext::SystemPrompt { name } => {
                    delete_system_prompt(app, &name);
                    app.focus = Focus::SystemPromptDialog;
                }
                DeleteContext::Worldbook { name } => {
                    delete_worldbook(app, &name);
                    app.focus = Focus::WorldbookDialog;
                }
                DeleteContext::Preset { kind } => {
                    super::preset::delete_preset(kind, &app.delete_confirm_filename);
                    super::preset::refresh_preset_list(app);
                    app.focus = Focus::PresetPickerDialog;
                }
            }
        }
        ConfirmResult::Cancelled => {
            let context = std::mem::replace(&mut app.delete_context, DeleteContext::Session);
            app.focus = match context {
                DeleteContext::Session => Focus::Sidebar,
                DeleteContext::Character { .. } => Focus::CharacterDialog,
                DeleteContext::Persona { .. } => Focus::PersonaDialog,
                DeleteContext::SystemPrompt { .. } => Focus::SystemPromptDialog,
                DeleteContext::Worldbook { .. } => Focus::WorldbookDialog,
                DeleteContext::Preset { .. } => Focus::PresetPickerDialog,
            };
        }
        ConfirmResult::Pending => {}
    }
    None
}

fn delete_selected_session(app: &mut App) {
    let Some(selected) = app.sidebar_state.selected() else {
        return;
    };
    let entry = &app.sidebar_sessions[selected];
    if entry.is_new_chat {
        return;
    }

    let session_id = entry.id.clone();
    let is_current = app.save_mode.id() == Some(session_id.as_str());

    if let Some(ref db) = app.db {
        if let Err(e) = db.delete_session(&session_id) {
            app.set_status(
                format!("Error deleting: {e}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    }

    if is_current {
        app.discard_pending_session_save();
        let mut new_session = libllm::session::Session::default();
        new_session.persona = app.config.default_persona.clone();
        *app.session = new_session;
        business::load_active_persona(app);
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_id = libllm::session::generate_session_id();
        app.save_mode.set_id(new_id);
    }

    business::refresh_sidebar(app);
    app.set_status(
        format!("Deleted: {session_id}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_character(app: &mut App, slug: &str) {
    if let Some(ref db) = app.db {
        if let Err(e) = db.delete_character(slug) {
            app.set_status(
                format!("Error deleting character: {e}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    }

    maintenance::reload_character_picker(app);
    app.set_status(
        format!("Deleted character: {slug}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_persona(app: &mut App, name: &str) {
    let slug = libllm::character::slugify(name);
    if let Some(ref db) = app.db {
        if let Err(e) = db.delete_persona(&slug) {
            app.set_status(
                format!("Error deleting persona: {e}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    }

    if app.session.persona.as_deref() == Some(name) {
        app.active_persona_name = None;
        app.active_persona_desc = None;
        app.session.persona = None;
        app.invalidate_chat_cache();
    }

    maintenance::reload_persona_picker(app);
    app.set_status(
        format!("Deleted persona: {name}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_system_prompt(app: &mut App, name: &str) {
    let slug = libllm::character::slugify(name);
    if let Some(ref db) = app.db {
        if let Err(e) = db.delete_prompt(&slug) {
            app.set_status(
                format!("Error deleting prompt: {e}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    }

    maintenance::reload_system_prompt_picker(app);
    app.set_status(
        format!("Deleted prompt: {name}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_worldbook(app: &mut App, name: &str) {
    let slug = libllm::character::slugify(name);
    if let Some(ref db) = app.db {
        if let Err(e) = db.delete_worldbook(&slug) {
            app.set_status(
                format!("Error deleting worldbook: {e}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    }

    app.config.worldbooks.retain(|n| n != name);
    app.session.worldbooks.retain(|n| n != name);
    if let Err(e) = libllm::config::save(&app.config) {
        app.set_status(
            format!("Failed to save config: {e}"),
            super::super::StatusLevel::Error,
        );
    }
    app.invalidate_worldbook_cache();

    maintenance::reload_worldbook_picker(app);
    app.set_status(
        format!("Deleted worldbook: {name}"),
        super::super::StatusLevel::Info,
    );
}
