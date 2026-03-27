use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::centered_rect;
use crate::tui::{Action, App, Focus};

fn render_edit_dialog_inner(f: &mut ratatui::Frame, app: &App, area: Rect, title: &str, hint: &str) {
    let width = (area.width as f32 * 0.7) as u16;
    let height = (area.height as f32 * 0.6) as u16;
    let dialog = centered_rect(width, height, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let border = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Yellow));
    f.render_widget(border, dialog);

    if let Some(ref editor) = app.edit_editor {
        let editor_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(4),
            height: dialog.height.saturating_sub(3),
        };
        f.render_widget(editor, editor_area);

        let hint_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + dialog.height - 2,
            width: dialog.width.saturating_sub(4),
            height: 1,
        };
        let hint_widget = Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint_widget, hint_area);
    }
}

pub(in crate::tui) fn render_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    render_edit_dialog_inner(
        f, app, area,
        " Edit Message (Esc to cancel, Enter to send) ",
        "Enter: send edited message  Alt+Enter: newline  Esc: cancel",
    );
}

pub(in crate::tui) fn render_raw_edit_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    render_edit_dialog_inner(
        f, app, area,
        " Edit Message (Esc to cancel, Alt+Enter to save) ",
        "Alt+Enter: save edit  Esc: cancel",
    );
}

enum EditConfirm {
    EnterWithoutAlt,
    AltEnter,
}

fn handle_edit_key_inner(
    key: KeyEvent,
    app: &mut App,
    confirm: EditConfirm,
    cancel_focus: Focus,
) -> Option<Action> {
    let Some(ref mut editor) = app.edit_editor else {
        return None;
    };

    let is_confirm = match confirm {
        EditConfirm::EnterWithoutAlt => {
            key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::ALT)
        }
        EditConfirm::AltEnter => {
            key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT)
        }
    };

    if key.code == KeyCode::Esc {
        app.edit_editor = None;
        app.raw_edit_node = None;
        app.focus = cancel_focus;
        app.status_message = "Edit cancelled.".to_owned();
        return None;
    }

    if is_confirm {
        let content = editor.lines().join("\n").trim().to_owned();
        let node_id = app.raw_edit_node.take();
        app.edit_editor = None;

        if content.is_empty() {
            app.focus = cancel_focus;
            app.status_message = "Edit cancelled (empty message).".to_owned();
            return None;
        }

        return match node_id {
            Some(id) => Some(Action::RawEditMessage { node_id: id, content }),
            None => {
                app.focus = cancel_focus;
                Some(Action::EditMessage(content))
            }
        };
    }

    editor.input(key);
    None
}

pub(in crate::tui) fn handle_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    handle_edit_key_inner(key, app, EditConfirm::EnterWithoutAlt, Focus::Input)
}

pub(in crate::tui) fn handle_raw_edit_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    handle_edit_key_inner(key, app, EditConfirm::AltEnter, Focus::Chat)
}
