//! SQLite/SQLCipher database layer with CRUD operations for all persistent entities.

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::Connection;

use crate::error::{DbError, Result};
use libllm_core::character::CharacterCard;
use libllm_core::crypto::DerivedKey;
use libllm_core::persona::PersonaFile;
use libllm_core::session::{Node, NodeId, SaveMode, Session};
use libllm_core::system_prompt::SystemPromptFile;
use libllm_core::worldinfo::WorldBook;

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
    let mut stmt = conn.prepare(sql).map_err(|source| DbError::Query {
        context: err_owned.clone(),
        source,
    })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|source| DbError::Query {
            context: err_owned.clone(),
            source,
        })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::Sqlite)
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
        libllm_core::timed_result!(
            tracing::Level::INFO,
            "db.open",
            path = path_str.as_str(),
            encrypted = encrypted
            ; {
                let conn = Connection::open(path)
                    .map_err(|source| DbError::Query {
                        context: format!("failed to open database: {}", path.display()),
                        source,
                    })?;

                libllm_core::crypto::chmod_0600(path)
                    .map_err(|source| DbError::Io {
                        context: format!("failed to restrict permissions: {}", path.display()),
                        source,
                    })?;

                if let Some(key) = key {
                    conn.execute_batch(&key.key_pragma())
                        .map_err(|source| DbError::Query {
                            context: "failed to set database encryption key".to_owned(),
                            source,
                        })?;
                }

                conn.execute_batch("PRAGMA journal_mode = WAL;")
                    .map_err(|source| DbError::Query {
                        context: "failed to enable WAL mode".to_owned(),
                        source,
                    })?;

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
                    libllm_core::crypto::chmod_0600(&path_wal).map_err(|source| DbError::Io {
                        context: format!(
                            "failed to restrict permissions: {}",
                            path_wal.display()
                        ),
                        source,
                    })?;
                }
                if path_shm.exists() {
                    libllm_core::crypto::chmod_0600(&path_shm).map_err(|source| DbError::Io {
                        context: format!(
                            "failed to restrict permissions: {}",
                            path_shm.display()
                        ),
                        source,
                    })?;
                }

                conn.execute_batch("PRAGMA foreign_keys = ON;")
                    .map_err(|source| DbError::Query {
                        context: "failed to enable foreign keys".to_owned(),
                        source,
                    })?;

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

    pub fn upsert_message(&mut self, session_id: &str, node: &Node) -> Result<()> {
        sessions::upsert_message(&mut self.conn, session_id, node)
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
        libllm_core::timed_result!(tracing::Level::INFO, "db.rekey", ; {
            self.conn
                .execute_batch(&new_key.rekey_pragma())
                .map_err(|source| DbError::Query {
                    context: "failed to rekey database".to_owned(),
                    source,
                })?;
            Ok(())
        })
    }

    pub fn session_exists(&self, id: &str) -> Result<bool> {
        sessions::session_exists(&self.conn, id)
    }

    pub fn session_ids_matching_display_name(&self, substring: &str) -> Result<Vec<String>> {
        sessions::ids_matching_display_name(&self.conn, substring)
    }

    /// Execute a single SQL statement that returns rows.
    /// Errors propagate the underlying rusqlite error verbatim, including
    /// `attempt to write a readonly database` when called on a connection
    /// opened with `PRAGMA query_only = ON`.
    pub fn execute_query(&self, sql: &str) -> Result<QueryRows> {
        let mut stmt = self.conn.prepare(sql).map_err(|source| DbError::Query {
            context: "failed to prepare query".to_owned(),
            source,
        })?;
        let headers: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
        let column_count = headers.len();
        let mut rows = Vec::new();
        let mut cursor = stmt.query([]).map_err(|source| DbError::Query {
            context: "failed to execute query".to_owned(),
            source,
        })?;
        while let Some(row) = cursor.next().map_err(|source| DbError::Query {
            context: "failed to read row".to_owned(),
            source,
        })? {
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
        self.conn.execute(sql, []).map_err(|source| DbError::Query {
            context: "failed to execute statement".to_owned(),
            source,
        })
    }

    /// Number of rows affected by the most recent INSERT/UPDATE/DELETE
    /// on this connection. Returns 0 for statements that did not modify rows
    /// (including SELECT, PRAGMA, schema changes, or no statement at all).
    pub fn changes(&self) -> u64 {
        self.conn.changes()
    }

    /// Expose the raw connection for crate-internal modules that build
    /// dynamic SQL outside the typed Database methods (e.g., the search executor).
    pub(crate) fn with_connection<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        f(&self.conn)
    }

    /// Run one or more SQL statements, discarding any returned rows.
    /// Use for pragma-like operations and SQLCipher control statements
    /// (ATTACH ... KEY, SELECT sqlcipher_export, DETACH) where the result
    /// set is unused but the side effect is needed.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.conn
            .execute_batch(sql)
            .map_err(|source| DbError::Query {
                context: "failed to execute batch".to_owned(),
                source,
            })
    }

    pub fn is_template_dismissed(&self, template_hash: &str) -> Result<bool> {
        dismissed_templates::is_dismissed(&self.conn, template_hash)
    }

    pub fn record_template_dismissal(&self, template_hash: &str) -> Result<()> {
        dismissed_templates::record_dismissal(&self.conn, template_hash)
    }

    pub fn clear_dismissed_templates(&self) -> Result<u64> {
        dismissed_templates::clear_all(&self.conn)
    }

    fn purge_table_in_txn(&self, table_name: &'static str, sql: &'static str) -> Result<u64> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|source| DbError::Query {
                context: format!("{table_name}: begin txn"),
                source,
            })?;
        let affected = self.conn.execute(sql, []).map_err(|source| DbError::Query {
            context: format!("failed to purge {table_name}"),
            source,
        });
        match affected {
            Ok(n) => {
                if let Err(commit_err) = self.conn.execute_batch("COMMIT") {
                    if let Err(rollback_err) = self.conn.execute_batch("ROLLBACK") {
                        tracing::warn!(
                            result = "error",
                            table = table_name,
                            error = %rollback_err,
                            "db.purge.rollback_failed"
                        );
                    }
                    return Err(DbError::Query {
                        context: format!("{table_name}: commit txn"),
                        source: commit_err,
                    });
                }
                Ok(n as u64)
            }
            Err(err) => {
                if let Err(rollback_err) = self.conn.execute_batch("ROLLBACK") {
                    tracing::warn!(
                        result = "error",
                        table = table_name,
                        error = %rollback_err,
                        "db.purge.rollback_failed"
                    );
                }
                Err(err)
            }
        }
    }

    pub fn purge_sessions(&self) -> Result<u64> {
        self.purge_table_in_txn("sessions", "DELETE FROM sessions")
    }

    pub fn purge_characters(&self) -> Result<u64> {
        self.purge_table_in_txn("characters", "DELETE FROM characters")
    }

    pub fn purge_personas(&self) -> Result<u64> {
        self.purge_table_in_txn("personas", "DELETE FROM personas")
    }

    pub fn purge_worldbooks(&self) -> Result<u64> {
        self.purge_table_in_txn("worldbooks", "DELETE FROM worldbooks")
    }
}

