//! Pure, allocation-light helpers for paged list viewport calculation and sizing.
//! No ratatui widget or I/O concerns.

use std::ops::Range;

/// Computes the visible index range for a paged list given total items, the
/// current selected index, and the number of visible slots.
pub(crate) fn viewport(total: usize, selected: usize, visible: usize) -> Range<usize> {
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
    let center_offset = visible / 2;
    let start = clamped.saturating_sub(center_offset);
    let start = start.min(total - visible);
    start..start + visible
}

/// Computes a reasonable dialog height for a list of `items`, capped at 70% of
/// terminal height with some chrome (title + borders + hints) reserved.
pub(crate) fn paged_list_height(items: usize, terminal_height: u16, chrome: u16) -> u16 {
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

/// Returns how many list items can fit given terminal height and chrome.
pub(crate) fn page_size(terminal_height: u16, chrome: u16) -> usize {
    terminal_height
        .saturating_sub(chrome)
        .saturating_sub(3)
        .max(1) as usize
}
