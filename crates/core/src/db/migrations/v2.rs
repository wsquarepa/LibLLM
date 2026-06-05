//! v2: `file_summaries` cache for per-session LLM summaries of attached files.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v2" ; {
        conn.execute_batch(
            "CREATE TABLE file_summaries (
                session_id   TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                basename     TEXT NOT NULL,
                summary      TEXT NOT NULL DEFAULT '',
                status       TEXT NOT NULL,
                created_at   TEXT NOT NULL,
                updated_at   TEXT NOT NULL,
                PRIMARY KEY (session_id, content_hash),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX idx_file_summaries_status ON file_summaries(status);",
        )
        .context("failed to run migration v2")
    })
}
