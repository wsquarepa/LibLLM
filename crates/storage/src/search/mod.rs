//! Full-text search across all stored messages.

pub mod query;

use rusqlite::types::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::search::query::CompiledQuery;
use libllm_core::session::Role;

pub const DEFAULT_MAX_HITS: usize = 200;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub session_id: String,
    pub session_display_name: String,
    pub message_id: i64,
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

pub fn search(
    db: &crate::db::Database,
    query: &CompiledQuery,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let result = libllm_core::timed_result!(
        tracing::Level::DEBUG,
        "search.execute",
        terms = query.match_expr.as_str() ;
        { db.with_connection(|conn| run(conn, query, limit)) }
    );
    if let Ok(hits) = &result {
        tracing::debug!(hits = hits.len(), "search.hits");
    }
    result
}

#[expect(
    clippy::expect_used,
    reason = "message timestamps were written by this crate and are within the RFC 3339 range"
)]
fn run(
    conn: &rusqlite::Connection,
    query: &CompiledQuery,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let mut sql = String::from(
        "SELECT m.session_id, \
                s.display_name, \
                m.id, \
                m.role, \
                m.timestamp, \
                snippet(messages_fts, 0, char(1), char(2), '...', 16), \
                highlight(messages_fts, 0, char(1), char(2)), \
                bm25(messages_fts) \
         FROM messages_fts \
         JOIN messages m ON m.rowid = messages_fts.rowid \
         JOIN sessions s ON s.id = m.session_id \
         WHERE messages_fts MATCH ?1",
    );

    let mut params: Vec<Value> = vec![Value::Text(query.match_expr.clone())];

    if let Some(ids) = &query.session_ids {
        sql.push_str(" AND m.session_id IN (");
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            params.push(Value::Text(id.clone()));
            sql.push_str(&format!("?{}", params.len()));
        }
        sql.push(')');
    }

    if let Some(role) = &query.role {
        params.push(Value::Text(role.to_string()));
        sql.push_str(&format!(" AND m.role = ?{}", params.len()));
    }
    if let Some(before) = &query.before {
        params.push(Value::Text(
            before.format(&Rfc3339).expect("RFC 3339 format"),
        ));
        sql.push_str(&format!(" AND m.timestamp < ?{}", params.len()));
    }
    if let Some(after) = &query.after {
        params.push(Value::Text(
            after.format(&Rfc3339).expect("RFC 3339 format"),
        ));
        sql.push_str(&format!(" AND m.timestamp >= ?{}", params.len()));
    }

    sql.push_str(" ORDER BY bm25(messages_fts) LIMIT ");
    sql.push_str(&limit.to_string());

    let mut stmt = conn.prepare(&sql).map_err(map_match_error)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let role_str: String = row.get(3)?;
            let timestamp_str: String = row.get(4)?;
            Ok(RawHit {
                session_id: row.get(0)?,
                session_display_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                message_id: row.get(2)?,
                role_raw: role_str,
                timestamp_raw: timestamp_str,
                snippet: row.get(5)?,
                preview_text: row.get(6)?,
                score: row.get(7)?,
            })
        })
        .map_err(map_match_error)?;

    let mut hits: Vec<SearchHit> = Vec::new();
    for row in rows {
        let raw = row.map_err(map_match_error)?;
        let role: Role = raw.role_raw.parse().map_err(|e| {
            SearchError::InvalidMatch(format!("unrecognised role '{}': {e}", raw.role_raw))
        })?;
        let timestamp = parse_iso(&raw.timestamp_raw).map_err(|e| {
            SearchError::InvalidMatch(format!("bad timestamp '{}': {e}", raw.timestamp_raw))
        })?;
        hits.push(SearchHit {
            session_id: raw.session_id,
            session_display_name: raw.session_display_name,
            message_id: raw.message_id,
            role,
            timestamp,
            snippet: raw.snippet,
            preview_text: raw.preview_text,
            score: raw.score,
        });
    }
    Ok(hits)
}

