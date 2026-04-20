//! Database schema definitions and versioned migration runner.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub const CURRENT_VERSION: i64 = 2;

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
            migrate_v1(conn)?;
            stamp_version(conn, 1)?;
            applied += 1;
        }
        if version < 2 {
            migrate_v2(conn)?;
            stamp_version(conn, 2)?;
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

pub(super) fn migrate_v1(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v1" ; {
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

fn migrate_v2(conn: &Connection) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v2" ; {
        conn.execute_batch(
            "CREATE TABLE file_summaries (
                session_id   TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                basename     TEXT NOT NULL,
                summary      TEXT NOT NULL DEFAULT '',
                status       TEXT NOT NULL,
                created_at   TEXT NOT NULL,
                updated_at   TEXT NOT NULL,
                PRIMARY KEY (session_id, content_hash),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX idx_file_summaries_status ON file_summaries(status);",
        )
        .context("failed to run migration v2")
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
    fn upgrade_from_v1_preserves_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (1);",
        )
        .unwrap();
        super::migrate_v1(&conn).unwrap();
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
}
