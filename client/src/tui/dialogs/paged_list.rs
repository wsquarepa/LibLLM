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
}
