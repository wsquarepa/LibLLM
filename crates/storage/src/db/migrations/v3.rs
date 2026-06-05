//! v3: add optional assistant thought-duration metadata to messages.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v3" ; {
        conn.execute_batch(
            "ALTER TABLE messages
             ADD COLUMN thought_seconds INTEGER;",
        )
        .map_err(|source| DbError::Query {
            context: "failed to run migration v3".to_owned(),
            source,
        })
    })
}
