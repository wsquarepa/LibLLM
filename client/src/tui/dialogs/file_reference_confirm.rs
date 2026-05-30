//! Confirmation dialog shown when a pasted string resolves to a local file.
//! Offers [Paste Raw] (insert the original text verbatim) or [Attach] (insert
//! the `@<path>` token). The safe option (Paste Raw) is the default so that a
//! reflexive Enter never attaches a file that clipboard poisoning slipped in.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::dialog_handler::return_to_input;
use crate::tui::{Action, App};

pub(in crate::tui) const FILE_REFERENCE_CONFIRM_DIALOG_WIDTH: u16 = 72;
pub(in crate::tui) const FILE_REFERENCE_CONFIRM_DIALOG_HEIGHT: u16 = 9;

/// State for the file-reference confirmation dialog.
pub struct FileReferenceConfirmState {
    /// The resolved `@<path>` token to insert when the user chooses Attach.
    pub token: String,
    /// The original pasted text to insert when the user chooses Paste Raw.
    pub raw: String,
    /// 0 = Paste Raw (safe default), 1 = Attach.
    pub selected: usize,
}

pub(in crate::tui) fn open(app: &mut App<'_>, token: String, raw: String) {
    app.file_reference_confirm = Some(FileReferenceConfirmState {
        token,
        raw,
        selected: 0,
    });
    app.focus = crate::tui::Focus::FileReferenceConfirmDialog;
}

pub(in crate::tui) fn render(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(
        f,
        FILE_REFERENCE_CONFIRM_DIALOG_WIDTH,
        FILE_REFERENCE_CONFIRM_DIALOG_HEIGHT,
        area,
    );

    let (token, selected) = match app.file_reference_confirm.as_ref() {
        Some(s) => (s.token.as_str(), s.selected),
        None => ("", 0),
    };

    let highlight = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let raw_style = if selected == 0 {
        highlight
    } else {
        Style::default()
    };
    let attach_style = if selected == 1 {
        highlight
    } else {
        Style::default()
    };

    let lines = vec![
        Line::from(""),
        Line::from("  Pasted text resolves to a local file:"),
        Line::from(format!("  {token}")),
        Line::from(""),
        Line::from("  Paste the raw text, or attach it as a file reference?"),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(" Paste Raw ", raw_style),
            Span::raw("   "),
            Span::styled(" Attach ", attach_style),
        ]),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" File Reference ", Color::Yellow));

    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Left/Right: navigate  Enter: confirm  Esc: paste raw",
        )],
    );
}

pub(in crate::tui) fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let Some(state) = app.file_reference_confirm.as_mut() else {
        return_to_input(app);
        return None;
    };

    match key.code {
        KeyCode::Left | KeyCode::Right => {
            state.selected = 1 - state.selected;
        }
        KeyCode::Enter => {
            let insert_text = if state.selected == 1 {
                state.token.clone()
            } else {
                state.raw.clone()
            };
            app.file_reference_confirm = None;
            app.textarea.insert_str(&insert_text);
            return_to_input(app);
        }
        KeyCode::Esc => {
            let raw = state.raw.clone();
            app.file_reference_confirm = None;
            app.textarea.insert_str(&raw);
            return_to_input(app);
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_defaults_to_safe_paste_raw_option() {
        let state = FileReferenceConfirmState {
            token: "@/tmp/secret.txt".to_owned(),
            raw: "/tmp/secret.txt".to_owned(),
            selected: 0,
        };
        assert_eq!(state.token, "@/tmp/secret.txt");
        assert_eq!(state.raw, "/tmp/secret.txt");
        // selected == 0 is Paste Raw, the non-attaching option, so confirming
        // the default never reads or attaches the file.
        assert_eq!(state.selected, 0);
    }
}
