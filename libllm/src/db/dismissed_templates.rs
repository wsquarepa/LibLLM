//! Stores per-template-hash dismissals so the auto-template-detect popup
//! does not re-prompt the user about a template they've already declined.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::timed_result;

pub fn is_dismissed(conn: &Connection, template_hash: &str) -> Result<bool> {
    timed_result!(tracing::Level::DEBUG, "db.dismissed_template.lookup", hash = template_hash ; {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM dismissed_template_prompts WHERE template_hash = ?1)",
                params![template_hash],
                |row| row.get(0),
            )
            .context("failed to query dismissed_template_prompts")?;
        Ok(exists)
    })
}

pub fn record_dismissal(conn: &Connection, template_hash: &str) -> Result<()> {
    timed_result!(tracing::Level::INFO, "db.dismissed_template.record", hash = template_hash ; {
        let now = unix_secs_now();
        conn.execute(
            "INSERT INTO dismissed_template_prompts (template_hash, dismissed_at) \
             VALUES (?1, ?2) \
             ON CONFLICT(template_hash) DO UPDATE SET dismissed_at = excluded.dismissed_at",
            params![template_hash, now],
        )
        .context("failed to record template dismissal")?;
        Ok(())
    })
}

pub fn clear_all(conn: &Connection) -> Result<u64> {
    timed_result!(tracing::Level::INFO, "db.dismissed_template.clear_all", ; {
        let affected = conn
            .execute("DELETE FROM dismissed_template_prompts", [])
            .context("failed to clear dismissed_template_prompts")?;
        Ok(affected as u64)
    })
}

fn unix_secs_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn unknown_hash_is_not_dismissed() {
        let conn = fresh_conn();
        assert!(!is_dismissed(&conn, "deadbeef").unwrap());
    }

    #[test]
    fn record_then_lookup_returns_true() {
        let conn = fresh_conn();
        record_dismissal(&conn, "abc123").unwrap();
        assert!(is_dismissed(&conn, "abc123").unwrap());
    }

    #[test]
    fn record_is_idempotent() {
        let conn = fresh_conn();
        record_dismissal(&conn, "abc123").unwrap();
        record_dismissal(&conn, "abc123").unwrap();
        assert!(is_dismissed(&conn, "abc123").unwrap());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM dismissed_template_prompts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn clear_all_removes_entries_and_returns_count() {
        let conn = fresh_conn();
        record_dismissal(&conn, "h1").unwrap();
        record_dismissal(&conn, "h2").unwrap();
        record_dismissal(&conn, "h3").unwrap();
        let removed = clear_all(&conn).unwrap();
        assert_eq!(removed, 3);
        assert!(!is_dismissed(&conn, "h1").unwrap());
    }

    #[test]
    fn clear_all_on_empty_returns_zero() {
        let conn = fresh_conn();
        let removed = clear_all(&conn).unwrap();
        assert_eq!(removed, 0);
    }
}
