//! CRUD for the `file_summaries` table: per-session cached LLM summaries of
//! attached file snapshots.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::session::now_iso8601;

/// Lifecycle of a cached file summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileSummaryStatus {
    Pending,
    Done,
    Failed,
}

impl FileSummaryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(anyhow::anyhow!("unknown file_summaries.status: {other}")),
        }
    }
}

/// Row shape returned by `lookup`.
#[derive(Debug, Clone)]
pub struct FileSummaryRow {
    pub basename: String,
    pub summary: String,
    pub status: FileSummaryStatus,
}

/// Insert a new `pending` row. If a row already exists for
/// `(session_id, content_hash)`, leaves it untouched and returns `Ok(false)`.
/// Returns `Ok(true)` when a new row was created.
pub fn insert_pending(
    conn: &Connection,
    session_id: &str,
    content_hash: &str,
    basename: &str,
) -> Result<bool> {
    let now = now_iso8601();
    let changes = conn
        .execute(
            "INSERT OR IGNORE INTO file_summaries
             (session_id, content_hash, basename, summary, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, '', 'pending', ?4, ?4)",
            params![session_id, content_hash, basename, now],
        )
        .context("failed to insert file_summaries row")?;
    Ok(changes == 1)
}

/// Transition a row to `done` with the given summary text. Returns the number
/// of rows affected (0 if the row was deleted by a cascading session delete).
pub fn set_done(
    conn: &Connection,
    session_id: &str,
    content_hash: &str,
    summary: &str,
) -> Result<usize> {
    let now = now_iso8601();
    let n = conn
        .execute(
            "UPDATE file_summaries
             SET summary = ?1, status = 'done', updated_at = ?2
             WHERE session_id = ?3 AND content_hash = ?4",
            params![summary, now, session_id, content_hash],
        )
        .context("failed to set file_summaries done")?;
    Ok(n)
}

/// Transition a row to `failed`. Clears the summary text.
pub fn set_failed(conn: &Connection, session_id: &str, content_hash: &str) -> Result<usize> {
    let now = now_iso8601();
    let n = conn
        .execute(
            "UPDATE file_summaries
             SET summary = '', status = 'failed', updated_at = ?1
             WHERE session_id = ?2 AND content_hash = ?3",
            params![now, session_id, content_hash],
        )
        .context("failed to set file_summaries failed")?;
    Ok(n)
}

/// Fetch a single row. Returns `Ok(None)` if absent.
pub fn lookup(
    conn: &Connection,
    session_id: &str,
    content_hash: &str,
) -> Result<Option<FileSummaryRow>> {
    let row = conn
        .query_row(
            "SELECT basename, summary, status
             FROM file_summaries
             WHERE session_id = ?1 AND content_hash = ?2",
            params![session_id, content_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .context("failed to lookup file_summaries row")?;
    match row {
        None => Ok(None),
        Some((basename, summary, status)) => Ok(Some(FileSummaryRow {
            basename,
            summary,
            status: FileSummaryStatus::parse(&status)?,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn insert_pending_creates_row() {
        let conn = setup();
        let inserted = insert_pending(&conn, "s1", "hash1", "a.md").unwrap();
        assert!(inserted);

        let row = lookup(&conn, "s1", "hash1").unwrap().unwrap();
        assert_eq!(row.basename, "a.md");
        assert_eq!(row.summary, "");
        assert_eq!(row.status, FileSummaryStatus::Pending);
    }

    #[test]
    fn insert_pending_is_idempotent() {
        let conn = setup();
        assert!(insert_pending(&conn, "s1", "hash1", "a.md").unwrap());
        assert!(!insert_pending(&conn, "s1", "hash1", "a.md").unwrap());

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_summaries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn set_done_transitions_pending_row() {
        let conn = setup();
        insert_pending(&conn, "s1", "hash1", "a.md").unwrap();
        let n = set_done(&conn, "s1", "hash1", "summary text").unwrap();
        assert_eq!(n, 1);

        let row = lookup(&conn, "s1", "hash1").unwrap().unwrap();
        assert_eq!(row.status, FileSummaryStatus::Done);
        assert_eq!(row.summary, "summary text");
    }

    #[test]
    fn set_failed_clears_summary() {
        let conn = setup();
        insert_pending(&conn, "s1", "hash1", "a.md").unwrap();
        set_done(&conn, "s1", "hash1", "old summary").unwrap();
        let n = set_failed(&conn, "s1", "hash1").unwrap();
        assert_eq!(n, 1);

        let row = lookup(&conn, "s1", "hash1").unwrap().unwrap();
        assert_eq!(row.status, FileSummaryStatus::Failed);
        assert_eq!(row.summary, "");
    }

    #[test]
    fn lookup_returns_none_for_missing_row() {
        let conn = setup();
        assert!(lookup(&conn, "s1", "nope").unwrap().is_none());
    }

    #[test]
    fn lookup_scoped_to_session_id() {
        let conn = setup();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s2', 'now', 'now')",
            [],
        )
        .unwrap();
        insert_pending(&conn, "s1", "hash1", "a.md").unwrap();

        assert!(lookup(&conn, "s2", "hash1").unwrap().is_none());
        assert!(lookup(&conn, "s1", "hash1").unwrap().is_some());
    }

    #[test]
    fn status_round_trip() {
        for s in [
            FileSummaryStatus::Pending,
            FileSummaryStatus::Done,
            FileSummaryStatus::Failed,
        ] {
            assert_eq!(FileSummaryStatus::parse(s.as_str()).unwrap(), s);
        }
    }
}
