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
mod v5;
mod v6;
mod v7;
mod v8;
mod v9;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub const CURRENT_VERSION: i64 = 9;

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
        if version < 5 {
            v5::migrate(conn)?;
            stamp_version(conn, 5)?;
            applied += 1;
        }
        if version < 6 {
            v6::migrate(conn)?;
            stamp_version(conn, 6)?;
            applied += 1;
        }
        if version < 7 {
            v7::migrate(conn)?;
            stamp_version(conn, 7)?;
            applied += 1;
        }
        if version < 8 {
            v8::migrate(conn)?;
            stamp_version(conn, 8)?;
            applied += 1;
        }
        if version < 9 {
            v9::migrate(conn)?;
            stamp_version(conn, 9)?;
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
    fn v6_adds_author_note_columns_to_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for expected in ["author_note", "author_note_depth", "author_note_at_top"] {
            assert!(
                cols.iter().any(|c| c == expected),
                "missing column '{expected}' on sessions; got {cols:?}"
            );
        }
    }

    #[test]
    fn v6_adds_author_note_columns_to_characters() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(characters)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for expected in ["author_note", "author_note_depth", "author_note_at_top"] {
            assert!(
                cols.iter().any(|c| c == expected),
                "missing column '{expected}' on characters; got {cols:?}"
            );
        }
    }

    #[test]
    fn v6_default_depth_is_four() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();

        let depth: i64 = conn
            .query_row(
                "SELECT author_note_depth FROM sessions WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(depth, 4);
    }

    #[test]
    fn upgrade_from_v4_preserves_existing_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (4);",
        )
        .unwrap();
        super::v1::migrate(&conn).unwrap();
        super::v2::migrate(&conn).unwrap();
        super::v3::migrate(&conn).unwrap();
        super::v4::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, character, created_at, updated_at)
             VALUES ('s-pre-v6', 'Aria', 'now', 'now')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let character: Option<String> = conn
            .query_row(
                "SELECT character FROM sessions WHERE id = 's-pre-v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let note: Option<String> = conn
            .query_row(
                "SELECT author_note FROM sessions WHERE id = 's-pre-v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let depth: i64 = conn
            .query_row(
                "SELECT author_note_depth FROM sessions WHERE id = 's-pre-v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(character.as_deref(), Some("Aria"));
        assert_eq!(note, None);
        assert_eq!(depth, 4);
    }

    #[test]
    fn fts5_is_available() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE probe USING fts5(content); \
             INSERT INTO probe(content) VALUES ('hello world'); \
             SELECT COUNT(*) FROM probe WHERE probe MATCH 'hello';",
        )
        .expect("FTS5 not available; check rusqlite build features");
    }

    #[test]
    fn v7_creates_messages_fts_and_triggers() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='messages_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "messages_fts virtual table missing");

        for trigger in ["messages_fts_ai", "messages_fts_ad", "messages_fts_au"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='trigger' AND name=?1",
                    rusqlite::params![trigger],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "trigger '{trigger}' missing");
        }
    }

    #[test]
    fn v7_triggers_keep_fts_in_sync() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (0, 's1', NULL, NULL, 'user', 'hello redact world', 'now')",
            [],
        )
        .unwrap();

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'redact'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "INSERT trigger did not populate FTS");

        conn.execute(
            "UPDATE messages SET content = 'hello world' WHERE session_id = 's1' AND id = 0",
            [],
        )
        .unwrap();
        let hits_after_update: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'redact'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits_after_update, 0, "UPDATE trigger did not retokenize");

        conn.execute("DELETE FROM messages WHERE session_id = 's1' AND id = 0", [])
            .unwrap();
        let hits_after_delete: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'world'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits_after_delete, 0, "DELETE trigger did not purge FTS row");
    }

    #[test]
    fn v7_backfill_indexes_pre_existing_messages() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (4);",
        )
        .unwrap();
        super::v1::migrate(&conn).unwrap();
        super::v2::migrate(&conn).unwrap();
        super::v3::migrate(&conn).unwrap();
        super::v4::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (0, 's1', NULL, NULL, 'user', 'pre-existing redact text', 'now')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'redact'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "backfill did not index existing messages");
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

    #[test]
    fn v5_adds_session_characters_table() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_characters')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "session_characters table missing");

        let idx_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_session_characters_session')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(idx_exists, "idx_session_characters_session missing");
    }

    #[test]
    fn v5_adds_chat_mode_to_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
        let cols: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        // v5 added chat_policy; v9 renamed it to chat_mode and dropped card_assembly
        assert!(cols.iter().any(|c| c == "chat_mode"), "missing chat_mode in {cols:?}");
        assert!(!cols.iter().any(|c| c == "chat_policy"), "stale chat_policy still present in {cols:?}");
        assert!(!cols.iter().any(|c| c == "card_assembly"), "stale card_assembly still present in {cols:?}");
    }

    #[test]
    fn v5_adds_speaker_slug_and_pre_turn_action_points_to_messages() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(messages)").unwrap();
        let cols: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        assert!(cols.iter().any(|c| c == "speaker_slug"), "missing column 'speaker_slug' in {cols:?}");
        assert!(cols.iter().any(|c| c == "pre_turn_action_points"), "missing column 'pre_turn_action_points' in {cols:?}");
    }

    #[test]
    fn upgrade_from_v4_backfills_solo_session_characters() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (4);",
        ).unwrap();
        super::v1::migrate(&conn).unwrap();
        super::v2::migrate(&conn).unwrap();
        super::v3::migrate(&conn).unwrap();
        super::v4::migrate(&conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, character, created_at, updated_at)
             VALUES ('s-solo', 'alice', 'now', 'now')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, character, created_at, updated_at)
             VALUES ('s-bare', NULL, 'now', 'now')",
            [],
        ).unwrap();

        run_migrations(&conn).unwrap();

        let solo_attachments: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_characters WHERE session_id = 's-solo'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(solo_attachments, 1);

        let (slug, idx, talk, ap): (String, i64, f64, f64) = conn.query_row(
            "SELECT slug, attach_index, talkativeness, action_points
             FROM session_characters WHERE session_id = 's-solo'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(slug, "alice");
        assert_eq!(idx, 0);
        assert!((talk - 1.0).abs() < 1e-6);
        assert!((ap - 0.0).abs() < 1e-6);

        let bare_attachments: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_characters WHERE session_id = 's-bare'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(bare_attachments, 0);
    }

    #[test]
    fn cascade_deletes_session_characters_when_session_deleted() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points)
             VALUES ('s1', 'alice', 0, 1.0, 0.0)",
            [],
        ).unwrap();

        conn.execute("DELETE FROM sessions WHERE id='s1'", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_characters", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
