//! v4: Adds the dismissed_template_prompts KV table for auto-template-detection.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v4" ; {
        conn.execute_batch(
            "CREATE TABLE dismissed_template_prompts (
                template_hash TEXT PRIMARY KEY,
                dismissed_at INTEGER NOT NULL
            );",
        )
        .map_err(|source| DbError::Query {
            context: "failed to create dismissed_template_prompts table".to_owned(),
            source,
        })
    })
}
