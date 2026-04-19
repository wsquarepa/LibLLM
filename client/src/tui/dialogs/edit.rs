//! Inline message editor dialog for modifying existing chat messages.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let width = (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16;
    let height = (area.height as f32 * super::DIALOG_HEIGHT_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    f.render_widget(dialog_block(" Edit Message ", Color::Yellow), dialog);

    if let Some(ref editor) = app.edit_editor {
        let editor_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(4),
            height: dialog.height.saturating_sub(2),
        };
        f.render_widget(editor, editor_area);
    }

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from("Alt+Enter: save edit  Esc: cancel")],
    );
}

pub(in crate::tui) fn handle_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let editor = app.edit_editor.as_mut()?;

    let is_confirm = key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT);

    if key.code == KeyCode::Esc {
        let current_content = editor.lines().join("\n");
        if current_content == app.edit_original_content {
            app.edit_editor = None;
            app.raw_edit_node = None;
            app.focus = Focus::Chat;
        } else {
            app.edit_confirm_selected = 0;
            app.focus = Focus::EditConfirmDialog;
        }
        return None;
    }

    if is_confirm {
        let content = editor.lines().join("\n").trim().to_owned();
        let node_id = app.raw_edit_node.take();
        app.edit_editor = None;
        app.focus = Focus::Chat;

        if content.is_empty() {
            return None;
        }

        return node_id.map(|id| Action::EditMessage {
            node_id: id,
            content,
        });
    }

    let (consumed, warning) = crate::tui::clipboard::handle_clipboard_key(&key, editor);
    if !consumed {
        editor.input(key);
    }
    if let Some(msg) = warning {
        app.set_status(msg, crate::tui::StatusLevel::Warning);
    }
    None
}

pub(in crate::tui) fn render_edit_confirm_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, 6, area);

    let save_style = if app.edit_confirm_selected == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let discard_style = if app.edit_confirm_selected == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let lines = vec![
        Line::from(""),
        Line::from("  You have unsaved changes."),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Save & Exit ", save_style),
            Span::raw("   "),
            Span::styled(" Discard ", discard_style),
        ]),
        Line::from(""),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Unsaved Changes ", Color::Yellow));

    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Left/Right: navigate  Enter: confirm  Esc: back",
        )],
    );
}

pub(in crate::tui) fn handle_edit_confirm_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            app.edit_confirm_selected = 1 - app.edit_confirm_selected;
        }
        KeyCode::Enter => {
            if app.edit_confirm_selected == 0 {
                let Some(ref editor) = app.edit_editor else {
                    app.focus = Focus::Chat;
                    return None;
                };
                let content = editor.lines().join("\n").trim().to_owned();
                let node_id = app.raw_edit_node.take();
                app.edit_editor = None;
                app.focus = Focus::Chat;

                if content.is_empty() {
                    return None;
                }

                return node_id.map(|id| Action::EditMessage {
                    node_id: id,
                    content,
                });
            } else {
                app.edit_editor = None;
                app.raw_edit_node = None;
                app.focus = Focus::Chat;
            }
        }
        KeyCode::Esc => {
            app.focus = Focus::EditDialog;
        }
        _ => {}
    }
    None
}
