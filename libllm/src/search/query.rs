use time::OffsetDateTime;

use crate::session::Role;

#[derive(Debug, Clone)]
pub struct CompiledQuery {
    pub match_expr: String,
    pub session_ids: Option<Vec<String>>,
    pub role: Option<Role>,
    pub before: Option<OffsetDateTime>,
    pub after: Option<OffsetDateTime>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    Empty,
    BadFilter(String),
    UnknownSession(String),
    ParseDate(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("query is empty"),
            Self::BadFilter(s) => write!(f, "malformed scope filter: {s}"),
            Self::UnknownSession(s) => write!(f, "no session matched: {s}"),
            Self::ParseDate(s) => write!(f, "malformed date: {s}"),
        }
    }
}

impl std::error::Error for QueryError {}
