use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use libllm::db::Database;
use libllm::search::{self, strip_terminal_controls, SearchHit};
use libllm::search::query as search_query;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use time::format_description::well_known::Rfc3339;

use super::super::render::{clear_centered, render_hints_below_dialog};
use super::super::theme::Theme;
use super::super::types::Focus;

const SEARCH_DIALOG_WIDTH_PERCENT: f32 = 0.8;
const SEARCH_DIALOG_HEIGHT_PERCENT: f32 = 0.8;

pub(crate) const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
pub(crate) const MIN_QUERY_CHARS: usize = 3;
pub(crate) const PAGE_SIZE: usize = 10;
const PREVIEW_LINE_CAP: usize = 2000;

pub(crate) struct SearchDialogState {
    pub input: String,
    pub cursor: usize,
    pub last_keystroke: Option<Instant>,
    pub last_compiled: Option<String>,
    pub hits: Vec<SearchHit>,
    pub selected: usize,
    pub error: Option<String>,
}

impl SearchDialogState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            last_keystroke: None,
            last_compiled: None,
            hits: Vec::new(),
            selected: 0,
            error: None,
        }
    }

    pub fn ready_for_query(&self, now: Instant) -> bool {
        let Some(stamp) = self.last_keystroke else {
            return false;
        };
        if now.duration_since(stamp) < DEBOUNCE {
            return false;
        }
        let same_as_last = self
            .last_compiled
            .as_deref()
            .is_some_and(|s| s == self.input);
        !same_as_last
    }
}

pub(crate) const TUI_LIMIT: usize = libllm::search::DEFAULT_MAX_HITS;

pub(crate) fn maybe_run_query(state: &mut SearchDialogState, db: &Database, now: Instant) {
    if !state.ready_for_query(now) {
        return;
    }
    let trimmed = state.input.trim();
    if trimmed.chars().count() < MIN_QUERY_CHARS {
        state.hits.clear();
        state.error = None;
        state.last_compiled = Some(state.input.clone());
        state.last_keystroke = None;
        state.selected = 0;
        return;
    }

    match search_query::compile(&state.input, db) {
        Ok(compiled) => match search::search(db, &compiled, TUI_LIMIT) {
            Ok(hits) => {
                state.hits = hits;
                state.error = None;
                if state.selected >= state.hits.len() {
                    state.selected = state.hits.len().saturating_sub(1);
                }
            }
            Err(err) => {
                state.error = Some(err.to_string());
            }
        },
        Err(err) => {
            state.error = Some(err.to_string());
        }
    }
    state.last_compiled = Some(state.input.clone());
    state.last_keystroke = None;
}

pub(crate) enum SearchDialogOutcome {
    Consumed,
    Close,
    Submit,
}

pub(crate) fn handle_key(state: &mut SearchDialogState, key: KeyEvent) -> SearchDialogOutcome {
    match key.code {
        KeyCode::Esc => SearchDialogOutcome::Close,
        KeyCode::Enter => SearchDialogOutcome::Submit,
        KeyCode::Up => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            SearchDialogOutcome::Consumed
        }
        KeyCode::Down => {
            if !state.hits.is_empty() && state.selected + 1 < state.hits.len() {
                state.selected += 1;
            }
            SearchDialogOutcome::Consumed
        }
        KeyCode::PageDown => {
            if !state.hits.is_empty() {
                let target = state.selected.saturating_add(PAGE_SIZE);
                state.selected = target.min(state.hits.len() - 1);
            }
            SearchDialogOutcome::Consumed
        }
        KeyCode::PageUp => {
            state.selected = state.selected.saturating_sub(PAGE_SIZE);
            SearchDialogOutcome::Consumed
        }
        KeyCode::Backspace => {
            if state.cursor > 0 {
                let mut chars: Vec<char> = state.input.chars().collect();
                chars.remove(state.cursor - 1);
                state.input = chars.into_iter().collect();
                state.cursor -= 1;
                state.last_keystroke = Some(Instant::now());
            }
            SearchDialogOutcome::Consumed
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(rest) = state.input.strip_prefix("m:") {
                state.input = rest.to_owned();
                state.cursor = state.input.chars().count();
            } else {
                state.input.insert_str(0, "m:");
                state.cursor += 2;
            }
            state.last_keystroke = Some(Instant::now());
            SearchDialogOutcome::Consumed
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return SearchDialogOutcome::Consumed;
            }
            let mut chars: Vec<char> = state.input.chars().collect();
            chars.insert(state.cursor, c);
            state.input = chars.into_iter().collect();
            state.cursor += 1;
            state.last_keystroke = Some(Instant::now());
            SearchDialogOutcome::Consumed
        }
        _ => SearchDialogOutcome::Consumed,
    }
}

