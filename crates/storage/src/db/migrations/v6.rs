//! v6: per-session and per-character author's note with injection depth and position.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v6" ; {
        let sessions_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)")
            .map_err(|source| DbError::Query {
                context: "failed to prepare PRAGMA table_info(sessions)".to_owned(),
                source,
            })?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|source| DbError::Query {
                context: "failed to query sessions columns".to_owned(),
                source,
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| DbError::Query {
                context: "failed to collect sessions columns".to_owned(),
                source,
            })?;

        if !sessions_cols.iter().any(|c| c == "author_note") {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN author_note TEXT;
                 ALTER TABLE sessions ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
                 ALTER TABLE sessions ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;",
            )
            .map_err(|source| DbError::Query {
                context: "failed to add author_note columns to sessions".to_owned(),
                source,
            })?;
        }

        let chars_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(characters)")
            .map_err(|source| DbError::Query {
                context: "failed to prepare PRAGMA table_info(characters)".to_owned(),
                source,
            })?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|source| DbError::Query {
                context: "failed to query characters columns".to_owned(),
                source,
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| DbError::Query {
                context: "failed to collect characters columns".to_owned(),
                source,
            })?;

        if !chars_cols.iter().any(|c| c == "author_note") {
            conn.execute_batch(
                "ALTER TABLE characters ADD COLUMN author_note TEXT;
                 ALTER TABLE characters ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
                 ALTER TABLE characters ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;",
            )
            .map_err(|source| DbError::Query {
                context: "failed to add author_note columns to characters".to_owned(),
                source,
            })?;
        }

        Ok(())
    })
}
