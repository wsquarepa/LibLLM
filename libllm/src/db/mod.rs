//! SQLite/SQLCipher database layer with CRUD operations for all persistent entities.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::character::CharacterCard;
use crate::crypto::DerivedKey;
use crate::persona::PersonaFile;
use crate::session::{Node, NodeId, Session};
use crate::system_prompt::SystemPromptFile;
use crate::worldinfo::WorldBook;

mod characters;
mod dismissed_templates;
pub mod file_summaries;
pub mod migrations;
mod personas;
mod prompts;
mod sessions;
mod worldbooks;

pub use dismissed_templates::{
    clear_all as clear_dismissed_templates, is_dismissed as is_template_dismissed,
    record_dismissal as record_template_dismissal,
};
pub use file_summaries::{FileSummaryRow, FileSummaryStatus};
pub use migrations::CURRENT_VERSION;
pub use prompts::PromptListEntry;
pub use sessions::SessionListEntry;

static CIPHER_LOG_SUPPRESSED: OnceLock<()> = OnceLock::new();

/// Silence SQLCipher's default stderr diagnostics for this process.
///
/// SQLCipher writes ERROR/WARN-level messages (e.g. `hmac check failed` on
/// wrong-passkey attempts) to `stderr` through its own log sink, independent
/// of SQLite's `sqlite3_log` callback. In the TUI that corrupts the screen.
/// The log target is a process-wide static, so a single `PRAGMA cipher_log`
/// on any connection silences every subsequent SQLCipher operation.
///
/// Idempotent; only the first call performs the PRAGMA.
pub fn suppress_sqlcipher_log() {
    CIPHER_LOG_SUPPRESSED.get_or_init(|| {
        let result = Connection::open_in_memory()
            .and_then(|conn| conn.execute_batch("PRAGMA cipher_log = off;"));
        if let Err(err) = result {
            tracing::warn!(
                phase = "suppress",
                status = "error",
                error = %err,
                "sqlcipher.log",
            );
        }
    });
}

fn query_slug_name_pairs(
    conn: &Connection,
    sql: &str,
    err_context: &str,
) -> Result<Vec<(String, String)>> {
    let err_owned = err_context.to_owned();
    let mut stmt = conn.prepare(sql).with_context(|| err_owned.clone())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .with_context(|| err_owned.clone())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!(e))
}

/// Result set returned by `Database::execute_query`.
pub struct QueryRows {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<rusqlite::types::Value>>,
}

