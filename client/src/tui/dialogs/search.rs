use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use libllm::db::Database;
use libllm::search::{self, SearchHit};
use libllm::search::query as search_query;

use super::super::types::Focus;

pub(crate) const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
pub(crate) const MIN_QUERY_CHARS: usize = 3;

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

    #[expect(dead_code, reason = "wired in by Task 17")]
    pub fn with_prefilled(query: &str) -> Self {
        let mut s = Self::new();
        s.input = query.to_owned();
        s.cursor = query.chars().count();
        if query.trim().chars().count() >= MIN_QUERY_CHARS {
            s.last_keystroke = Some(Instant::now());
        }
        s
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
}