struct RawHit {
    session_id: String,
    session_display_name: String,
    message_id: i64,
    role_raw: String,
    timestamp_raw: String,
    snippet: String,
    preview_text: String,
    score: f64,
}

fn parse_iso(s: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(s, &Rfc3339)
}

fn map_match_error(err: rusqlite::Error) -> SearchError {
    if let rusqlite::Error::SqliteFailure(ref code, Some(ref msg)) = err
        && code.code == rusqlite::ErrorCode::Unknown
    {
        return SearchError::InvalidMatch(msg.clone());
    }
    SearchError::Db(err)
}

pub use libllm_core::text::strip_terminal_controls;

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::db::Database;

    fn seed() -> (Database, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(file.path(), None).unwrap();
        db.execute_statement(
            "INSERT INTO sessions (id, display_name, created_at, updated_at) \
             VALUES ('s1', 'feature-x', 'now', 'now')",
        )
        .unwrap();
        db.execute_statement(
            "INSERT INTO sessions (id, display_name, created_at, updated_at) \
             VALUES ('s2', 'log-pipeline', 'now', 'now')",
        )
        .unwrap();
        db.execute_statement(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp) \
             VALUES (0, 's1', NULL, NULL, 'user', 'remember to redact PII before sending', '2026-01-12T14:32:11Z')",
        )
        .unwrap();
        db.execute_statement(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp) \
             VALUES (1, 's1', 0, NULL, 'assistant', 'redaction working now', '2026-01-12T14:33:02Z')",
        )
        .unwrap();
        db.execute_statement(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp) \
             VALUES (0, 's2', NULL, NULL, 'user', 'PII redaction at sink', '2026-02-03T09:11:00Z')",
        )
        .unwrap();
        (db, file)
    }

    #[test]
    fn returns_hits_ordered_by_bm25() {
        let (db, _file) = seed();
        let q = query::compile("redact", &db).unwrap();
        let hits = search(&db, &q, DEFAULT_MAX_HITS).unwrap();
        assert_eq!(hits.len(), 3);
        assert!(hits[0].snippet.contains('\u{1}'));
        assert!(hits[0].preview_text.contains('\u{1}'));
    }

    #[test]
    fn role_filter_excludes_other_roles() {
        let (db, _file) = seed();
        let q = query::compile("role:assistant redact", &db).unwrap();
        let hits = search(&db, &q, DEFAULT_MAX_HITS).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, Role::Assistant);
    }

    #[test]
    fn before_filter_is_exclusive() {
        let (db, _file) = seed();
        let q = query::compile("before:2026-02-01 redact", &db).unwrap();
        let hits = search(&db, &q, DEFAULT_MAX_HITS).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.session_id == "s1"));
    }

    #[test]
    fn limit_is_respected() {
        let (db, _file) = seed();
        let q = query::compile("redact", &db).unwrap();
        let hits = search(&db, &q, 1).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn session_substring_filter_narrows_results() {
        let (db, _file) = seed();
        let q = query::compile("session:log redact", &db).unwrap();
        let hits = search(&db, &q, DEFAULT_MAX_HITS).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "s2");
    }

    #[test]
    fn combined_filters_apply_together() {
        let (db, _file) = seed();
        let q = query::compile("role:user before:2026-02-01 redact", &db).unwrap();
        let hits = search(&db, &q, DEFAULT_MAX_HITS).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "s1");
        assert_eq!(hits[0].role, Role::User);
    }

    #[test]
    fn malformed_match_expression_returns_invalid_match() {
        let (db, _file) = seed();
        let q = query::compile("m:(((", &db).unwrap();
        let err = search(&db, &q, DEFAULT_MAX_HITS).unwrap_err();
        assert!(
            matches!(err, SearchError::InvalidMatch(_)),
            "expected InvalidMatch, got {err:?}"
        );
    }
}
