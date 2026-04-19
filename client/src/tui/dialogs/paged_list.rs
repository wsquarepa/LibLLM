//! Shared paging logic for list-selection dialogs.
//!
//! Exposes a pure `viewport` function, a `paged_list_height` sizing helper,
//! a `handle_paged_list_key` motion helper, and a `render_paged_list` composer.

use std::ops::Range;

#[cfg_attr(not(test), expect(dead_code, reason = "callers in this module are added in follow-up commits; remove this attribute when the first caller lands"))]
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

#[cfg_attr(not(test), expect(dead_code, reason = "callers in this module are added in follow-up commits; remove this attribute when the first caller lands"))]
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
}
