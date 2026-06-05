//! v5: Adds session_characters table and group-chat columns on sessions/messages.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v5" ; {
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN chat_policy TEXT NOT NULL DEFAULT 'round_robin';
             ALTER TABLE sessions ADD COLUMN card_assembly TEXT NOT NULL DEFAULT 'join_cards';

             CREATE TABLE session_characters (
                 session_id     TEXT NOT NULL,
                 slug           TEXT NOT NULL,
                 attach_index   INTEGER NOT NULL,
                 talkativeness  REAL NOT NULL DEFAULT 0.5,
                 action_points  REAL NOT NULL DEFAULT 0.0,
                 PRIMARY KEY (session_id, slug),
                 FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
             );
             CREATE INDEX idx_session_characters_session
                 ON session_characters(session_id, attach_index);

             ALTER TABLE messages ADD COLUMN speaker_slug TEXT;
             ALTER TABLE messages ADD COLUMN pre_turn_action_points TEXT;

             INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points)
             SELECT id, character, 0, 1.0, 0.0
             FROM sessions
             WHERE character IS NOT NULL;",
        )
        .map_err(|source| DbError::Query {
            context: "failed to run migration v5".to_owned(),
            source,
        })
    })
}
