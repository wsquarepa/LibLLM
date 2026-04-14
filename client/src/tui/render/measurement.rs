use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use libllm::session::NodeId;

use super::ChatContentCache;

pub(super) fn measure_wrapped_offset(lines: &[Line], up_to: usize, area: Rect) -> u16 {
    if up_to == 0 {
        return 0;
    }
    measure_wrapped_height(&lines[..up_to], area)
}

pub(super) fn measure_wrapped_height(lines: &[Line], area: Rect) -> u16 {
    lines
        .iter()
        .map(|line| wrapped_line_height(line, area))
        .sum()
}

// -----------------------------------------------------------------------
// This MUST use Paragraph::line_count() -- not a manual width calculation.
// Ratatui's WordWrapper breaks at word boundaries and measures unicode
// display width, both of which differ from ceil(byte_len / columns).
// A naive approximation underestimates height, and the error accumulates
// across messages, causing auto-scroll to miss the actual bottom of text.
// -----------------------------------------------------------------------
pub(super) fn wrapped_line_height(line: &Line, area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2).max(1);
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner_width) as u16
}

pub fn hit_test_chat_message(
    cache: &ChatContentCache,
    branch_ids: &[NodeId],
    chat_area: Rect,
    chat_scroll: u16,
    screen_row: u16,
) -> Option<NodeId> {
    let inner_top = chat_area.y + 1;
    let inner_bottom = chat_area.y + chat_area.height.saturating_sub(1);
    if screen_row < inner_top || screen_row >= inner_bottom {
        return None;
    }
    let content_row = (screen_row - inner_top) as u32 + chat_scroll as u32;
    let mut cumulative: u32 = 0;
    for (i, entry) in cache.entries.iter().enumerate() {
        cumulative += entry.total_height as u32;
        if content_row < cumulative {
            return branch_ids.get(i).copied();
        }
    }
    None
}
