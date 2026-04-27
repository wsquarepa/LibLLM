//! Confirmation dialog for Danger tab items 1-6 (synchronous destructive ops).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::theme::Theme;
use crate::tui::types::DangerOp;

use super::{clear_centered, dialog_block};

#[derive(Debug, Clone, Copy)]
pub enum DangerConfirmResult {
    Pending,
    Cancel,
    Confirm,
}

#[expect(dead_code, reason = "wired in Task T26 (danger dispatch handler)")]
pub(in crate::tui) fn render_danger_confirm(
    f: &mut Frame,
    area: Rect,
    op: DangerOp,
    selected: usize,
    theme: &Theme,
) {
    let (title, body, confirm_label) = match op {
        DangerOp::ClearStores => (
            "Clear Stores",
            "This will clear all dismissed-template prompts.",
            "Clear",
        ),
        DangerOp::RegeneratePresets => (
            "Regenerate Presets",
            "This will overwrite the bundled built-in presets.",
            "Regenerate",
        ),
        DangerOp::PurgeChats => (
            "Purge Chats",
            "This will delete ALL chats from the database.",
            "Purge",
        ),
        DangerOp::PurgeCharacters => (
            "Purge Characters",
            "This will delete ALL characters from the database.",
            "Purge",
        ),
        DangerOp::PurgePersonas => (
            "Purge Personas",
            "This will delete ALL personas from the database.",
            "Purge",
        ),
        DangerOp::PurgeWorldbooks => (
            "Purge Worldbooks",
            "This will delete ALL worldbooks from the database.",
            "Purge",
        ),
        DangerOp::DestroyAll => unreachable!("DestroyAll uses typed-confirm dialog"),
    };

    let width = area.width.min(64);
    let height = 8;
    let popup = clear_centered(f, width, height, area);

    let title_span = Span::styled(
        format!("Confirm: {title}"),
        Style::default()
            .fg(theme.status_error_fg)
            .add_modifier(Modifier::BOLD),
    );
    let block = dialog_block(title_span, theme.status_error_fg);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let cancel_style = if selected == 0 {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
    };
    let confirm_style = if selected == 1 {
        Style::default()
            .fg(theme.status_error_fg)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default().fg(theme.status_error_fg)
    };

    let lines = vec![
        Line::from(""),
        Line::from(body.to_owned()),
        Line::from("This action cannot be undone."),
        Line::from(""),
        Line::from(vec![
            Span::raw("                          "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("  "),
            Span::styled(format!(" {confirm_label} "), confirm_style),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Left),
        inner,
    );
}

#[cfg_attr(not(test), expect(dead_code, reason = "wired in Task T26 (danger dispatch handler)"))]
pub(in crate::tui) fn handle_danger_confirm_key(
    key: KeyEvent,
    selected: &mut usize,
) -> DangerConfirmResult {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            *selected = 1 - *selected;
            DangerConfirmResult::Pending
        }
        KeyCode::Enter => {
            if *selected == 1 {
                DangerConfirmResult::Confirm
            } else {
                DangerConfirmResult::Cancel
            }
        }
        KeyCode::Esc => DangerConfirmResult::Cancel,
        _ => DangerConfirmResult::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn arrow_toggles_selection() {
        let mut s = 0;
        let _ = handle_danger_confirm_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut s);
        assert_eq!(s, 1);
        let _ = handle_danger_confirm_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut s);
        assert_eq!(s, 0);
    }

    #[test]
    fn enter_on_cancel_returns_cancel() {
        let mut s = 0;
        let r = handle_danger_confirm_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerConfirmResult::Cancel));
    }

    #[test]
    fn enter_on_confirm_returns_confirm() {
        let mut s = 1;
        let r = handle_danger_confirm_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerConfirmResult::Confirm));
    }
}
