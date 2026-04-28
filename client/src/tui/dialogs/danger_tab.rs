//! Renders the Danger tab body inside the /config dialog and handles its key events.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::theme::Theme;
use crate::tui::types::{App, DangerOp};

/// Index of the Danger tab within the /config TabbedFieldDialog sections.
pub const DANGER_TAB_INDEX: usize = 5;

const ITEMS: &[(DangerOp, &str)] = &[
    (DangerOp::ClearStores, "1. Clear Stores"),
    (DangerOp::RegeneratePresets, "2. Regenerate Presets"),
    (DangerOp::PurgeChats, "3. Purge Chats"),
    (DangerOp::PurgeCharacters, "4. Purge Characters"),
    (DangerOp::PurgePersonas, "5. Purge Personas"),
    (DangerOp::PurgeWorldbooks, "6. Purge Worldbooks"),
    (DangerOp::DestroyAll, "7. Destroy All Data"),
];

/// Render the body of the Danger tab inside the /config dialog.
///
/// `area` is the body rect inside the dialog border, below the tab bar.
/// Highlights the item at `app.danger_selected` with a reversed style.
/// `DestroyAll` is always rendered in the error foreground color to signal
/// its destructive nature regardless of selection state.
pub(in crate::tui) fn render_danger_tab_body(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line<'static>> = vec![
        Line::from("  Destructive actions. Each requires confirmation."),
        Line::from(""),
    ];
    for (idx, (op, label)) in ITEMS.iter().enumerate() {
        let prefix = if idx == app.danger_selected {
            "    > "
        } else {
            "      "
        };
        let is_destroy_all = matches!(op, DangerOp::DestroyAll);
        let style = if idx == app.danger_selected {
            if is_destroy_all {
                Style::default()
                    .fg(theme.status_error_fg)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            }
        } else if is_destroy_all {
            Style::default().fg(theme.status_error_fg)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::raw(prefix),
            Span::styled((*label).to_owned(), style),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

#[derive(Debug, Clone, Copy)]
pub(in crate::tui) enum DangerTabResult {
    Pending,
    OpenConfirm(DangerOp),
    Passthrough,
}

pub(in crate::tui) fn handle_danger_tab_key(
    key: KeyEvent,
    selected: &mut usize,
) -> DangerTabResult {
    match key.code {
        KeyCode::Up => {
            *selected = if *selected == 0 {
                ITEMS.len() - 1
            } else {
                *selected - 1
            };
            DangerTabResult::Pending
        }
        KeyCode::Down => {
            *selected = (*selected + 1) % ITEMS.len();
            DangerTabResult::Pending
        }
        KeyCode::Enter => {
            let op = ITEMS[*selected].0;
            DangerTabResult::OpenConfirm(op)
        }
        _ => DangerTabResult::Passthrough,
    }
}

/// Body rect for a TabbedFieldDialog rendered at `dialog_outer`.
///
/// Mirrors the internal layout of TabbedFieldDialog::render_tabs_and_fields:
/// 1 border row + 1 blank + 1 tab bar + 1 blank = 4 rows before fields;
/// 1 border row at the bottom.
pub fn tab_body_rect(dialog_outer: Rect) -> Rect {
    Rect {
        x: dialog_outer.x + 1,
        y: dialog_outer.y + 4,
        width: dialog_outer.width.saturating_sub(2),
        height: dialog_outer.height.saturating_sub(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn down_increments_with_wrap() {
        let mut s = 6;
        let _ = handle_danger_tab_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut s);
        assert_eq!(s, 0);
    }

    #[test]
    fn up_decrements_with_wrap() {
        let mut s = 0;
        let _ = handle_danger_tab_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut s);
        assert_eq!(s, 6);
    }

    #[test]
    fn enter_returns_op_for_selection() {
        let mut s = 0;
        let r = handle_danger_tab_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerTabResult::OpenConfirm(DangerOp::ClearStores)));
        s = 6;
        let r = handle_danger_tab_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut s);
        assert!(matches!(r, DangerTabResult::OpenConfirm(DangerOp::DestroyAll)));
    }
}
