//! Typed-confirmation dialog for "Destroy All Data". User must type a randomly
//! generated 8-char [A-Z0-9] string to enable the Destroy button.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::theme::Theme;
use crate::tui::types::TypedConfirmState;

use super::{clear_centered, dialog_block};

const CONFIRM_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

pub fn generate_challenge() -> String {
    (0..8)
        .map(|_| {
            let idx = rand::random_range(0..CONFIRM_CHARS.len());
            CONFIRM_CHARS[idx] as char
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub enum DangerTypedResult {
    Pending,
    Cancel,
    Destroy,
}

#[expect(dead_code, reason = "wired in Task T27 (Destroy All Data)")]
pub(in crate::tui) fn render_danger_typed_confirm(
    f: &mut Frame,
    area: Rect,
    state: &TypedConfirmState,
    theme: &Theme,
) {
    let width = area.width.min(72);
    let height = 14;
    let popup = clear_centered(f, width, height, area);

    let title_span = Span::styled(
        "Confirm: DESTROY ALL DATA",
        Style::default()
            .fg(theme.status_error_fg)
            .add_modifier(Modifier::BOLD),
    );
    let block = dialog_block(title_span, theme.status_error_fg);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let matches = state.input == state.challenge;
    let cancel_style = if state.focus_idx == 1 {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
    };
    let destroy_style = if !matches {
        Style::default().fg(theme.dimmed)
    } else if state.focus_idx == 2 {
        Style::default()
            .fg(theme.status_error_fg)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default().fg(theme.status_error_fg)
    };

    let lines = vec![
        Line::from(""),
        Line::from("This will permanently delete the entire LibLLM data directory."),
        Line::from("A snapshot will be written to:"),
        Line::from(""),
        Line::from(format!("  {}", state.snapshot_path.display())),
        Line::from(""),
        Line::from("Type the confirmation string below to enable."),
        Line::from(""),
        Line::from(format!("  Confirmation: {}", state.challenge)),
        Line::from(format!("  Your input:   [{}]", state.input)),
        Line::from(""),
        Line::from(vec![
            Span::raw("                          "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("  "),
            Span::styled(" Destroy ", destroy_style),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Left), inner);
}

#[cfg_attr(not(test), expect(dead_code, reason = "wired in Task T27 (Destroy All Data)"))]
pub(in crate::tui) fn handle_danger_typed_key(
    key: KeyEvent,
    state: &mut TypedConfirmState,
) -> DangerTypedResult {
    let matches = state.input == state.challenge;
    match key.code {
        KeyCode::Esc => DangerTypedResult::Cancel,
        KeyCode::Tab => {
            state.focus_idx = match state.focus_idx {
                0 => 1,
                1 if matches => 2,
                1 => 0,
                _ => 0,
            };
            DangerTypedResult::Pending
        }
        KeyCode::BackTab => {
            state.focus_idx = match state.focus_idx {
                0 if matches => 2,
                0 => 1,
                1 => 0,
                _ => 1,
            };
            DangerTypedResult::Pending
        }
        KeyCode::Enter => match state.focus_idx {
            0 if matches => {
                state.focus_idx = 2;
                DangerTypedResult::Destroy
            }
            0 => DangerTypedResult::Pending,
            1 => DangerTypedResult::Cancel,
            2 if matches => DangerTypedResult::Destroy,
            _ => DangerTypedResult::Pending,
        },
        KeyCode::Backspace if state.focus_idx == 0 => {
            state.input.pop();
            DangerTypedResult::Pending
        }
        KeyCode::Char(c) if state.focus_idx == 0 => {
            if state.input.len() < 8 {
                state.input.push(c.to_ascii_uppercase());
                if state.input == state.challenge {
                    state.focus_idx = 2;
                }
            }
            DangerTypedResult::Pending
        }
        _ => DangerTypedResult::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::types::DangerOp;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn fixture(challenge: &str) -> TypedConfirmState {
        TypedConfirmState {
            challenge: challenge.to_owned(),
            input: String::new(),
            op: DangerOp::DestroyAll,
            snapshot_path: std::path::PathBuf::from("/tmp/test.tar.zst"),
            focus_idx: 1,
        }
    }

    #[test]
    fn challenge_is_8_chars_alphanumeric() {
        for _ in 0..32 {
            let c = generate_challenge();
            assert_eq!(c.len(), 8);
            assert!(c.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()));
        }
    }

    #[test]
    fn enter_on_destroy_disabled_when_input_empty() {
        let mut s = fixture("ABCDEFGH");
        s.focus_idx = 2;
        let r = handle_danger_typed_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerTypedResult::Pending));
    }

    #[test]
    fn typing_matching_input_auto_focuses_destroy() {
        let mut s = fixture("ABC12XYZ");
        s.focus_idx = 0;
        for c in "ABC12XYZ".chars() {
            handle_danger_typed_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), &mut s);
        }
        assert_eq!(s.input, "ABC12XYZ");
        assert_eq!(s.focus_idx, 2);
    }

    #[test]
    fn enter_when_matched_returns_destroy() {
        let mut s = fixture("AAAAAAAA");
        s.input = "AAAAAAAA".to_owned();
        s.focus_idx = 2;
        let r = handle_danger_typed_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerTypedResult::Destroy));
    }

    #[test]
    fn esc_returns_cancel() {
        let mut s = fixture("AAAAAAAA");
        let r = handle_danger_typed_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerTypedResult::Cancel));
    }

    #[test]
    fn input_uppercases_lowercase_typed_chars() {
        let mut s = fixture("ABCDEFGH");
        s.focus_idx = 0;
        handle_danger_typed_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &mut s);
        assert_eq!(s.input, "A");
    }
}
