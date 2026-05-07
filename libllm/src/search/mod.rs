//! Full-text search across all stored messages.

pub mod query;

use time::OffsetDateTime;

use crate::session::Role;

pub const DEFAULT_MAX_HITS: usize = 200;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub session_id: String,
    pub session_display_name: String,
    pub message_id: i64,
    pub message_rowid: i64,
    pub role: Role,
    pub timestamp: OffsetDateTime,
    pub snippet: String,
    pub preview_text: String,
    pub score: f64,
}

#[derive(Debug)]
pub enum SearchError {
    Db(rusqlite::Error),
    InvalidMatch(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(err) => write!(f, "database error: {err}"),
            Self::InvalidMatch(msg) => write!(f, "invalid match expression: {msg}"),
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(err) => Some(err),
            Self::InvalidMatch(_) => None,
        }
    }
}

impl From<rusqlite::Error> for SearchError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_hit_is_clone() {
        let hit = SearchHit {
            session_id: "s".into(),
            session_display_name: "S".into(),
            message_id: 0,
            message_rowid: 0,
            role: Role::User,
            timestamp: OffsetDateTime::now_utc(),
            snippet: String::new(),
            preview_text: String::new(),
            score: 0.0,
        };
        let _clone = hit.clone();
    }
}