/// Handle to an open SQLite/SQLCipher database with methods for all persistent entity operations.
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path, key: Option<&DerivedKey>) -> Result<Self> {
        suppress_sqlcipher_log();
        let encrypted = key.is_some();
        let path_str = path.display().to_string();
        crate::timed_result!(
            tracing::Level::INFO,
            "db.open",
            path = path_str.as_str(),
            encrypted = encrypted
            ; {
                let conn = Connection::open(path)
                    .with_context(|| format!("failed to open database: {}", path.display()))?;

                crate::crypto::chmod_0600(path)
                    .with_context(|| format!("failed to restrict permissions: {}", path.display()))?;

                if let Some(key) = key {
                    conn.execute_batch(&key.key_pragma())
                        .context("failed to set database encryption key")?;
                }

                conn.execute_batch("PRAGMA journal_mode = WAL;")
                    .context("failed to enable WAL mode")?;

                let path_wal = {
                    let mut s = path.as_os_str().to_owned();
                    s.push("-wal");
                    std::path::PathBuf::from(s)
                };
                let path_shm = {
                    let mut s = path.as_os_str().to_owned();
                    s.push("-shm");
                    std::path::PathBuf::from(s)
                };
                if path_wal.exists() {
                    crate::crypto::chmod_0600(&path_wal).with_context(|| {
                        format!("failed to restrict permissions: {}", path_wal.display())
                    })?;
                }
                if path_shm.exists() {
                    crate::crypto::chmod_0600(&path_shm).with_context(|| {
                        format!("failed to restrict permissions: {}", path_shm.display())
                    })?;
                }

                conn.execute_batch("PRAGMA foreign_keys = ON;")
                    .context("failed to enable foreign keys")?;

                migrations::run_migrations(&conn)?;

                Ok(Self { conn })
            }
        )
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn insert_session(&mut self, id: &str, session: &Session) -> Result<()> {
        sessions::insert_session(&mut self.conn, id, session)
    }

    pub fn save_session(&mut self, id: &str, session: &Session) -> Result<()> {
        sessions::save_session(&mut self.conn, id, session)
    }

    pub fn load_session(&self, id: &str) -> Result<Session> {
        sessions::load_session(&self.conn, id)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionListEntry>> {
        sessions::list_sessions(&self.conn)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        sessions::delete_session(&self.conn, id)
    }

    pub fn upsert_message(&self, session_id: &str, node: &Node) -> Result<()> {
        sessions::upsert_message(&self.conn, session_id, node)
    }

    pub fn update_head(&self, session_id: &str, head_id: Option<NodeId>) -> Result<()> {
        sessions::update_head(&self.conn, session_id, head_id)
    }

    pub fn update_preferred_child(
        &self,
        session_id: &str,
        parent_id: NodeId,
        child_id: NodeId,
    ) -> Result<()> {
        sessions::update_preferred_child(&self.conn, session_id, parent_id, child_id)
    }

    pub fn insert_character(&self, slug: &str, card: &CharacterCard) -> Result<()> {
        characters::insert_character(&self.conn, slug, card)
    }

    pub fn load_character(&self, slug: &str) -> Result<CharacterCard> {
        characters::load_character(&self.conn, slug)
    }

    pub fn list_characters(&self) -> Result<Vec<(String, String)>> {
        characters::list_characters(&self.conn)
    }

    pub fn update_character(&self, slug: &str, card: &CharacterCard) -> Result<()> {
        characters::update_character(&self.conn, slug, card)
    }

    pub fn delete_character(&self, slug: &str) -> Result<()> {
        characters::delete_character(&self.conn, slug)
    }

    pub fn insert_worldbook(&self, slug: &str, book: &WorldBook) -> Result<()> {
        worldbooks::insert_worldbook(&self.conn, slug, book)
    }

    pub fn load_worldbook(&self, slug: &str) -> Result<WorldBook> {
        worldbooks::load_worldbook(&self.conn, slug)
    }

    pub fn list_worldbooks(&self) -> Result<Vec<(String, String)>> {
        worldbooks::list_worldbooks(&self.conn)
    }

    pub fn update_worldbook(&self, slug: &str, book: &WorldBook) -> Result<()> {
        worldbooks::update_worldbook(&self.conn, slug, book)
    }

    pub fn delete_worldbook(&self, slug: &str) -> Result<()> {
        worldbooks::delete_worldbook(&self.conn, slug)
    }

    pub fn insert_prompt(
        &self,
        slug: &str,
        prompt: &SystemPromptFile,
        builtin: bool,
    ) -> Result<()> {
        prompts::insert_prompt(&self.conn, slug, prompt, builtin)
    }

    pub fn load_prompt(&self, slug: &str) -> Result<SystemPromptFile> {
        prompts::load_prompt(&self.conn, slug)
    }

    pub fn list_prompts(&self) -> Result<Vec<PromptListEntry>> {
        prompts::list_prompts(&self.conn)
    }

    pub fn update_prompt(&self, slug: &str, prompt: &SystemPromptFile) -> Result<()> {
        prompts::update_prompt(&self.conn, slug, prompt)
    }

    pub fn rename_prompt(
        &self,
        old_slug: &str,
        new_slug: &str,
        prompt: &SystemPromptFile,
    ) -> Result<()> {
        prompts::rename_prompt(&self.conn, old_slug, new_slug, prompt)
    }

    pub fn delete_prompt(&self, slug: &str) -> Result<()> {
        prompts::delete_prompt(&self.conn, slug)
    }

    pub fn ensure_builtin_prompts(&self) -> Result<()> {
        prompts::ensure_builtins(&self.conn)
    }

    pub fn insert_persona(&self, slug: &str, persona: &PersonaFile) -> Result<()> {
        personas::insert_persona(&self.conn, slug, persona)
    }

    pub fn load_persona(&self, slug: &str) -> Result<PersonaFile> {
        personas::load_persona(&self.conn, slug)
    }

    pub fn list_personas(&self) -> Result<Vec<(String, String)>> {
        personas::list_personas(&self.conn)
    }

    pub fn update_persona(&self, slug: &str, persona: &PersonaFile) -> Result<()> {
        personas::update_persona(&self.conn, slug, persona)
    }

    pub fn delete_persona(&self, slug: &str) -> Result<()> {
        personas::delete_persona(&self.conn, slug)
    }

    pub fn rekey(&self, new_key: &DerivedKey) -> Result<()> {
        crate::timed_result!(tracing::Level::INFO, "db.rekey", ; {
            self.conn
                .execute_batch(&new_key.rekey_pragma())
                .context("failed to rekey database")?;
            Ok(())
        })
    }

    pub fn session_exists(&self, id: &str) -> Result<bool> {
        sessions::session_exists(&self.conn, id)
    }

    /// Execute a single SQL statement that returns rows.
    /// Errors propagate the underlying rusqlite error verbatim, including
    /// `attempt to write a readonly database` when called on a connection
    /// opened with `PRAGMA query_only = ON`.
    pub fn execute_query(&self, sql: &str) -> Result<QueryRows> {
        let mut stmt = self.conn.prepare(sql).context("failed to prepare query")?;
        let headers: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
        let column_count = headers.len();
        let mut rows = Vec::new();
        let mut cursor = stmt.query([]).context("failed to execute query")?;
        while let Some(row) = cursor.next().context("failed to read row")? {
            let mut values = Vec::with_capacity(column_count);
            for idx in 0..column_count {
                values.push(row.get::<_, rusqlite::types::Value>(idx)?);
            }
            rows.push(values);
        }
        Ok(QueryRows { headers, rows })
    }

    /// Execute a single SQL statement that does not return rows.
    /// Returns the number of affected rows.
    pub fn execute_statement(&self, sql: &str) -> Result<usize> {
        self.conn
            .execute(sql, [])
            .context("failed to execute statement")
    }

    /// Number of rows affected by the most recent INSERT/UPDATE/DELETE
    /// on this connection. Returns 0 for statements that did not modify rows
    /// (including SELECT, PRAGMA, schema changes, or no statement at all).
    pub fn changes(&self) -> u64 {
        self.conn.changes()
    }

    /// Run one or more SQL statements, discarding any returned rows.
    /// Use for pragma-like operations and SQLCipher control statements
    /// (ATTACH ... KEY, SELECT sqlcipher_export, DETACH) where the result
    /// set is unused but the side effect is needed.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.conn
            .execute_batch(sql)
            .context("failed to execute batch")
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::Database;
    use crate::crypto::{derive_key, load_or_create_salt};

    fn make_key(dir: &TempDir) -> crate::crypto::DerivedKey {
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).unwrap();
        derive_key("test-passkey", &salt).unwrap()
    }

    #[test]
    fn execute_query_returns_rows_and_headers() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("plain.db");
        let db = Database::open(&db_path, None).unwrap();

        db.execute_statement(
            "INSERT INTO personas (slug, name, persona, created_at, updated_at) \
             VALUES ('alice', 'Alice', 'curious', '2026-04-17T00:00:00Z', '2026-04-17T00:00:00Z')",
        )
        .unwrap();

        let rows = db
            .execute_query("SELECT slug, name FROM personas ORDER BY slug")
            .unwrap();

        assert_eq!(rows.headers, vec!["slug".to_owned(), "name".to_owned()]);
        assert_eq!(rows.rows.len(), 1);
        let first = &rows.rows[0];
        assert_eq!(first.len(), 2);
        match (&first[0], &first[1]) {
            (rusqlite::types::Value::Text(s), rusqlite::types::Value::Text(n)) => {
                assert_eq!(s, "alice");
                assert_eq!(n, "Alice");
            }
            other => panic!("unexpected row values: {other:?}"),
        }
    }

    #[test]
    fn execute_statement_returns_affected_row_count() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("plain.db");
        let db = Database::open(&db_path, None).unwrap();

        let inserted = db
            .execute_statement(
                "INSERT INTO personas (slug, name, persona, created_at, updated_at) \
                 VALUES ('bob', 'Bob', 'wise', '2026-04-17T00:00:00Z', '2026-04-17T00:00:00Z')",
            )
            .unwrap();
        assert_eq!(inserted, 1);
    }

    #[test]
    fn open_creates_new_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        let db = Database::open(&db_path, None).unwrap();

        let version: i64 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, super::migrations::CURRENT_VERSION);
    }

    #[test]
    fn open_encrypted_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("encrypted.db");
        let key = make_key(&dir);

        {
            let db = Database::open(&db_path, Some(&key)).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        "test-session-id",
                        "2026-01-01T00:00:00Z",
                        "2026-01-01T00:00:00Z"
                    ],
                )
                .unwrap();
        }

        {
            let db = Database::open(&db_path, Some(&key)).unwrap();
            let id: String = db
                .conn()
                .query_row(
                    "SELECT id FROM sessions WHERE id = ?1",
                    rusqlite::params!["test-session-id"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(id, "test-session-id");
        }

        {
            let result = Database::open(&db_path, None);
            assert!(
                result.is_err(),
                "opening encrypted database without key should fail"
            );
        }
    }

    #[test]
    fn open_unencrypted_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("plain.db");

        {
            let db = Database::open(&db_path, None).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        "plain-session-id",
                        "2026-01-01T00:00:00Z",
                        "2026-01-01T00:00:00Z"
                    ],
                )
                .unwrap();
        }

        {
            let db = Database::open(&db_path, None).unwrap();
            let id: String = db
                .conn()
                .query_row(
                    "SELECT id FROM sessions WHERE id = ?1",
                    rusqlite::params!["plain-session-id"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(id, "plain-session-id");
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_restricts_database_file_to_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("perms.db");

        let _db = Database::open(&db_path, None).unwrap();

        let mode = std::fs::metadata(&db_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "database file must be owner read/write only"
        );
    }

}
