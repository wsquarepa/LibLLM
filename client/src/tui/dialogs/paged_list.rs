//! Shared paging logic for list-selection dialogs.
//!
//! Exposes a pure `viewport` function, a `paged_list_height` sizing helper,
//! a `handle_paged_list_key` motion helper, and a `render_paged_list` composer.

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent};
use regex::{Regex, RegexBuilder};
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::tui::render::dialog_block;
use crate::tui::theme::Theme;

pub(in crate::tui) struct SearchState {
    pub query: String,
    pub active: bool,
    compiled: Option<Result<Regex, regex::Error>>,
    pre_search_selected: Option<usize>,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            compiled: None,
            pre_search_selected: None,
        }
    }

    pub fn enter(&mut self, current_selected: usize) {
        self.active = true;
        self.pre_search_selected = Some(current_selected);
    }

    pub fn commit(&mut self) {
        self.active = false;
        self.pre_search_selected = None;
    }

    pub fn cancel(&mut self) -> Option<usize> {
        self.active = false;
        self.query.clear();
        self.compiled = None;
        self.pre_search_selected.take()
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.recompile();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.recompile();
    }

    pub fn is_filtering(&self) -> bool {
        !self.query.is_empty()
    }

    pub fn matches(&self, label: &str) -> bool {
        match &self.compiled {
            None => true,
            Some(Ok(re)) => re.is_match(label),
            Some(Err(_)) => false,
        }
    }

    pub fn compile_error(&self) -> bool {
        matches!(&self.compiled, Some(Err(_)))
    }

    fn recompile(&mut self) {
        if self.query.is_empty() {
            self.compiled = None;
            return;
        }
        self.compiled = Some(
            RegexBuilder::new(&self.query)
                .case_insensitive(true)
                .build(),
        );
    }

    #[cfg(test)]
    pub fn pre_search_selected(&self) -> Option<usize> {
        self.pre_search_selected
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

pub(in crate::tui) fn filter_indices(labels: &[String], state: &SearchState) -> Vec<usize> {
    if !state.is_filtering() {
        return (0..labels.len()).collect();
    }
    labels
        .iter()
        .enumerate()
        .filter_map(|(i, label)| if state.matches(label) { Some(i) } else { None })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum PagedListAction {
    Consumed,
    Passthrough,
    EnteredSearch,
    ExitedSearch,
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
    let center_offset = visible / 2;
    let start = clamped.saturating_sub(center_offset);
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
    labels: &[String],
    visible: usize,
    key: KeyEvent,
    search: Option<&mut SearchState>,
) -> PagedListAction {
    use crossterm::event::KeyModifiers;

    if let Some(state) = search {
        let is_ctrl_f = key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL);
        if is_ctrl_f {
            if state.active {
                return PagedListAction::Consumed;
            }
            state.enter(*selected);
            return PagedListAction::EnteredSearch;
        }

        if state.active {
            match key.code {
                KeyCode::Esc => {
                    if let Some(restored) = state.cancel() {
                        *selected = restored.min(labels.len().saturating_sub(1));
                    }
                    return PagedListAction::ExitedSearch;
                }
                KeyCode::Enter => {
                    state.commit();
                    return PagedListAction::ExitedSearch;
                }
                KeyCode::Backspace => {
                    state.pop_char();
                    return PagedListAction::Consumed;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.push_char(c);
                    return PagedListAction::Consumed;
                }
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End => {
                    let filtered = filter_indices(labels, state);
                    navigate_filtered(selected, &filtered, visible, key);
                    return PagedListAction::Consumed;
                }
                _ => return PagedListAction::Consumed,
            }
        }

        if state.is_filtering()
            && matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            let filtered = filter_indices(labels, state);
            navigate_filtered(selected, &filtered, visible, key);
            return PagedListAction::Consumed;
        }
    }

    let total = labels.len();
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

fn navigate_filtered(selected: &mut usize, filtered: &[usize], visible: usize, key: KeyEvent) {
    if filtered.is_empty() {
        return;
    }
    let current_pos = filtered.iter().position(|&i| i == *selected).unwrap_or(0);
    let last = filtered.len() - 1;
    let new_pos = match key.code {
        KeyCode::Up => current_pos.saturating_sub(1),
        KeyCode::Down => (current_pos + 1).min(last),
        KeyCode::PageUp => current_pos.saturating_sub(visible.max(1)),
        KeyCode::PageDown => (current_pos + visible.max(1)).min(last),
        KeyCode::Home => 0,
        KeyCode::End => last,
        _ => current_pos,
    };
    *selected = filtered[new_pos];
}

pub(in crate::tui) struct PagedListContent<'a> {
    pub selected: usize,
    pub items: Vec<ListItem<'a>>,
    pub title_base: &'a str,
    pub search: Option<&'a SearchState>,
    pub unfiltered_total: Option<usize>,
}

