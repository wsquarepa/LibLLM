//! Database schema definitions and versioned migration runner.

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::debug_log;

pub const CURRENT_VERSION: i64 = 1;

pub fn run_migrations(conn: &Connection) -> Result<()> {
    debug_log::timed_result("db.migrate", &[], || {
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
        if version < CURRENT_VERSION {
            migrate_v1(conn)?;
            conn.execute_batch(&format!(
                "INSERT INTO schema_version (version) VALUES ({CURRENT_VERSION});"
            ))
            .context("failed to record schema version")?;
            applied += 1;
        }

        debug_log::log_kv(
            "db.migrate",
            &[
                debug_log::field("phase", "summary"),
                debug_log::field("from_version", version),
                debug_log::field("to_version", CURRENT_VERSION),
                debug_log::field("applied", applied),
            ],
        );
        Ok(())
    })
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    debug_log::timed_result("db.migrate", &[debug_log::field("phase", "v1")], || {
        conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY NOT NULL,
            display_name TEXT,
            model TEXT,
            template TEXT,
            system_prompt TEXT,
            character TEXT,
            persona TEXT,
            head_id INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE session_worldbooks (
            session_id TEXT NOT NULL,
            worldbook_slug TEXT NOT NULL,
            PRIMARY KEY (session_id, worldbook_slug),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE messages (
            id INTEGER NOT NULL,
            session_id TEXT NOT NULL,
            parent_id INTEGER,
            preferred_child_id INTEGER,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            PRIMARY KEY (session_id, id),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX idx_messages_session ON messages(session_id);

        CREATE TABLE characters (
            slug TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            personality TEXT,
            scenario TEXT,
            first_mes TEXT,
            mes_example TEXT,
            system_prompt TEXT,
            post_history_instructions TEXT,
            alternate_greetings TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE worldbooks (
            slug TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            entries TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE system_prompts (
            slug TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            content TEXT NOT NULL,
            builtin INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE personas (
            slug TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            persona TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        )
        .context("failed to run migration v1")
    })
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
            .query_row(
                "SELECT MAX(version) FROM schema_version",
                [],
                |row| row.get(0),
            )
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
}
