use std::time::Instant;

use libllm::search::SearchHit;

pub(crate) const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
pub(crate) const MIN_QUERY_CHARS: usize = 3;

#[expect(dead_code, reason = "wired in by Tasks 11-17")]
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

    #[expect(dead_code, reason = "wired in by Tasks 11-17")]
    pub fn with_prefilled(query: &str) -> Self {
        let mut s = Self::new();
        s.input = query.to_owned();
        s.cursor = query.chars().count();
        if query.trim().chars().count() >= MIN_QUERY_CHARS {
            s.last_keystroke = Some(Instant::now());
        }
        s
    }

    #[expect(dead_code, reason = "wired in by Tasks 11-17")]
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