pub(in crate::tui) fn render_paged_list(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    content: PagedListContent<'_>,
) {
    let PagedListContent { selected, items, title_base, search, unfiltered_total } = content;
    let total = items.len();
    let list_area = area;

    let visible = visible_rows(list_area);
    let range = viewport(total, selected, visible);

    let title = format_title(title_base, total, selected, visible, unfiltered_total);
    let mut block = dialog_block(Line::from(title), theme.border_focused).padding(Padding::horizontal(1));
    if let Some(state) = search {
        let max = list_area.width.saturating_sub(2);
        block = block.title_bottom(search_title_line(state, theme.border_focused, theme, max));
    }

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
    f.render_stateful_widget(list, list_area, &mut list_state);

    if total > visible && visible > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        let mut scrollbar_state =
            ScrollbarState::new(total.saturating_sub(visible)).position(range.start);
        f.render_stateful_widget(
            scrollbar,
            list_area.inner(Margin { horizontal: 0, vertical: 1 }),
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
    let list_area = area;

    let visible = list_area.height as usize;
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
    let inner_list_area = list_area.inner(Margin { horizontal: 1, vertical: 0 });
    f.render_stateful_widget(list, inner_list_area, &mut list_state);

    if total > visible && visible > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        let mut scrollbar_state =
            ScrollbarState::new(total.saturating_sub(visible)).position(range.start);
        f.render_stateful_widget(scrollbar, list_area, &mut scrollbar_state);
    }
}

pub(in crate::tui) fn list_dialog_rect(
    terminal_area: Rect,
    visible_count: usize,
    chrome: u16,
    dialog_width: u16,
) -> Rect {
    let height = paged_list_height(visible_count, terminal_area.height, chrome);
    crate::tui::render::centered_rect(dialog_width, height, terminal_area)
}

pub(in crate::tui) fn map_list_click(
    items_area: Rect,
    visible_indices: &[usize],
    current_orig_selected: usize,
    screen_row: u16,
) -> Option<usize> {
    let inner = screen_row.checked_sub(items_area.y)?;
    if inner >= items_area.height {
        return None;
    }
    let current_pos = visible_indices
        .iter()
        .position(|&i| i == current_orig_selected)
        .unwrap_or(0);
    let range = viewport(visible_indices.len(), current_pos, items_area.height as usize);
    visible_indices.get(range.start + inner as usize).copied()
}

const SEARCH_PREFIX_ACTIVE: &str = " Search: ";
const SEARCH_SUFFIX_ACTIVE: &str = "_ ";
const SEARCH_PREFIX_COMMITTED: &str = " Search: ";
const SEARCH_SUFFIX_COMMITTED: &str = " ";
const SEARCH_LABEL_IDLE: &str = " Search ";

fn tail_fit(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_owned();
    }
    s.chars().skip(total - max_chars).collect()
}

pub(in crate::tui) fn search_title_line(
    state: &SearchState,
    title_color: Color,
    theme: &Theme,
    max_width: u16,
) -> Line<'static> {
    let (body, bold, fg) = search_body(state, title_color, theme, max_width);
    let mut style = Style::default().fg(fg);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(Span::styled(body, style))
}

