//! v4: Adds the dismissed_template_prompts KV table for auto-template-detection.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE dismissed_template_prompts (
            template_hash TEXT PRIMARY KEY NOT NULL,
            dismissed_at INTEGER NOT NULL
        );",
    )
    .context("failed to create dismissed_template_prompts table")?;
    Ok(())
}
