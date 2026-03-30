use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_system_prompt_dialog(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
) {
    let count = app.system_prompt_list.len();
    let dialog = clear_centered(f, 50, count as u16 + 6, area);

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
        "  a: add new  Esc: cancel",
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
            app.system_prompt_selected = app.system_prompt_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.system_prompt_selected =
                (app.system_prompt_selected + 1).min(app.system_prompt_list.len() - 1);
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
            let new_name = generate_unique_name(&existing);
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

fn generate_unique_name(existing: &std::collections::HashSet<String>) -> String {
    let base = "custom";
    if !existing.contains(base) {
        return base.to_owned();
    }
    let mut i = 1u32;
    loop {
        let candidate = format!("{base}-{i}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}