pub(in crate::tui) fn search_title_width(state: &SearchState, max_width: u16) -> u16 {
    if state.active {
        let fixed = (SEARCH_PREFIX_ACTIVE.len() + SEARCH_SUFFIX_ACTIVE.len()) as u16;
        let budget = max_width.saturating_sub(fixed) as usize;
        let query_chars = state.query.chars().count().min(budget);
        fixed + query_chars as u16
    } else if state.is_filtering() {
        let fixed = (SEARCH_PREFIX_COMMITTED.len() + SEARCH_SUFFIX_COMMITTED.len()) as u16;
        let budget = max_width.saturating_sub(fixed) as usize;
        let query_chars = state.query.chars().count().min(budget);
        fixed + query_chars as u16
    } else {
        SEARCH_LABEL_IDLE.len() as u16
    }
}

fn search_body(
    state: &SearchState,
    title_color: Color,
    theme: &Theme,
    max_width: u16,
) -> (String, bool, Color) {
    if state.active {
        let fg = if state.compile_error() { theme.status_error_fg } else { title_color };
        let fixed = SEARCH_PREFIX_ACTIVE.len() + SEARCH_SUFFIX_ACTIVE.len();
        let budget = (max_width as usize).saturating_sub(fixed);
        let visible_query = tail_fit(&state.query, budget);
        (
            format!("{SEARCH_PREFIX_ACTIVE}{visible_query}{SEARCH_SUFFIX_ACTIVE}"),
            true,
            fg,
        )
    } else if state.is_filtering() {
        let fg = if state.compile_error() { theme.status_error_fg } else { title_color };
        let fixed = SEARCH_PREFIX_COMMITTED.len() + SEARCH_SUFFIX_COMMITTED.len();
        let budget = (max_width as usize).saturating_sub(fixed);
        let visible_query = tail_fit(&state.query, budget);
        (
            format!("{SEARCH_PREFIX_COMMITTED}{visible_query}{SEARCH_SUFFIX_COMMITTED}"),
            false,
            fg,
        )
    } else {
        (SEARCH_LABEL_IDLE.to_owned(), false, theme.dimmed)
    }
}

