//! v6: per-session and per-character author's note with injection depth and position.
//!
//! Each column is added independently so a partially-applied intermediate build
//! (e.g. only `author_note` present) still receives the missing depth/position
//! columns. v11 re-runs the same heal for databases already stamped past v6.

use rusqlite::Connection;

use crate::error::{DbError, Result};

pub(super) fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let pragma = format!("PRAGMA table_info({table})");
    conn.prepare(&pragma)
        .map_err(|source| DbError::Query {
            context: format!("failed to prepare PRAGMA table_info({table})"),
            source,
        })?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| DbError::Query {
            context: format!("failed to query {table} columns"),
            source,
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|source| DbError::Query {
            context: format!("failed to collect {table} columns"),
            source,
        })
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    columns: &[String],
    name: &str,
    ddl: &str,
) -> Result<()> {
    if columns.iter().any(|c| c == name) {
        return Ok(());
    }
    conn.execute(ddl, []).map_err(|source| DbError::Query {
        context: format!("failed to add {name} column to {table}"),
        source,
    })?;
    Ok(())
}

/// Ensures sessions and characters each have the full author-note column set.
/// Safe to re-run: existing columns are left alone; only missing ones are added.
pub(super) fn ensure_author_note_columns(conn: &Connection) -> Result<()> {
    let sessions_cols = table_columns(conn, "sessions")?;
    ensure_column(
        conn,
        "sessions",
        &sessions_cols,
        "author_note",
        "ALTER TABLE sessions ADD COLUMN author_note TEXT",
    )?;
    ensure_column(
        conn,
        "sessions",
        &sessions_cols,
        "author_note_depth",
        "ALTER TABLE sessions ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4",
    )?;
    ensure_column(
        conn,
        "sessions",
        &sessions_cols,
        "author_note_at_top",
        "ALTER TABLE sessions ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0",
    )?;

    let chars_cols = table_columns(conn, "characters")?;
    ensure_column(
        conn,
        "characters",
        &chars_cols,
        "author_note",
        "ALTER TABLE characters ADD COLUMN author_note TEXT",
    )?;
    ensure_column(
        conn,
        "characters",
        &chars_cols,
        "author_note_depth",
        "ALTER TABLE characters ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4",
    )?;
    ensure_column(
        conn,
        "characters",
        &chars_cols,
        "author_note_at_top",
        "ALTER TABLE characters ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0",
    )?;

    Ok(())
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.migrate", phase = "v6" ; {
        ensure_author_note_columns(conn)
    })
}
