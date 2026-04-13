use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::crypto::DerivedKey;

mod schema;

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

    pub fn conn(&self) -> &Connection {
        &self.conn
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