fn visible_rows(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

fn format_title(
    base: &str,
    total: usize,
    selected: usize,
    visible: usize,
    unfiltered_total: Option<usize>,
) -> String {
    let trimmed = base.trim_end();
    let is_filtered = unfiltered_total.is_some_and(|u| u > total);

    if is_filtered {
        let unfiltered = unfiltered_total.unwrap();
        let display_position = if total == 0 { 0 } else { selected.min(total - 1) + 1 };
        return format!("{trimmed} [ {display_position} of {total} filtered / {unfiltered} ] ");
    }

    if total <= visible {
        return base.to_owned();
    }
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
    fn viewport_selection_near_top_stays_at_top() {
        // For visible=5, center_offset=2. Selections at 0..=2 keep the window at 0..5.
        assert_eq!(viewport(20, 0, 5), 0..5);
        assert_eq!(viewport(20, 1, 5), 0..5);
        assert_eq!(viewport(20, 2, 5), 0..5);
    }

    #[test]
    fn viewport_selection_in_middle_is_centered() {
        // Selected sits at relative index center_offset within the visible window.
        assert_eq!(viewport(20, 5, 5), 3..8);
        assert_eq!(viewport(20, 10, 5), 8..13);
    }

    #[test]
    fn viewport_selection_near_bottom_stays_at_bottom() {
        // Window clamps to the last `visible` rows as selection approaches the end.
        assert_eq!(viewport(20, 19, 5), 15..20);
        assert_eq!(viewport(20, 18, 5), 15..20);
        assert_eq!(viewport(20, 17, 5), 15..20);
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
        assert_eq!(paged_list_height(3, 100, 4), 7);
    }

    #[test]
    fn height_caps_at_seventy_percent() {
        assert_eq!(paged_list_height(200, 100, 4), 70);
    }

    #[test]
    fn height_respects_minimum_floor_when_possible() {
        assert_eq!(paged_list_height(0, 10, 4), 7);
    }

    #[test]
    fn height_skips_floor_when_terminal_too_small() {
        assert_eq!(paged_list_height(100, 5, 4), 5);
    }

    #[test]
    fn height_uses_branch_chrome() {
        assert_eq!(paged_list_height(10, 50, 3), 13);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn key_up_decrements_and_clamps_at_zero() {
        let labels = vec!["x".to_owned(); 10];
        let mut sel = 3usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Up), None), PagedListAction::Consumed);
        assert_eq!(sel, 2);

        sel = 0;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Up), None), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_down_increments_and_clamps_at_last() {
        let labels = vec!["x".to_owned(); 10];
        let mut sel = 3usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Down), None), PagedListAction::Consumed);
        assert_eq!(sel, 4);

        sel = 9;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Down), None), PagedListAction::Consumed);
        assert_eq!(sel, 9);
    }

    #[test]
    fn key_page_down_jumps_by_visible_and_clamps() {
        let labels = vec!["x".to_owned(); 20];
        let mut sel = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::PageDown), None), PagedListAction::Consumed);
        assert_eq!(sel, 5);

        sel = 18;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::PageDown), None), PagedListAction::Consumed);
        assert_eq!(sel, 19);
    }

    #[test]
    fn key_page_up_jumps_by_visible_and_clamps_at_zero() {
        let labels = vec!["x".to_owned(); 20];
        let mut sel = 15usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::PageUp), None), PagedListAction::Consumed);
        assert_eq!(sel, 10);

        sel = 2;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::PageUp), None), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_home_jumps_to_zero() {
        let labels = vec!["x".to_owned(); 20];
        let mut sel = 7usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Home), None), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    #[test]
    fn key_end_jumps_to_last() {
        let labels = vec!["x".to_owned(); 20];
        let mut sel = 2usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::End), None), PagedListAction::Consumed);
        assert_eq!(sel, 19);

        let labels_empty: Vec<String> = Vec::new();
        let mut sel_empty = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel_empty, &labels_empty, 5, key(KeyCode::End), None), PagedListAction::Consumed);
        assert_eq!(sel_empty, 0);
    }

    #[test]
    fn unrelated_keys_pass_through_without_mutation() {
        let labels = vec!["x".to_owned(); 20];
        let mut sel = 5usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Enter), None), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Esc), None), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Char('a')), None), PagedListAction::Passthrough);
        assert_eq!(sel, 5);
    }

    #[test]
    fn motion_on_empty_list_is_idempotent() {
        let labels: Vec<String> = Vec::new();
        let mut sel = 0usize;
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Down), None), PagedListAction::Consumed);
        assert_eq!(sel, 0);
        assert_eq!(handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::PageDown), None), PagedListAction::Consumed);
        assert_eq!(sel, 0);
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::CONTROL)
    }

    #[test]
    fn search_ctrl_f_activates_and_snapshots() {
        let mut sel = 4usize;
        let labels = vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned(), "e".to_owned()];
        let mut state = SearchState::new();
        let action = handle_paged_list_key(&mut sel, &labels, 5, ctrl_key('f'), Some(&mut state));
        assert_eq!(action, PagedListAction::EnteredSearch);
        assert!(state.active);
        assert_eq!(state.pre_search_selected(), Some(4));
    }

    #[test]
    fn search_printable_char_appends_to_query() {
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned(), "beta".to_owned()];
        let mut state = SearchState::new();
        state.enter(0);
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Char('a')), Some(&mut state));
        assert_eq!(action, PagedListAction::Consumed);
        assert_eq!(state.query, "a");
    }

    #[test]
    fn search_backspace_pops_query() {
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned()];
        let mut state = SearchState::new();
        state.enter(0);
        state.push_char('x');
        state.push_char('y');
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Backspace), Some(&mut state));
        assert_eq!(action, PagedListAction::Consumed);
        assert_eq!(state.query, "x");
    }

    #[test]
    fn search_navigation_keys_move_within_filtered_slice() {
        // Labels: alpha, beta, gamma, alfalfa.
        // Filter "al" -> matches indices [0, 3].
        // selected starts at 0 (alpha). Down should advance to filtered[1] = original index 3.
        let mut sel = 0usize;
        let labels = vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned(),
            "alfalfa".to_owned(),
        ];
        let mut state = SearchState::new();
        state.enter(0);
        state.push_char('a');
        state.push_char('l');
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Down), Some(&mut state));
        assert_eq!(action, PagedListAction::Consumed);
        assert_eq!(sel, 3);
    }

    #[test]
    fn search_navigation_clamps_at_filtered_end() {
        // Same fixture as above. Selected on alfalfa (idx 3). Down should stay on idx 3.
        let mut sel = 3usize;
        let labels = vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned(),
            "alfalfa".to_owned(),
        ];
        let mut state = SearchState::new();
        state.enter(3);
        state.push_char('a');
        state.push_char('l');
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Down), Some(&mut state));
        assert_eq!(action, PagedListAction::Consumed);
        assert_eq!(sel, 3);
    }

    #[test]
    fn search_enter_commits_and_keeps_query() {
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned()];
        let mut state = SearchState::new();
        state.enter(0);
        state.push_char('a');
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Enter), Some(&mut state));
        assert_eq!(action, PagedListAction::ExitedSearch);
        assert!(!state.active);
        assert_eq!(state.query, "a");
    }

    #[test]
    fn search_esc_cancels_and_restores_selection() {
        // sel starts at 0 to simulate the user having navigated within the filtered view.
        // The snapshot captured by enter(2) is what Esc must restore.
        let mut sel = 0usize;
        let labels = vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned(),
        ];
        let mut state = SearchState::new();
        state.enter(2);
        state.push_char('a');
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Esc), Some(&mut state));
        assert_eq!(action, PagedListAction::ExitedSearch);
        assert!(!state.active);
        assert!(state.query.is_empty());
        assert_eq!(sel, 2);
    }

    #[test]
    fn search_passthrough_when_state_inactive_and_no_ctrlf() {
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned()];
        let mut state = SearchState::new();
        let action = handle_paged_list_key(&mut sel, &labels, 5, key(KeyCode::Char('a')), Some(&mut state));
        assert_eq!(action, PagedListAction::Passthrough);
    }

    #[test]
    fn search_ctrl_f_when_active_stays_active() {
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned()];
        let mut state = SearchState::new();
        state.enter(0);
        let action = handle_paged_list_key(&mut sel, &labels, 5, ctrl_key('f'), Some(&mut state));
        assert_eq!(action, PagedListAction::Consumed);
        assert!(state.active);
    }

    #[test]
    fn search_none_argument_keeps_existing_behavior() {
        // When dialogs pass None, Ctrl+F is passthrough (caller may bind it elsewhere).
        let mut sel = 0usize;
        let labels = vec!["alpha".to_owned(), "beta".to_owned()];
        let action = handle_paged_list_key(&mut sel, &labels, 5, ctrl_key('f'), None);
        assert_eq!(action, PagedListAction::Passthrough);
    }

    #[test]
    fn title_omits_counter_when_list_fits() {
        assert_eq!(format_title(" Personas ", 5, 0, 10, None), " Personas ");
        assert_eq!(format_title(" Personas ", 10, 0, 10, None), " Personas ");
    }

    #[test]
    fn title_injects_counter_when_list_overflows() {
        assert_eq!(format_title(" Personas ", 42, 2, 10, None), " Personas [ 3 of 42 ] ");
        assert_eq!(format_title(" Personas ", 42, 41, 10, None), " Personas [ 42 of 42 ] ");
    }

    #[test]
    fn title_counter_clamps_when_selected_out_of_bounds() {
        assert_eq!(format_title(" Personas ", 42, 99, 10, None), " Personas [ 42 of 42 ] ");
    }

    #[test]
    fn title_shows_filtered_counter_when_subset_matches() {
        assert_eq!(
            format_title(" Personas ", 12, 0, 10, Some(87)),
            " Personas [ 1 of 12 filtered / 87 ] "
        );
    }

    #[test]
    fn title_shows_filtered_counter_when_zero_matches() {
        assert_eq!(
            format_title(" Personas ", 0, 0, 10, Some(87)),
            " Personas [ 0 of 0 filtered / 87 ] "
        );
    }

    #[test]
    fn title_omits_filtered_when_all_match() {
        assert_eq!(
            format_title(" Personas ", 87, 0, 10, Some(87)),
            " Personas [ 1 of 87 ] "
        );
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

    #[test]
    fn search_state_new_is_inactive_and_empty() {
        let s = SearchState::new();
        assert!(!s.active);
        assert!(s.query.is_empty());
        assert!(!s.is_filtering());
    }

    #[test]
    fn search_state_enter_activates_and_snapshots_selection() {
        let mut s = SearchState::new();
        s.enter(7);
        assert!(s.active);
        assert_eq!(s.pre_search_selected(), Some(7));
    }

    #[test]
    fn search_state_commit_deactivates_and_keeps_query() {
        let mut s = SearchState::new();
        s.enter(0);
        s.push_char('a');
        s.commit();
        assert!(!s.active);
        assert_eq!(s.query, "a");
        assert!(s.is_filtering());
    }

    #[test]
    fn search_state_cancel_clears_query_and_returns_snapshot() {
        let mut s = SearchState::new();
        s.enter(3);
        s.push_char('a');
        s.push_char('b');
        let restored = s.cancel();
        assert_eq!(restored, Some(3));
        assert!(!s.active);
        assert!(s.query.is_empty());
        assert!(!s.is_filtering());
    }

    #[test]
    fn search_state_cancel_without_enter_returns_none() {
        let mut s = SearchState::new();
        assert_eq!(s.cancel(), None);
    }

    #[test]
    fn search_state_push_pop_char_edits_query() {
        let mut s = SearchState::new();
        s.push_char('a');
        s.push_char('b');
        s.push_char('c');
        assert_eq!(s.query, "abc");
        s.pop_char();
        assert_eq!(s.query, "ab");
        s.pop_char();
        s.pop_char();
        s.pop_char();
        assert_eq!(s.query, "");
    }

    #[test]
    fn search_state_is_filtering_independent_of_active() {
        let mut s = SearchState::new();
        s.push_char('x');
        assert!(s.is_filtering());
        assert!(!s.active);
        s.enter(0);
        assert!(s.is_filtering());
        assert!(s.active);
    }

    #[test]
    fn search_state_matches_is_case_insensitive() {
        let mut s = SearchState::new();
        s.push_char('a');
        s.push_char('l');
        s.push_char('i');
        assert!(s.matches("Alice"));
        assert!(s.matches("alibi"));
        assert!(s.matches("SALIM"));
        assert!(!s.matches("Bob"));
    }

    #[test]
    fn search_state_matches_returns_false_when_regex_invalid() {
        let mut s = SearchState::new();
        s.push_char('(');
        assert!(s.compile_error());
        assert!(!s.matches("anything"));
        assert!(!s.matches(""));
    }

    #[test]
    fn search_state_matches_anything_with_empty_query() {
        let s = SearchState::new();
        assert!(s.matches("Alice"));
        assert!(s.matches(""));
        assert!(!s.compile_error());
    }

    #[test]
    fn search_state_recompiles_on_pop_back_to_valid() {
        let mut s = SearchState::new();
        s.push_char('(');
        assert!(s.compile_error());
        s.pop_char();
        assert!(!s.compile_error());
        assert!(s.matches("anything"));
    }

    #[test]
    fn search_state_cancel_drops_compile_error() {
        let mut s = SearchState::new();
        s.push_char('[');
        assert!(s.compile_error());
        s.cancel();
        assert!(!s.compile_error());
    }

    #[test]
    fn filter_indices_inactive_returns_all() {
        let labels = vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()];
        let s = SearchState::new();
        assert_eq!(filter_indices(&labels, &s), vec![0, 1, 2]);
    }

    #[test]
    fn filter_indices_filters_matches_in_order() {
        let labels = vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned(),
            "alfalfa".to_owned(),
        ];
        let mut s = SearchState::new();
        s.push_char('a');
        s.push_char('l');
        // "al" matches "alpha" (idx 0) and "alfalfa" (idx 3).
        assert_eq!(filter_indices(&labels, &s), vec![0, 3]);
    }

    #[test]
    fn filter_indices_invalid_regex_returns_empty() {
        let labels = vec!["alpha".to_owned(), "beta".to_owned()];
        let mut s = SearchState::new();
        s.push_char('(');
        assert_eq!(filter_indices(&labels, &s), Vec::<usize>::new());
    }

    #[test]
    fn filter_indices_empty_label_list() {
        let labels: Vec<String> = Vec::new();
        let s = SearchState::new();
        assert_eq!(filter_indices(&labels, &s), Vec::<usize>::new());
    }
}