pub(crate) fn render_dialog(
    state: &SearchDialogState,
    f: &mut ratatui::Frame,
    area: Rect,
    theme: &Theme,
) {
    let w = (area.width as f32 * SEARCH_DIALOG_WIDTH_PERCENT) as u16;
    let h = (area.height as f32 * SEARCH_DIALOG_HEIGHT_PERCENT) as u16;
    let dialog = clear_centered(f, w, h, area);
    render(state, dialog, f.buffer_mut(), theme);
    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Up/Down: select  Enter: jump  Ctrl+R: raw FTS5  Esc: close",
        )],
    );
}

pub(crate) fn render(state: &SearchDialogState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let block = Block::default()
        .title(" Search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused));
    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .split(inner);

    render_input(state, chunks[0], buf);
    render_hits(state, chunks[1], buf, theme);
    render_separator(chunks[2], buf, theme);
    render_preview(state, chunks[3], buf, theme);
}

fn render_input(state: &SearchDialogState, area: Rect, buf: &mut Buffer) {
    let prompt = format!("> {}_", state.input);
    Paragraph::new(prompt).render(area, buf);
}

fn render_hits(state: &SearchDialogState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if state.hits.is_empty() {
        let empty = if let Some(err) = &state.error {
            err.clone()
        } else if state.input.trim().chars().count() < MIN_QUERY_CHARS {
            "type at least 3 characters".to_owned()
        } else {
            "no matches".to_owned()
        };
        Paragraph::new(empty)
            .style(Style::default().fg(theme.status_bar_fg))
            .render(area, buf);
        return;
    }

    let session_col_width = (area.width / 4).clamp(8, 20) as usize;
    for (i, hit) in state.hits.iter().enumerate().take(area.height as usize) {
        let y = area.y + i as u16;
        let prefix = if i == state.selected { "> " } else { "  " };
        let session = trunc(&hit.session_display_name, session_col_width);
        let role = trunc(&hit.role.to_string(), 6);
        let snippet_spans = highlight_spans(&collapse_newlines(&hit.snippet), theme);
        let mut line = vec![
            Span::raw(prefix.to_owned()),
            Span::styled(
                format!("{session:<width$}", width = session_col_width),
                Style::default().fg(theme.status_bar_fg),
            ),
            Span::raw("  ".to_owned()),
            Span::styled(format!("{role:<6}"), Style::default().fg(theme.status_bar_fg)),
            Span::raw("  ".to_owned()),
        ];
        line.extend(snippet_spans);
        Paragraph::new(Line::from(line)).render(
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }
}

fn render_separator(area: Rect, buf: &mut Buffer, theme: &Theme) {
    let bar = "─".repeat(area.width as usize);
    Paragraph::new(bar)
        .style(Style::default().fg(theme.status_bar_fg))
        .render(area, buf);
}

fn render_preview(state: &SearchDialogState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(hit) = state.hits.get(state.selected) else {
        Paragraph::new("(no preview)")
            .style(Style::default().fg(theme.status_bar_fg))
            .render(area, buf);
        return;
    };

    let safe_name = strip_terminal_controls(&hit.session_display_name);
    let header = format!(
        "{}  {}  {}",
        safe_name,
        hit.role,
        hit.timestamp.format(&Rfc3339).expect("RFC 3339 format")
    );
    Paragraph::new(header)
        .style(Style::default().fg(theme.status_bar_fg))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );

    let body_area = Rect {
        x: area.x,
        y: area.y.saturating_add(2),
        width: area.width,
        height: area.height.saturating_sub(2),
    };
    if body_area.height == 0 {
        return;
    }

    let total = hit.preview_text.lines().take(PREVIEW_LINE_CAP + 1).count();
    let visible_rows = body_area.height as usize;

    let truncated = total > visible_rows || total > PREVIEW_LINE_CAP;
    let body_rows = if truncated {
        visible_rows.saturating_sub(1).min(PREVIEW_LINE_CAP)
    } else {
        visible_rows
    };

    let body_lines: Vec<Line<'static>> = hit
        .preview_text
        .lines()
        .take(body_rows)
        .map(|line_text| Line::from(highlight_spans(line_text, theme)))
        .collect();
    Paragraph::new(ratatui::text::Text::from(body_lines)).render(
        Rect {
            x: body_area.x,
            y: body_area.y,
            width: body_area.width,
            height: body_rows as u16,
        },
        buf,
    );

    if truncated {
        let remaining = total.saturating_sub(body_rows);
        let noun = if remaining == 1 { "line" } else { "lines" };
        let footer = format!("... +{remaining} more {noun}");
        Paragraph::new(footer)
            .style(Style::default().fg(theme.status_bar_fg))
            .render(
                Rect {
                    x: body_area.x,
                    y: body_area.y + body_rows as u16,
                    width: body_area.width,
                    height: 1,
                },
                buf,
            );
    }
}

fn highlight_spans(input: &str, theme: &Theme) -> Vec<Span<'static>> {
    const OPEN: char = '\u{1}';
    const CLOSE: char = '\u{2}';
    let sanitized = strip_terminal_controls(input);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buffer = String::new();
    let mut highlighted = false;
    for c in sanitized.chars() {
        match c {
            OPEN => {
                if !buffer.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut buffer)));
                }
                highlighted = true;
            }
            CLOSE => {
                if !buffer.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut buffer),
                        Style::default()
                            .fg(theme.search_highlight_fg)
                            .bg(theme.search_highlight_bg)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                highlighted = false;
            }
            other => buffer.push(other),
        }
    }
    if !buffer.is_empty() {
        let style = if highlighted {
            Style::default()
                .fg(theme.search_highlight_fg)
                .bg(theme.search_highlight_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(buffer, style));
    }
    spans
}

