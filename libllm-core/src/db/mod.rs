use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::character::CharacterCard;
use crate::crypto::DerivedKey;
use crate::persona::PersonaFile;
use crate::session::{Node, NodeId, Session};
use crate::system_prompt::SystemPromptFile;
use crate::worldinfo::WorldBook;

mod characters;
mod personas;
mod prompts;
mod schema;
mod sessions;
mod worldbooks;

pub use prompts::PromptListEntry;
pub use sessions::SessionListEntry;

fn query_slug_name_pairs(conn: &Connection, sql: &str, err_context: &str) -> Result<Vec<(String, String)>> {
    let err_owned = err_context.to_owned();
    let mut stmt = conn.prepare(sql).with_context(|| err_owned.clone())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .with_context(|| err_owned.clone())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| anyhow::anyhow!(e))
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path, key: Option<&DerivedKey>) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database: {}", path.display()))?;

        if let Some(key) = key {
            let hex_key = key.hex();
            conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex_key))
                .context("failed to set database encryption key")?;
        }

        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .context("failed to enable WAL mode")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to enable foreign keys")?;

        schema::run_migrations(&conn)?;

        Ok(Self { conn })
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

    pub fn insert_prompt(&self, slug: &str, prompt: &SystemPromptFile, builtin: bool) -> Result<()> {
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
        let hex_key = new_key.hex();
        self.conn
            .execute_batch(&format!("PRAGMA rekey = \"x'{}'\";", hex_key))
            .context("failed to rekey database")?;
        Ok(())
    }

    pub fn session_exists(&self, id: &str) -> Result<bool> {
        sessions::session_exists(&self.conn, id)
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
    fn open_creates_new_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        let db = Database::open(&db_path, None).unwrap();

        let version: i64 = db
            .conn()
            .query_row(
                "SELECT MAX(version) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
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
                    rusqlite::params!["test-session-id", "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"],
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
                    rusqlite::params!["plain-session-id", "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"],
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
}
