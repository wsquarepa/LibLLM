//! Versioned database migration runner.
//!
//! Each migration lives in its own file (`v1.rs`, `v2.rs`, ...) and exposes a
//! single `pub(super) fn migrate(conn: &Connection) -> Result<()>`. `run_migrations`
//! reads the current schema version, runs every missing step in order, and
//! stamps each one as it finishes. Adding a new migration is three lines: a new
//! file, a `mod vN;` declaration, and an `if version < N` branch below.

mod v1;
mod v2;
mod v3;
mod v4;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub const CURRENT_VERSION: i64 = 4;

pub fn run_migrations(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", ; {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );",
        )
        .context("failed to create schema_version table")?;

        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .context("failed to read schema version")?;

        let mut applied = 0usize;
        if version < 1 {
            v1::migrate(conn)?;
            stamp_version(conn, 1)?;
            applied += 1;
        }
        if version < 2 {
            v2::migrate(conn)?;
            stamp_version(conn, 2)?;
            applied += 1;
        }
        if version < 3 {
            v3::migrate(conn)?;
            stamp_version(conn, 3)?;
            applied += 1;
        }
        if version < 4 {
            v4::migrate(conn)?;
            stamp_version(conn, 4)?;
            applied += 1;
        }

        tracing::info!(
            phase = "summary",
            from_version = version,
            to_version = CURRENT_VERSION,
            applied = applied,
            "db.migrate",
        );
        Ok(())
    })
}

fn stamp_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        rusqlite::params![version],
    )
    .context("failed to record schema version")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::run_migrations;

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, super::CURRENT_VERSION);
    }

    #[test]
    fn v1_creates_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let expected_tables = [
            "schema_version",
            "sessions",
            "session_worldbooks",
            "messages",
            "characters",
            "worldbooks",
            "system_prompts",
            "personas",
            "file_summaries",
        ];

        for table in &expected_tables {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "table '{table}' was not created");
        }
    }

    #[test]
    fn v2_creates_file_summaries_with_expected_columns() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(file_summaries)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for expected in [
            "session_id",
            "content_hash",
            "basename",
            "summary",
            "status",
            "created_at",
            "updated_at",
        ] {
            assert!(
                cols.iter().any(|c| c == expected),
                "missing column '{expected}' in {cols:?}"
            );
        }
    }

    #[test]
    fn v2_creates_status_index() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_file_summaries_status'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists);
    }

    #[test]
    fn v3_adds_messages_thought_seconds_column() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(messages)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(
            cols.iter().any(|c| c == "thought_seconds"),
            "missing column 'thought_seconds' in {cols:?}"
        );
    }

    #[test]
    fn upgrade_from_v1_preserves_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (1);",
        )
        .unwrap();
        super::v1::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO characters (slug, name, created_at, updated_at)
             VALUES ('alice', 'Alice', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let name: String = conn
            .query_row(
                "SELECT name FROM characters WHERE slug='alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Alice");
    }

    #[test]
    fn upgrade_from_v2_preserves_existing_messages() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (2);",
        )
        .unwrap();
        super::v1::migrate(&conn).unwrap();
        super::v2::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (0, 's1', NULL, NULL, 'assistant', 'hello', 'now')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let content: String = conn
            .query_row(
                "SELECT content FROM messages WHERE session_id = 's1' AND id = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let thought_seconds: Option<i64> = conn
            .query_row(
                "SELECT thought_seconds FROM messages WHERE session_id = 's1' AND id = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(content, "hello");
        assert_eq!(thought_seconds, None);
    }

    #[test]
    fn cascade_deletes_file_summaries_when_session_deleted() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_summaries (session_id, content_hash, basename, summary, status, created_at, updated_at)
             VALUES ('s1', 'hash1', 'a.md', 'summary', 'done', 'now', 'now')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM sessions WHERE id='s1'", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_summaries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn fresh_db_has_dismissed_template_prompts_table() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                ["dismissed_template_prompts"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists);

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, super::CURRENT_VERSION);
    }
}
