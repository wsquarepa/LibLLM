use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use libllm::search::SearchHit;

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
    #[expect(dead_code, reason = "wired in by Task 12")]
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

    #[expect(dead_code, reason = "wired in by Task 12")]
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
}
