//! v5: Adds author_note, author_note_depth, and author_note_at_top columns to sessions and characters.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v5" ; {
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN author_note TEXT;
             ALTER TABLE sessions ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
             ALTER TABLE sessions ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE characters ADD COLUMN author_note TEXT;
             ALTER TABLE characters ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
             ALTER TABLE characters ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;",
        )
        .context("v5 migration failed")
    })
}
