//! v1: initial schema — sessions, messages, characters, worldbooks, prompts, personas.

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
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
