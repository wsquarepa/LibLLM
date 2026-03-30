use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App, DeleteContext, Focus};

pub(in crate::tui) fn render_system_prompt_dialog(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
) {
    let count = app.system_prompt_list.len();
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, count as u16 + super::LIST_DIALOG_TALL_PADDING, area);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.system_prompt_list.iter().enumerate() {
        let is_selected = i == app.system_prompt_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{name}"), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: select  Right: edit",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  a: add new  Del: delete  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Drop .txt to import",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(dialog_block(" System Prompts ", Color::Yellow));

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_system_prompt_dialog_key(
    key: KeyEvent,
    app: &mut App,
) -> Option<Action> {
    if app.system_prompt_list.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            super::move_selection_up(&mut app.system_prompt_selected);
        }
        KeyCode::Down => {
            super::move_selection_down(&mut app.system_prompt_selected, app.system_prompt_list.len());
        }
        KeyCode::Enter => {
            let name = app.system_prompt_list[app.system_prompt_selected].clone();
            let dir = crate::config::system_prompts_dir();
            let content =
                crate::system_prompt::load_prompt_content(&dir, &name, app.save_mode.key());

            app.session.system_prompt = content;
            app.invalidate_chat_cache();
            app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
            app.set_status(
                format!("System prompt set to '{name}'."),
                super::super::StatusLevel::Info,
            );
            app.focus = Focus::Input;
        }
        KeyCode::Right => {
            let name = app.system_prompt_list[app.system_prompt_selected].clone();
            open_prompt_editor(app, &name);
        }
        KeyCode::Char('a') => {
            let dir = crate::config::system_prompts_dir();
            let existing: std::collections::HashSet<String> =
                app.system_prompt_list.iter().cloned().collect();
            let new_name = super::generate_unique_name("custom", &existing);
            let prompt = crate::system_prompt::SystemPromptFile {
                name: new_name.clone(),
                content: String::new(),
            };
            if let Err(e) = crate::system_prompt::save_prompt(&prompt, &dir, app.save_mode.key()) {
                app.set_status(
                    format!("Failed to create prompt: {e}"),
                    super::super::StatusLevel::Error,
                );
                return None;
            }
            app.system_prompt_list.push(new_name.clone());
            app.system_prompt_selected = app.system_prompt_list.len() - 1;
            open_prompt_editor(app, &new_name);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.system_prompt_list[app.system_prompt_selected].clone();
            if name == crate::system_prompt::BUILTIN_ASSISTANT
                || name == crate::system_prompt::BUILTIN_ROLEPLAY
            {
                app.set_status(
                    "Cannot delete built-in prompts.".to_owned(),
                    super::super::StatusLevel::Warning,
                );
            } else {
                app.delete_confirm_filename = name.clone();
                app.delete_confirm_selected = 0;
                app.delete_context = DeleteContext::SystemPrompt { name };
                app.focus = Focus::DeleteConfirmDialog;
            }
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn open_prompt_editor(app: &mut App, name: &str) {
    let dir = crate::config::system_prompts_dir();
    let content =
        crate::system_prompt::load_prompt_content(&dir, name, app.save_mode.key())
            .unwrap_or_default();

    let is_roleplay = name == crate::system_prompt::BUILTIN_ROLEPLAY;

    let mut editor =
        tui_textarea::TextArea::from(content.lines().map(String::from).collect::<Vec<_>>());
    editor.set_cursor_line_style(ratatui::style::Style::default());
    editor.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);

    app.system_editor = Some(editor);
    app.system_editor_roleplay = is_roleplay;
    app.system_editor_prompt_name = name.to_owned();
    app.system_editor_return_focus = Focus::SystemPromptDialog;
    app.focus = Focus::SystemDialog;
}


pub(in crate::tui) fn handle_system_prompt_paste(
    path: &std::path::Path,
    ext: &str,
    app: &mut App,
) -> bool {
    if ext != "txt" {
        app.set_status(
            "System prompt import supports .txt files only.".to_owned(),
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
            app.set_status(format!("Cannot read file: {e}"), super::super::StatusLevel::Error);
            return true;
        }
        _ => {}
    }

    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => {
            app.set_status("Invalid filename.".to_owned(), super::super::StatusLevel::Error);
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

    let prompt = crate::system_prompt::SystemPromptFile {
        name: name.clone(),
        content,
    };
    let dir = crate::config::system_prompts_dir();
    match crate::system_prompt::save_prompt(&prompt, &dir, app.save_mode.key()) {
        Ok(_) => {
            let prompts = crate::system_prompt::list_prompts(&dir, app.save_mode.key());
            app.system_prompt_list = prompts.into_iter().map(|p| p.name).collect();
            app.system_prompt_selected = 0;
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
