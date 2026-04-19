//! Shared paging logic for list-selection dialogs.
//!
//! Exposes a pure `viewport` function, a `paged_list_height` sizing helper,
//! a `handle_paged_list_key` motion helper, and a `render_paged_list` composer.

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::tui::render::dialog_block;
use crate::tui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum PagedListAction {
    Consumed,
    Passthrough,
}

pub(in crate::tui) fn viewport(total: usize, selected: usize, visible: usize) -> Range<usize> {
    if total == 0 {
        return 0..0;
    }
    let clamped = selected.min(total - 1);
    if visible == 0 {
        return clamped..clamped + 1;
    }
    if total <= visible {
        return 0..total;
    }
    let start = clamped.saturating_sub(visible - 1);
    let start = start.min(total - visible);
    start..start + visible
}

pub(in crate::tui) fn paged_list_height(items: usize, terminal_height: u16, chrome: u16) -> u16 {
    let cap = (terminal_height as f32 * 0.7) as u16;
    let content_sized = (items as u16).saturating_add(chrome);
    let desired = cap.min(content_sized);

    let floor = chrome.saturating_add(3);
    if terminal_height >= floor {
        desired.max(floor).min(terminal_height)
    } else {
        terminal_height
    }
}

pub(in crate::tui) fn page_size(terminal_height: u16, chrome: u16) -> usize {
    terminal_height
        .saturating_sub(chrome)
        .saturating_sub(3)
        .max(1) as usize
}

pub(in crate::tui) fn handle_paged_list_key(
    selected: &mut usize,
    total: usize,
    visible: usize,
    key: KeyEvent,
) -> PagedListAction {
    let last = total.saturating_sub(1);
    match key.code {
        KeyCode::Up => {
            *selected = selected.saturating_sub(1);
            PagedListAction::Consumed
        }
        KeyCode::Down => {
            *selected = (*selected + 1).min(last);
            PagedListAction::Consumed
        }
        KeyCode::PageUp => {
            *selected = selected.saturating_sub(visible.max(1));
            PagedListAction::Consumed
        }
        KeyCode::PageDown => {
            *selected = (*selected + visible.max(1)).min(last);
            PagedListAction::Consumed
        }
        KeyCode::Home => {
            *selected = 0;
            PagedListAction::Consumed
        }
        KeyCode::End => {
            *selected = last;
            PagedListAction::Consumed
        }
        _ => PagedListAction::Passthrough,
    }
}

pub(in crate::tui) fn render_paged_list(
    f: &mut Frame,
    area: Rect,
    selected: usize,
    items: Vec<ListItem<'_>>,
    title_base: &str,
    theme: &Theme,
) {
    let total = items.len();
    let visible = visible_rows(area);
    let range = viewport(total, selected, visible);

    let title = format_title(title_base, total, selected, visible);
    let block = dialog_block(Line::from(title), theme.border_focused).padding(Padding::horizontal(1));

    let clamped_selected = selected.min(total.saturating_sub(1));
    let relative_selected = clamped_selected.saturating_sub(range.start);
    let visible_items: Vec<ListItem<'_>> = items
        .into_iter()
        .skip(range.start)
        .take(range.end - range.start)
        .collect();

    let list = List::new(visible_items).block(block).highlight_style(
        Style::default()
            .fg(theme.sidebar_highlight_fg)
            .bg(theme.sidebar_highlight_bg)
            .add_modifier(Modifier::BOLD),
    );

    let mut list_state = ListState::default();
    if total > 0 {
        list_state.select(Some(relative_selected));
    }
    f.render_stateful_widget(list, area, &mut list_state);

    if total > visible && visible > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        let mut scrollbar_state =
            ScrollbarState::new(total.saturating_sub(visible)).position(range.start);
        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin { horizontal: 0, vertical: 1 }),
            &mut scrollbar_state,
        );
    }
}

