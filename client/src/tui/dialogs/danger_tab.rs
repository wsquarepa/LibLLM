//! Danger tab body renderer for the /config dialog.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::tui::theme::Theme;
use crate::tui::types::App;

/// Index of the Danger tab within the /config TabbedFieldDialog sections.
pub const DANGER_TAB_INDEX: usize = 5;

const DANGER_ACTIONS: &[&str] = &["Destroy All Data"];

/// Render the body of the Danger tab inside the /config dialog.
///
/// `area` is the body rect inside the dialog border, below the tab bar.
/// Highlights the item at `app.danger_selected` in bold error color.
/// This function owns the ConfigTab::Danger variant — it identifies which
/// /config tab this module corresponds to.
pub(in crate::tui) fn render_danger_tab_body(f: &mut ratatui::Frame, area: Rect, app: &App, theme: &Theme) {
    let items: Vec<Line> = DANGER_ACTIONS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.danger_selected {
                Style::default()
                    .fg(theme.status_error_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_error_fg)
            };
            Line::from(vec![Span::styled(format!("  {label}"), style)])
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(items)), area);
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
