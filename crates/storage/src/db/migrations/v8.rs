//! v8: Reset `session_characters.action_points` for the HSR-style action-value engine.
//!
//! Prior to v8, `action_points` was AP under a threshold-and-cost model: high values
//! meant "ready to act". Starting in v8 the same column stores Honkai-Star-Rail-style
//! action *values* where low (and zero) means "ready to act". Carrying old AP numbers
//! into the new engine would freeze high-AP characters at the back of the queue
//! indefinitely, so we zero them out on upgrade. Talkativeness values are unchanged.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v8" ; {
        conn.execute_batch("UPDATE session_characters SET action_points = 0;")
            .map_err(|source| DbError::Query {
                context: "failed to run migration v8".to_owned(),
                source,
            })
    })
}