pub(in crate::tui) fn render_paged_list_inline(
    f: &mut Frame,
    area: Rect,
    selected: usize,
    items: Vec<ListItem<'_>>,
    theme: &Theme,
) {
    let total = items.len();
    let visible = area.height as usize;
    let range = viewport(total, selected, visible);

    let clamped_selected = selected.min(total.saturating_sub(1));
    let relative_selected = clamped_selected.saturating_sub(range.start);
    let visible_items: Vec<ListItem<'_>> = items
        .into_iter()
        .skip(range.start)
        .take(range.end - range.start)
        .collect();

    let list = List::new(visible_items).highlight_style(
        Style::default()
            .fg(theme.sidebar_highlight_fg)
            .bg(theme.sidebar_highlight_bg)
            .add_modifier(Modifier::BOLD),
    );

    let mut list_state = ListState::default();
    if total > 0 && selected != usize::MAX {
        list_state.select(Some(relative_selected));
    }
    let list_area = area.inner(Margin { horizontal: 1, vertical: 0 });
    f.render_stateful_widget(list, list_area, &mut list_state);

    if total > visible && visible > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        let mut scrollbar_state =
            ScrollbarState::new(total.saturating_sub(visible)).position(range.start);
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn visible_rows(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

fn format_title(base: &str, total: usize, selected: usize, visible: usize) -> String {
    if total <= visible {
        return base.to_owned();
    }
    let trimmed = base.trim_end();
    let display_position = selected.min(total.saturating_sub(1)) + 1;
    format!("{trimmed} [ {display_position} of {total} ] ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_empty_list_returns_empty_range() {
        assert_eq!(viewport(0, 0, 10), 0..0);
    }

    #[test]
    fn viewport_short_list_shows_everything() {
        assert_eq!(viewport(3, 0, 10), 0..3);
        assert_eq!(viewport(10, 5, 10), 0..10);
    }

    #[test]
    fn viewport_equal_list_shows_everything() {
        assert_eq!(viewport(10, 0, 10), 0..10);
        assert_eq!(viewport(10, 9, 10), 0..10);
    }

    #[test]
    fn viewport_selection_at_top_shows_top_slice() {
        assert_eq!(viewport(20, 0, 5), 0..5);
        assert_eq!(viewport(20, 4, 5), 0..5);
    }

    #[test]
    fn viewport_selection_past_bottom_scrolls_down() {
        assert_eq!(viewport(20, 5, 5), 1..6);
        assert_eq!(viewport(20, 10, 5), 6..11);
    }

    #[test]
    fn viewport_selection_at_end_clamps_window() {
        assert_eq!(viewport(20, 19, 5), 15..20);
    }

    #[test]
    fn viewport_selection_out_of_bounds_clamps() {
        assert_eq!(viewport(10, 99, 5), 5..10);
    }

    #[test]
    fn viewport_visible_zero_returns_selected_only() {
        assert_eq!(viewport(10, 3, 0), 3..4);
    }

    #[test]
    fn viewport_single_item() {
        assert_eq!(viewport(1, 0, 5), 0..1);
    }

    #[test]
    fn height_content_sized_for_short_list() {
        // 3 items + chrome 4 = 7 rows. 70% of 100-row terminal = 70.
        // content fits well under cap -> content-sized.
        assert_eq!(paged_list_height(3, 100, 4), 7);
    }

    #[test]
    fn height_caps_at_seventy_percent() {
        // 200 items would need 204 rows with chrome 4.
        // 70% of 100 = 70.
        assert_eq!(paged_list_height(200, 100, 4), 70);
    }

    #[test]
    fn height_respects_minimum_floor_when_possible() {
        // terminal_height = 10, chrome = 4 -> chrome + 3 = 7 floor.
        // 0 items would compute 4 (content-sized), but floor lifts it to 7.
        assert_eq!(paged_list_height(0, 10, 4), 7);
    }

    #[test]
    fn height_skips_floor_when_terminal_too_small() {
        // terminal_height = 5, chrome = 4 -> floor would be 7 but terminal only has 5.
        // Fall back to whatever fits: terminal_height itself.
        assert_eq!(paged_list_height(100, 5, 4), 5);
    }

    #[test]
    fn height_uses_branch_chrome() {
        // Branch picker uses chrome = 3.
        // 10 items + 3 = 13 rows, fits under 70% of 50 = 35.
        assert_eq!(paged_list_height(10, 50, 3), 13);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn key_up_decrements_and_clamps_at_zero() {
        let mut sel = 3usize;
        assert_eq!(handle_paged_list_key(&mut sel, 10, 5, key(KeyCode::Up)), PagedListAction::Consumed);
        assert_eq!(sel, 2);

        sel = 0;
        assert_eq!(handle_paged_list_key(&mut sel, 10, 5, key(KeyCode::Up)), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_down_increments_and_clamps_at_last() {
        let mut sel = 3usize;
        assert_eq!(handle_paged_list_key(&mut sel, 10, 5, key(KeyCode::Down)), PagedListAction::Consumed);
        assert_eq!(sel, 4);

        sel = 9;
        assert_eq!(handle_paged_list_key(&mut sel, 10, 5, key(KeyCode::Down)), PagedListAction::Consumed);
        assert_eq!(sel, 9);
    }

    #[test]
    fn key_page_down_jumps_by_visible_and_clamps() {
        let mut sel = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::PageDown)), PagedListAction::Consumed);
        assert_eq!(sel, 5);

        sel = 18;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::PageDown)), PagedListAction::Consumed);
        assert_eq!(sel, 19);
    }

    #[test]
    fn key_page_up_jumps_by_visible_and_clamps_at_zero() {
        let mut sel = 15usize;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::PageUp)), PagedListAction::Consumed);
        assert_eq!(sel, 10);

        sel = 2;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::PageUp)), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_home_jumps_to_zero() {
        let mut sel = 7usize;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::Home)), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_end_jumps_to_last() {
        let mut sel = 2usize;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::End)), PagedListAction::Consumed);
        assert_eq!(sel, 19);

        let mut sel_empty = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel_empty, 0, 5, key(KeyCode::End)), PagedListAction::Consumed);
        assert_eq!(sel_empty, 0);
    }

    #[test]
    fn unrelated_keys_pass_through_without_mutation() {
        let mut sel = 5usize;
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::Enter)), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::Esc)), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
        assert_eq!(handle_paged_list_key(&mut sel, 20, 5, key(KeyCode::Char('a'))), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
    }

    #[test]
    fn motion_on_empty_list_is_idempotent() {
        let mut sel = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel, 0, 5, key(KeyCode::Down)), PagedListAction::Consumed);
        assert_eq!(sel, 0);
        assert_eq!(handle_paged_list_key(&mut sel, 0, 5, key(KeyCode::PageDown)), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn title_omits_counter_when_list_fits() {
        assert_eq!(format_title(" Personas ", 5, 0, 10), " Personas ");
        assert_eq!(format_title(" Personas ", 10, 0, 10), " Personas ");
    }

    #[test]
    fn title_injects_counter_when_list_overflows() {
        assert_eq!(format_title(" Personas ", 42, 2, 10), " Personas [ 3 of 42 ] ");
        assert_eq!(format_title(" Personas ", 42, 41, 10), " Personas [ 42 of 42 ] ");
    }

    #[test]
    fn title_counter_clamps_when_selected_out_of_bounds() {
        assert_eq!(format_title(" Personas ", 42, 99, 10), " Personas [ 42 of 42 ] ");
    }

    #[test]
    fn page_size_normal_terminal() {
        assert_eq!(page_size(100, 4), 93);
    }

    #[test]
    fn page_size_floors_at_one_for_tiny_terminal() {
        assert_eq!(page_size(5, 4), 1);
        assert_eq!(page_size(0, 4), 1);
    }

    #[test]
    fn page_size_branch_chrome() {
        assert_eq!(page_size(50, 3), 44);
    }
}
