//! Shared unsaved-changes warning dialog.
//!
//! Pushed on top of any editor dialog when the user presses Esc while the
//! dialog is dirty. Offers Save & Close / Discard / Cancel as the only place
//! these three outcomes are surfaced uniformly across editors.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::tui::theme::Theme;
use crate::tui::Focus;

use super::{clear_centered, dialog_block, render_hints_below_dialog};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum UnsavedButton {
    SaveAndClose,
    Discard,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum UnsavedOutcome {
    Pending,
    Chosen(UnsavedButton),
}

pub(in crate::tui) struct UnsavedWarningState {
    pub(in crate::tui) focused: UnsavedButton,
    /// Focus to dispatch on (Save & Close / Discard) or restore on Cancel.
    /// Set by the caller that pushes the warning.
    pub(in crate::tui) return_focus: Focus,
}

impl UnsavedWarningState {
    #[expect(dead_code, reason = "called by editors wired in Tasks 3.2-3.5; no callers exist yet")]
    pub(in crate::tui) fn new(return_focus: Focus) -> Self {
        Self {
            focused: UnsavedButton::SaveAndClose,
            return_focus,
        }
    }
}

pub(in crate::tui) fn handle_key(
    state: &mut UnsavedWarningState,
    key: KeyEvent,
) -> UnsavedOutcome {
    match key.code {
        KeyCode::Left => {
            state.focused = match state.focused {
                UnsavedButton::SaveAndClose => UnsavedButton::Cancel,
                UnsavedButton::Discard => UnsavedButton::SaveAndClose,
                UnsavedButton::Cancel => UnsavedButton::Discard,
            };
            UnsavedOutcome::Pending
        }
        KeyCode::Right => {
            state.focused = match state.focused {
                UnsavedButton::SaveAndClose => UnsavedButton::Discard,
                UnsavedButton::Discard => UnsavedButton::Cancel,
                UnsavedButton::Cancel => UnsavedButton::SaveAndClose,
            };
            UnsavedOutcome::Pending
        }
        KeyCode::Enter => UnsavedOutcome::Chosen(state.focused),
        KeyCode::Esc => UnsavedOutcome::Chosen(UnsavedButton::Cancel),
        _ => UnsavedOutcome::Pending,
    }
}

pub(in crate::tui) fn render(
    f: &mut Frame,
    area: Rect,
    state: &UnsavedWarningState,
    theme: &Theme,
) {
    let width: u16 = 56;
    let height: u16 = 6;
    let dialog = clear_centered(f, width, height, area);

    let mut button_spans: Vec<Span<'_>> = vec![Span::raw("    ")];
    for (i, (label, button)) in [
        (" Save & Close ", UnsavedButton::SaveAndClose),
        (" Discard ",      UnsavedButton::Discard),
        (" Cancel ",       UnsavedButton::Cancel),
    ].iter().enumerate() {
        if i > 0 {
            button_spans.push(Span::raw("  "));
        }
        let style = if state.focused == *button {
            let bg = match button {
                UnsavedButton::Discard => theme.status_error_bg,
                _ => theme.border_focused,
            };
            Style::default()
                .fg(Color::Black)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            match button {
                UnsavedButton::Discard => Style::default().fg(theme.status_error_bg),
                _ => Style::default(),
            }
        };
        button_spans.push(Span::styled(*label, style));
    }

    let lines = vec![
        Line::from(""),
        Line::from("  You have unsaved changes."),
        Line::from(""),
        Line::from(button_spans),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(Text::from(lines))
        .block(dialog_block(" Unsaved Changes ", theme.border_focused));
    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Left/Right: navigate  Enter: confirm  Esc: cancel",
        )],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn fixture() -> UnsavedWarningState {
        UnsavedWarningState {
            focused: UnsavedButton::SaveAndClose,
            return_focus: Focus::EditDialog,
        }
    }

    #[test]
    fn left_right_cycle_through_three_buttons() {
        let mut state = fixture();
        assert_eq!(state.focused, UnsavedButton::SaveAndClose);
        handle_key(&mut state, k(KeyCode::Right));
        assert_eq!(state.focused, UnsavedButton::Discard);
        handle_key(&mut state, k(KeyCode::Right));
        assert_eq!(state.focused, UnsavedButton::Cancel);
        handle_key(&mut state, k(KeyCode::Right));
        assert_eq!(state.focused, UnsavedButton::SaveAndClose);
    }

    #[test]
    fn left_wraps_backward() {
        let mut state = fixture();
        handle_key(&mut state, k(KeyCode::Left));
        assert_eq!(state.focused, UnsavedButton::Cancel);
    }

    #[test]
    fn enter_chooses_focused() {
        let mut state = fixture();
        state.focused = UnsavedButton::Discard;
        assert_eq!(
            handle_key(&mut state, k(KeyCode::Enter)),
            UnsavedOutcome::Chosen(UnsavedButton::Discard),
        );
    }

    #[test]
    fn esc_always_chooses_cancel() {
        let mut state = fixture();
        state.focused = UnsavedButton::Discard;
        assert_eq!(
            handle_key(&mut state, k(KeyCode::Esc)),
            UnsavedOutcome::Chosen(UnsavedButton::Cancel),
        );
    }
}
