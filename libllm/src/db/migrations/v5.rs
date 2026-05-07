//! v5: Adds messages_fts (FTS5 external-content) and the three sync triggers.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v5" ; {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE messages_fts USING fts5(
                 content,
                 content='messages',
                 content_rowid='rowid',
                 tokenize='unicode61 remove_diacritics 2'
             );

             CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
                 INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
             END;

             CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content)
                 VALUES('delete', old.rowid, old.content);
             END;

             CREATE TRIGGER messages_fts_au AFTER UPDATE OF content ON messages BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content)
                 VALUES('delete', old.rowid, old.content);
                 INSERT INTO messages_fts(rowid, content)
                 VALUES (new.rowid, new.content);
             END;

             INSERT INTO messages_fts(rowid, content) SELECT rowid, content FROM messages;",
        )
        .context("failed to run migration v5")
    })
}
