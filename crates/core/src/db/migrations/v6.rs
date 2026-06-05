//! v6: per-session and per-character author's note with injection depth and position.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v6" ; {
        let sessions_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)")
            .context("failed to prepare PRAGMA table_info(sessions)")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("failed to query sessions columns")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to collect sessions columns")?;

        if !sessions_cols.iter().any(|c| c == "author_note") {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN author_note TEXT;
                 ALTER TABLE sessions ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
                 ALTER TABLE sessions ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;",
            )
            .context("failed to add author_note columns to sessions")?;
        }

        let chars_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(characters)")
            .context("failed to prepare PRAGMA table_info(characters)")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("failed to query characters columns")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to collect characters columns")?;

        if !chars_cols.iter().any(|c| c == "author_note") {
            conn.execute_batch(
                "ALTER TABLE characters ADD COLUMN author_note TEXT;
                 ALTER TABLE characters ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
                 ALTER TABLE characters ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;",
            )
            .context("failed to add author_note columns to characters")?;
        }

        Ok(())
    })
}
