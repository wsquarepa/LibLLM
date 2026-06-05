//! v7: Adds messages_fts (FTS5 external-content) and the three sync triggers.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v7" ; {
        let fts_existed: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='messages_fts'",
                [],
                |row| row.get(0),
            )
            .map_err(|source| DbError::Query {
                context: "failed to check messages_fts existence".to_owned(),
                source,
            })?;

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                 content,
                 content='messages',
                 content_rowid='rowid',
                 tokenize='unicode61 remove_diacritics 2'
             );

             CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
                 INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
             END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content)
                 VALUES('delete', old.rowid, old.content);
             END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE OF content ON messages BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content)
                 VALUES('delete', old.rowid, old.content);
                 INSERT INTO messages_fts(rowid, content)
                 VALUES (new.rowid, new.content);
             END;",
        )
        .map_err(|source| DbError::Query {
            context: "failed to create messages_fts table and triggers".to_owned(),
            source,
        })?;

        // Backfill only when the table was just created. On an old-v5 database
        // the table already holds indexed content and a second pass would
        // double-count every term. A COUNT(*) on an external-content FTS5 table
        // proxies to the content table, so it cannot detect an empty index --
        // the pre-DDL existence check is the correct signal.
        if !fts_existed {
            conn.execute_batch(
                "INSERT INTO messages_fts(rowid, content) SELECT rowid, content FROM messages;",
            )
            .map_err(|source| DbError::Query {
                context: "failed to backfill messages_fts".to_owned(),
                source,
            })?;
        }
        Ok(())
    })
}
