//! v11: heal partial author-note columns left by a buggy v6 path.
//!
//! Early v6 checked only for `author_note` and, if present, skipped adding
//! `author_note_depth` and `author_note_at_top`. Databases already stamped at
//! schema version >= 6 never re-ran that migration, so this step re-applies the
//! independent per-column ensure for sessions and characters.

use rusqlite::Connection;

use crate::error::Result;

use super::v6::ensure_author_note_columns;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v11" ; {
        ensure_author_note_columns(conn)
    })
}
