//! v3: add optional assistant thought-duration metadata to messages.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v3" ; {
        conn.execute_batch(
            "ALTER TABLE messages
             ADD COLUMN thought_seconds INTEGER;",
        )
        .context("failed to run migration v3")
    })
}
