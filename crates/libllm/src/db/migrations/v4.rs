//! v4: Adds the dismissed_template_prompts KV table for auto-template-detection.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v4" ; {
        conn.execute_batch(
            "CREATE TABLE dismissed_template_prompts (
                template_hash TEXT PRIMARY KEY,
                dismissed_at INTEGER NOT NULL
            );",
        )
        .context("failed to create dismissed_template_prompts table")
    })
}