fn collapse_newlines(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub(in crate::tui) fn close(focus: &mut Focus, slot: &mut Option<SearchDialogState>) {
    *slot = None;
    *focus = Focus::Input;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_chars_appends_to_input_and_marks_keystroke() {
        let mut state = SearchDialogState::new();
        for c in "red".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        assert_eq!(state.input, "red");
        assert_eq!(state.cursor, 3);
        assert!(state.last_keystroke.is_some());
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut state = SearchDialogState::new();
        for c in "redact".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Backspace));
        assert_eq!(state.input, "redac");
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn esc_returns_close_outcome() {
        let mut state = SearchDialogState::new();
        handle_key(&mut state, key(KeyCode::Char('x')));
        let outcome = handle_key(&mut state, key(KeyCode::Esc));
        assert!(matches!(outcome, SearchDialogOutcome::Close));
    }

    #[test]
    fn enter_returns_submit_outcome() {
        let mut state = SearchDialogState::new();
        let outcome = handle_key(&mut state, key(KeyCode::Enter));
        assert!(matches!(outcome, SearchDialogOutcome::Submit));
    }

    #[test]
    fn arrow_keys_move_selection_within_hits() {
        let mut state = SearchDialogState::new();
        state.hits = vec![dummy_hit(), dummy_hit(), dummy_hit()];
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, 1);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, 2);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, 2, "selection should clamp at hits.len() - 1");
        handle_key(&mut state, key(KeyCode::Up));
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn ctrl_modified_chars_are_consumed_without_input_change() {
        let mut state = SearchDialogState::new();
        let ctrl_key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        let outcome = handle_key(&mut state, ctrl_key);
        assert_eq!(state.input, "");
        assert!(matches!(outcome, SearchDialogOutcome::Consumed));
    }

    fn dummy_hit() -> libllm::search::SearchHit {
        libllm::search::SearchHit {
            session_id: "s".into(),
            session_display_name: "S".into(),
            message_id: 0,
            message_rowid: 0,
            role: libllm::session::Role::User,
            timestamp: time::OffsetDateTime::now_utc(),
            snippet: String::new(),
            preview_text: String::new(),
            score: 0.0,
        }
    }

    fn seed_db_for_tests(rows: &[(&str, &str, &str)]) -> (libllm::db::Database, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        {
            let _db = libllm::db::Database::open(file.path(), None).unwrap();
        }
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        let mut session_ids: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for (display, role, content) in rows {
            let session_id = format!("sess-{display}");
            let display_owned = (*display).to_owned();
            if !session_ids.contains_key(&display_owned) {
                conn.execute(
                    "INSERT INTO sessions (id, display_name, created_at, updated_at) \
                     VALUES (?1, ?2, 'now', 'now')",
                    rusqlite::params![session_id, display],
                )
                .unwrap();
                session_ids.insert(display_owned.clone(), 0);
            }
            let next_id = session_ids.get_mut(&display_owned).unwrap();
            conn.execute(
                "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp) \
                 VALUES (?1, ?2, NULL, NULL, ?3, ?4, '2026-01-01T00:00:00Z')",
                rusqlite::params![*next_id, session_id, role, content],
            )
            .unwrap();
            *next_id += 1;
        }
        drop(conn);
        let db = libllm::db::Database::open(file.path(), None).unwrap();
        (db, file)
    }

    #[test]
    fn maybe_run_query_returns_hits_after_debounce() {
        let (db, _file) = seed_db_for_tests(&[
            ("alpha", "user", "remember to redact PII"),
            ("alpha", "assistant", "redaction working"),
        ]);
        let mut state = SearchDialogState::new();
        for c in "red".chars() {
            handle_key(&mut state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let now = Instant::now();
        state.last_keystroke = Some(now - std::time::Duration::from_millis(200));

        maybe_run_query(&mut state, &db, now);

        assert!(!state.hits.is_empty(), "expected hits, error={:?}", state.error);
        assert_eq!(state.last_compiled.as_deref(), Some("red"));
        assert!(state.last_keystroke.is_none(), "last_keystroke should be cleared");
    }

    #[test]
    fn maybe_run_query_with_short_input_clears_hits() {
        let (db, _file) = seed_db_for_tests(&[("alpha", "user", "anything")]);
        let mut state = SearchDialogState::new();
        state.hits = vec![dummy_hit()];
        state.input = "ab".into();
        let now = Instant::now();
        state.last_keystroke = Some(now - std::time::Duration::from_millis(200));

        maybe_run_query(&mut state, &db, now);

        assert!(state.hits.is_empty());
        assert_eq!(state.last_compiled.as_deref(), Some("ab"));
    }

    #[test]
    fn maybe_run_query_skips_within_debounce_window() {
        let (db, _file) = seed_db_for_tests(&[("alpha", "user", "redact")]);
        let mut state = SearchDialogState::new();
        state.input = "redact".into();
        let now = Instant::now();
        state.last_keystroke = Some(now);

        maybe_run_query(&mut state, &db, now);

        assert!(state.hits.is_empty(), "should not run within debounce window");
        assert!(state.last_keystroke.is_some(), "last_keystroke must NOT be cleared");
    }

    #[test]
    fn maybe_run_query_skips_when_input_unchanged() {
        let (db, _file) = seed_db_for_tests(&[("alpha", "user", "redact")]);
        let mut state = SearchDialogState::new();
        state.input = "redact".into();
        state.last_compiled = Some("redact".into());
        let now = Instant::now();
        state.last_keystroke = Some(now - std::time::Duration::from_millis(200));

        maybe_run_query(&mut state, &db, now);

        assert!(state.hits.is_empty(), "should not run when input matches last_compiled");
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn dialog_renders_three_column_list() {
        let theme = crate::tui::theme::Theme::dark();
        let mut state = SearchDialogState::new();
        state.input = "redact".into();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.session_display_name = "feature-x".into();
            hit.role = libllm::session::Role::User;
            hit.snippet = "remember to \u{1}redact\u{2} PII".into();
            hit
        }];

        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);

        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("feature-x"));
        assert!(rendered.contains("user"));
        assert!(rendered.contains("remember to"));
        assert!(rendered.contains("redact"));
        assert!(!rendered.contains('\u{1}'), "raw delimiter leaked into rendered buffer");
    }

    #[test]
    fn dialog_renders_empty_state_message() {
        let theme = crate::tui::theme::Theme::dark();
        let state = SearchDialogState::new();
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("type at least 3"), "expected hint, got: {rendered}");
    }

    #[test]
    fn preview_pane_shows_full_content_with_highlight() {
        let theme = crate::tui::theme::Theme::dark();
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.session_display_name = "alpha".into();
            hit.role = libllm::session::Role::Assistant;
            hit.preview_text = "remember to \u{1}redact\u{2} PII before sending and document why".into();
            hit
        }];

        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);

        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("redact"));
        assert!(rendered.contains("PII"));
        assert!(rendered.contains("alpha"));
    }

    #[test]
    fn preview_truncates_long_messages() {
        let theme = crate::tui::theme::Theme::dark();
        let many_lines: String = (0..200).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.preview_text = many_lines;
            hit
        }];

        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);

        let rendered = buffer_to_string(&buf);
        assert!(
            rendered.contains("more lines"),
            "expected truncation footer, got: {rendered}"
        );
    }

    #[test]
    fn preview_shows_no_preview_when_hits_empty() {
        let theme = crate::tui::theme::Theme::dark();
        let mut state = SearchDialogState::new();
        state.hits = vec![];
        state.input = "redact".into();

        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);

        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("(no preview)"));
    }

    #[test]
    fn preview_truncates_pluralisation_for_many_extra_lines() {
        let theme = crate::tui::theme::Theme::dark();
        let lines: String = (0..30).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.preview_text = lines;
            hit
        }];
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("more lines"), "expected 'more lines' for plural");
    }

    #[test]
    fn preview_does_not_truncate_when_content_fits() {
        let theme = crate::tui::theme::Theme::dark();
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.preview_text = "short message".into();
            hit
        }];
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(!rendered.contains("more line"), "expected no truncation footer");
    }

    #[test]
    fn ctrl_r_adds_m_prefix_when_absent() {
        let mut state = SearchDialogState::new();
        state.input = "redact".into();
        state.cursor = state.input.chars().count();
        handle_key(&mut state, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(state.input, "m:redact");
        assert_eq!(state.cursor, "m:redact".chars().count());
    }

    #[test]
    fn ctrl_r_strips_m_prefix_when_present() {
        let mut state = SearchDialogState::new();
        state.input = "m:redact".into();
        state.cursor = state.input.chars().count();
        handle_key(&mut state, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(state.input, "redact");
        assert_eq!(state.cursor, "redact".chars().count());
    }

    #[test]
    fn page_down_advances_by_page_size() {
        let mut state = SearchDialogState::new();
        state.hits = (0..50).map(|_| dummy_hit()).collect();
        handle_key(&mut state, KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(state.selected, PAGE_SIZE);
    }

    #[test]
    fn page_down_clamps_at_last_hit() {
        let mut state = SearchDialogState::new();
        state.hits = (0..5).map(|_| dummy_hit()).collect();
        handle_key(&mut state, KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn page_up_retreats_by_page_size() {
        let mut state = SearchDialogState::new();
        state.hits = (0..50).map(|_| dummy_hit()).collect();
        state.selected = 25;
        handle_key(&mut state, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(state.selected, 25 - PAGE_SIZE);
    }

    #[test]
    fn page_up_saturates_at_zero() {
        let mut state = SearchDialogState::new();
        state.hits = (0..5).map(|_| dummy_hit()).collect();
        handle_key(&mut state, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn render_preview_caps_at_preview_line_cap() {
        let theme = crate::tui::theme::Theme::dark();
        // Build a preview with PREVIEW_LINE_CAP + 500 lines, each just "x"
        let many_lines: String = std::iter::repeat("x")
            .take(PREVIEW_LINE_CAP + 500)
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.preview_text = many_lines;
            hit
        }];
        // Use a tall area so body_rows is large enough to expose the cap
        let area = ratatui::layout::Rect::new(0, 0, 80, 60);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // Must complete without panicking or OOMing
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("more lines"), "expected truncation footer when lines exceed cap");
    }

    #[test]
    fn render_preview_does_not_allocate_all_lines_eagerly() {
        let theme = crate::tui::theme::Theme::dark();
        // PREVIEW_LINE_CAP * 10 short lines — should complete quickly without OOM
        let huge: String = std::iter::repeat("x")
            .take(PREVIEW_LINE_CAP * 10)
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.preview_text = huge;
            hit
        }];
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(rendered.contains("more lines"));
    }

    #[test]
    fn highlight_spans_strips_csi_in_normal_text() {
        let theme = crate::tui::theme::Theme::dark();
        let spans = highlight_spans("\x1b[31mred\x1b[0m", &theme);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "red");
        assert!(!joined.contains('\x1b'));
    }

    #[test]
    fn highlight_spans_strips_osc_in_highlighted_text() {
        let theme = crate::tui::theme::Theme::dark();
        let spans = highlight_spans("\u{1}hi\x1b]52;c;AA\x07\u{2}", &theme);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hi");
        assert!(!joined.contains('\x1b'));
    }

    #[test]
    fn render_preview_header_strips_escape_in_session_name() {
        let theme = crate::tui::theme::Theme::dark();
        let mut state = SearchDialogState::new();
        state.hits = vec![{
            let mut hit = dummy_hit();
            hit.session_display_name = "name\x1b[31m\x1b[0m".into();
            hit.preview_text = "content".into();
            hit
        }];
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render(&state, area, &mut buf, &theme);
        let rendered = buffer_to_string(&buf);
        assert!(!rendered.contains('\x1b'), "ESC byte must not appear in rendered output");
        assert!(rendered.contains("name"));
    }
}
