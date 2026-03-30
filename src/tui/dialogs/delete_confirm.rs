use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
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
    hint: Option<&str>,
    selected: usize,
) {
    let height = if hint.is_some() { 7 } else { 6 };
    let dialog = clear_centered(f, 50, height, area);

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

    let mut lines = vec![
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

    if let Some(hint_text) = hint {
        lines.push(Line::from(Span::styled(
            format!("  {hint_text}"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Confirm Delete ", Color::Yellow));

    f.render_widget(paragraph, dialog);
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
    render_confirm_dialog(
        f,
        area,
        &format!("Delete \"{}\"?", app.delete_confirm_filename),
        Some("Left/Right: navigate  Enter: confirm  Esc: cancel"),
        app.delete_confirm_selected,
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

    let path = entry.path.clone();
    let filename = entry.filename.clone();
    let is_current = app.save_mode.path().is_some_and(|p| p == path);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    crate::index::warn_if_save_fails(
        crate::index::remove_session(&path),
        "failed to remove session index entry",
    );

    if is_current {
        app.discard_pending_session_save();
        *app.session = crate::session::Session::default();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path = crate::config::sessions_dir().join(crate::session::generate_session_name());
        app.save_mode.set_path(new_path);
    }

    business::refresh_sidebar(app);
    app.set_status(
        format!("Deleted: {filename}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_character(app: &mut App, slug: &str) {
    let path = crate::character::resolve_card_path(&crate::config::characters_dir(), slug);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting character: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    crate::index::warn_if_save_fails(
        crate::index::remove_character(&path),
        "failed to remove character index entry",
    );

    maintenance::reload_character_picker(app);
    app.set_status(
        format!("Deleted character: {slug}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_persona(app: &mut App, name: &str) {
    let path = crate::persona::resolve_persona_path(&crate::config::personas_dir(), name);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting persona: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
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
    let path = crate::system_prompt::resolve_prompt_path(&crate::config::system_prompts_dir(), name);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting prompt: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }

    maintenance::reload_system_prompt_picker(app);
    app.set_status(
        format!("Deleted prompt: {name}"),
        super::super::StatusLevel::Info,
    );
}

fn delete_worldbook(app: &mut App, name: &str) {
    let path = crate::worldinfo::resolve_worldbook_path(&crate::config::worldinfo_dir(), name);

    if let Err(e) = std::fs::remove_file(&path) {
        app.set_status(
            format!("Error deleting worldbook: {e}"),
            super::super::StatusLevel::Error,
        );
        return;
    }
    crate::index::warn_if_save_fails(
        crate::index::remove_worldbook(&path),
        "failed to remove worldbook index entry",
    );

    app.config.worldbooks.retain(|n| n != name);
    app.session.worldbooks.retain(|n| n != name);
    let _ = crate::config::save(&app.config);
    app.invalidate_worldbook_cache();

    maintenance::reload_worldbook_picker(app);
    app.set_status(
        format!("Deleted worldbook: {name}"),
        super::super::StatusLevel::Info,
    );
}