/// Persists `session` according to `mode`, writing to `db` only when the mode is
/// [`SaveMode::Database`]. Non-persisting modes succeed without touching storage.
pub fn save_session_for_mode(
    mode: &SaveMode,
    session: &Session,
    db: Option<&mut Database>,
) -> Result<()> {
    match mode {
        SaveMode::None | SaveMode::PendingPasskey { .. } => Ok(()),
        SaveMode::Database { id } => {
            let db = db.ok_or(DbError::DatabaseNotAvailable)?;
            db.save_session(id, session)
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::Database;
    use libllm_core::crypto::{derive_key, load_or_create_salt};
    use libllm_core::session::{Message, NodeId, Role, Session};

    fn make_key(dir: &TempDir) -> libllm_core::crypto::DerivedKey {
        let salt_path = dir.path().join(".salt");
        let salt = load_or_create_salt(&salt_path).unwrap();
        derive_key("test-passkey", &salt).unwrap()
    }

    struct BranchingIds {
        branch_parent: NodeId,
        branch_leaf: NodeId,
    }

    fn build_branching_session() -> (Session, BranchingIds) {
        let mut session = Session::default();
        let root = session
            .tree
            .push(None, Message::new(Role::User, "root".to_owned()));
        let intro = session.tree.push(
            Some(root),
            Message::new(Role::Assistant, "intro".to_owned()),
        );
        let branch_parent = session.tree.push(
            Some(intro),
            Message::new(Role::User, "branch here".to_owned()),
        );
        session.tree.push(
            Some(branch_parent),
            Message::new(Role::Assistant, "left branch".to_owned()),
        );
        session.tree.set_head(Some(branch_parent));

        let right_branch = session.tree.push(
            Some(branch_parent),
            Message::new(Role::Assistant, "right branch".to_owned()),
        );
        let right_user = session.tree.push(
            Some(right_branch),
            Message::new(Role::User, "right follow-up".to_owned()),
        );
        let branch_leaf = session.tree.push(
            Some(right_user),
            Message::new(Role::Assistant, "right leaf".to_owned()),
        );

        (
            session,
            BranchingIds {
                branch_parent,
                branch_leaf,
            },
        )
    }

    #[test]
    fn persists_preferred_branch_choices_via_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let mut db = Database::open(&db_path, None).unwrap();

        let (session, ids) = build_branching_session();
        db.insert_session("branch-test", &session).unwrap();
        let mut loaded = db.load_session("branch-test").unwrap();

        loaded.tree.switch_to(ids.branch_parent);
        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }

    #[test]
    fn encrypted_db_load_rehydrates_preferred_branch_choices() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("encrypted.db");
        let key = make_key(&dir);

        let (session, ids) = build_branching_session();
        let mut db = Database::open(&db_path, Some(&key)).unwrap();
        db.insert_session("enc-branch-test", &session).unwrap();
        drop(db);

        let db = Database::open(&db_path, Some(&key)).unwrap();
        let mut loaded = db.load_session("enc-branch-test").unwrap();

        loaded.tree.switch_to(ids.branch_parent);
        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
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

    #[test]
    fn purge_table_in_txn_rollback_on_commit_failure() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path, None).unwrap();

        // Add a deferred FK that makes COMMIT fail when personas rows are deleted
        // (FK is DEFERRABLE INITIALLY DEFERRED so it passes at DELETE time but
        // fires at COMMIT).
        db.conn()
            .execute_batch(
                "CREATE TABLE blocker (
                    id INTEGER PRIMARY KEY,
                    persona_slug TEXT NOT NULL
                        REFERENCES personas(slug)
                        DEFERRABLE INITIALLY DEFERRED
                );",
            )
            .unwrap();

        db.execute_statement(
            "INSERT INTO personas (slug, name, persona, created_at, updated_at) \
             VALUES ('alice', 'Alice', 'curious', 'now', 'now')",
        )
        .unwrap();
        db.conn()
            .execute_batch("INSERT INTO blocker (id, persona_slug) VALUES (1, 'alice')")
            .unwrap();

        // purge_personas issues BEGIN IMMEDIATE + DELETE FROM personas + COMMIT.
        // The COMMIT will fail due to the deferred FK from blocker.
        let result = db.purge_personas();
        assert!(result.is_err(), "expected COMMIT failure to surface as Err");

        // The connection must be usable after the failed purge — no open transaction.
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM personas", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "personas row must still exist after rolled-back purge"
        );

        // A subsequent purge call must not fail with 'cannot start a transaction
        // within a transaction'.
        let result2 = db.purge_personas();
        let err_msg = result2.unwrap_err().to_string();
        assert!(
            !err_msg.contains("within a transaction"),
            "connection left in open transaction: {err_msg}"
        );
    }
}
