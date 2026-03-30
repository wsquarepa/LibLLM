use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App};

pub(in crate::tui) fn render_system_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let width = (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16;
    let height = (area.height as f32 * super::DIALOG_HEIGHT_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    let title = if app.system_editor_read_only {
        " System Prompt (read-only, set via -r) "
    } else if app.system_editor_roleplay {
        " System Prompt - Roleplay (Esc to save & close) "
    } else {
        " System Prompt - Assistant (Esc to save & close) "
    };

    let border_color = if app.system_editor_read_only {
        Color::Red
    } else {
        Color::Yellow
    };

    f.render_widget(dialog_block(title, border_color), dialog);

    if let Some(ref editor) = app.system_editor {
        let editor_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(4),
            height: dialog.height.saturating_sub(3),
        };

        if app.system_editor_read_only {
            let content = editor.lines().join("\n");
            let lines: Vec<Line> = content
                .lines()
                .map(|line| Line::from(Span::styled(line.to_owned(), Style::default().fg(Color::Red))))
                .collect();
            let paragraph = Paragraph::new(lines);
            f.render_widget(paragraph, editor_area);
        } else {
            f.render_widget(editor, editor_area);
        }

        let hint_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + dialog.height - 2,
            width: dialog.width.saturating_sub(4),
            height: 1,
        };
        let hint_text = if app.system_editor_read_only {
            "Esc: close"
        } else {
            "Esc: save & close"
        };
        let hint = Paragraph::new(Line::from(Span::styled(
            hint_text,
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, hint_area);
    }
}

pub(in crate::tui) fn handle_system_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if let Some(ref mut editor) = app.system_editor {
        if app.system_editor_read_only {
            if key.code == KeyCode::Esc {
                app.system_editor = None;
                app.system_editor_read_only = false;
                app.focus = app.system_editor_return_focus;
            }
            return None;
        }

        match key.code {
            KeyCode::Esc => {
                let content = editor.lines().join("\n");
                let value = if content.trim().is_empty() {
                    None
                } else {
                    Some(content.clone())
                };

                app.session.system_prompt = value;
                app.invalidate_chat_cache();
                app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);

                let prompt_name = &app.system_editor_prompt_name;
                if prompt_name.is_empty() {
                    app.system_editor = None;
                    app.focus = app.system_editor_return_focus;
                } else {
                    let dir = crate::config::system_prompts_dir();
                    let prompt = crate::system_prompt::SystemPromptFile {
                        name: prompt_name.clone(),
                        content,
                    };

                    match crate::system_prompt::save_prompt(
                        &prompt,
                        &dir,
                        app.save_mode.key(),
                    ) {
                        Ok(_) => {
                            app.set_status(
                                format!("System prompt '{}' saved.", prompt_name),
                                super::super::StatusLevel::Info,
                            );
                        }
                        Err(e) => {
                            app.set_status(
                                format!("Failed to save prompt: {e}"),
                                super::super::StatusLevel::Error,
                            );
                        }
                    }
                    app.system_editor = None;
                    app.focus = app.system_editor_return_focus;
                }
            }
            _ => {
                editor.input(key);
            }
        }
    }
    None
}
