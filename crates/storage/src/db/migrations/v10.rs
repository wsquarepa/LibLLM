//! v10: Rebuild dismissed_template_prompts with NOT NULL on the TEXT PRIMARY KEY.
//!
//! SQLite TEXT PRIMARY KEY without explicit NOT NULL permits NULL values, which
//! breaks the intended key-value semantics. The table-rebuild idiom is the only
//! way to add a NOT NULL constraint to an existing column in SQLite.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v10" ; {
        conn.execute_batch(
            "CREATE TABLE dismissed_template_prompts_new (
                template_hash TEXT PRIMARY KEY NOT NULL,
                dismissed_at  INTEGER NOT NULL
            );

            INSERT INTO dismissed_template_prompts_new (template_hash, dismissed_at)
            SELECT template_hash, dismissed_at
            FROM dismissed_template_prompts
            WHERE template_hash IS NOT NULL;

            DROP TABLE dismissed_template_prompts;

            ALTER TABLE dismissed_template_prompts_new
                RENAME TO dismissed_template_prompts;",
        )
        .map_err(|source| DbError::Query {
            context: "failed to rebuild dismissed_template_prompts with NOT NULL constraint"
                .to_owned(),
            source,
        })
    })
}
