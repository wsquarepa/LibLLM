//! Typed-confirmation dialog for "Destroy All Data". User must type a randomly
//! generated 8-char [A-Z0-9] string to enable the Destroy button.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::types::TypedConfirmState;

use super::{byte_pos_at_char, clear_centered, dialog_block, render_hints_below_dialog};

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

pub(in crate::tui) fn render_danger_typed_confirm(
    f: &mut Frame,
    area: Rect,
    state: &TypedConfirmState,
) {
    let width = area.width.min(76);
    let height = 14;
    let popup = clear_centered(f, width, height, area);

    let title_span = Span::styled(
        "Confirm: DESTROY ALL DATA",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    );
    let block = dialog_block(title_span, Color::Red);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let matches = state.input == state.challenge;
    let body_style = Style::default().fg(Color::Red);
    let cancel_style = if state.focus_idx == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let destroy_style = if !matches {
        Style::default().fg(Color::DarkGray)
    } else if state.focus_idx == 2 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let lines = vec![
        Line::from(""),
        Line::styled(
            "  This will permanently delete the entire LibLLM data directory.",
            body_style,
        ),
        Line::styled("  A snapshot will be written to:", body_style),
        Line::from(""),
        Line::styled(
            format!("    {}", state.snapshot_path.display()),
            body_style,
        ),
        Line::from(""),
        Line::styled(
            "  Type the confirmation string below to enable.",
            body_style,
        ),
        Line::from(""),
        Line::styled(format!("    Confirmation: {}", state.challenge), body_style),
        build_input_line(state, body_style),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(" Destroy ", destroy_style),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).alignment(Alignment::Left), inner);

    let hint = if state.focus_idx == 0 {
        "Type the code  Tab: move focus  Esc: cancel"
    } else {
        "Tab/Shift-Tab: move focus  Enter: activate  Esc: cancel"
    };
    render_hints_below_dialog(f, popup, area, &[Line::from(hint)]);
}

fn build_input_line(state: &TypedConfirmState, body_style: Style) -> Line<'static> {
    let label_style = if state.focus_idx == 0 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        body_style
    };
    let value_style = if state.focus_idx == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        body_style
    };

    let chars: Vec<char> = state.input.chars().collect();
    let cursor = state.cursor_pos.min(chars.len());

    let mut spans: Vec<Span<'static>> = vec![
        Span::raw("    "),
        Span::styled("Your input:   ", label_style),
        Span::styled("[", value_style),
    ];

    if state.focus_idx == 0 {
        let before: String = chars[..cursor].iter().collect();
        let cursor_ch = if cursor < chars.len() {
            chars[cursor].to_string()
        } else {
            " ".to_string()
        };
        let after_start = (cursor + 1).min(chars.len());
        let after: String = chars[after_start..].iter().collect();
        spans.push(Span::styled(before, value_style));
        spans.push(Span::styled(
            cursor_ch,
            value_style.add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(after, value_style));
    } else {
        spans.push(Span::styled(state.input.clone(), value_style));
    }

    spans.push(Span::styled("]", value_style));
    Line::from(spans)
}

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
            1 => DangerTypedResult::Cancel,
            2 if matches => DangerTypedResult::Destroy,
            _ => DangerTypedResult::Pending,
        },
        KeyCode::Backspace if state.focus_idx == 0 && state.cursor_pos > 0 => {
            state.cursor_pos -= 1;
            let byte_pos = byte_pos_at_char(&state.input, state.cursor_pos);
            state.input.remove(byte_pos);
            DangerTypedResult::Pending
        }
        KeyCode::Delete if state.focus_idx == 0 => {
            let char_count = state.input.chars().count();
            if state.cursor_pos < char_count {
                let byte_pos = byte_pos_at_char(&state.input, state.cursor_pos);
                state.input.remove(byte_pos);
            }
            DangerTypedResult::Pending
        }
        KeyCode::Left if state.focus_idx == 0 && state.cursor_pos > 0 => {
            state.cursor_pos -= 1;
            DangerTypedResult::Pending
        }
        KeyCode::Right if state.focus_idx == 0 => {
            let char_count = state.input.chars().count();
            if state.cursor_pos < char_count {
                state.cursor_pos += 1;
            }
            DangerTypedResult::Pending
        }
        KeyCode::Home if state.focus_idx == 0 => {
            state.cursor_pos = 0;
            DangerTypedResult::Pending
        }
        KeyCode::End if state.focus_idx == 0 => {
            state.cursor_pos = state.input.chars().count();
            DangerTypedResult::Pending
        }
        KeyCode::Char(c) if state.focus_idx == 0 => {
            let byte_pos = byte_pos_at_char(&state.input, state.cursor_pos);
            state.input.insert(byte_pos, c);
            state.cursor_pos += 1;
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
            cursor_pos: 0,
            op: DangerOp::DestroyAll,
            snapshot_path: std::path::PathBuf::from("/tmp/test.tar.zst"),
            focus_idx: 0,
        }
    }

    fn type_string(state: &mut TypedConfirmState, s: &str) {
        for c in s.chars() {
            handle_danger_typed_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), state);
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
    fn typing_records_chars_at_cursor() {
        let mut s = fixture("ABC12XYZ");
        type_string(&mut s, "ABC12XYZ");
        assert_eq!(s.input, "ABC12XYZ");
        assert_eq!(s.cursor_pos, 8);
    }

    #[test]
    fn enter_when_matched_returns_destroy() {
        let mut s = fixture("AAAAAAAA");
        s.input = "AAAAAAAA".to_owned();
        s.cursor_pos = 8;
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
    fn typed_chars_preserve_case() {
        let mut s = fixture("ABCDEFGH");
        handle_danger_typed_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &mut s);
        assert_eq!(s.input, "a");
    }

    #[test]
    fn left_arrow_moves_cursor_back() {
        let mut s = fixture("X");
        type_string(&mut s, "AB");
        assert_eq!(s.cursor_pos, 2);
        handle_danger_typed_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut s);
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn backspace_removes_char_before_cursor() {
        let mut s = fixture("X");
        type_string(&mut s, "ABC");
        handle_danger_typed_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut s);
        handle_danger_typed_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), &mut s);
        assert_eq!(s.input, "AC");
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn tab_advances_focus_when_matched() {
        let mut s = fixture("AB");
        type_string(&mut s, "AB");
        assert_eq!(s.focus_idx, 0);
        handle_danger_typed_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut s);
        assert_eq!(s.focus_idx, 1);
        handle_danger_typed_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut s);
        assert_eq!(s.focus_idx, 2);
    }
}
