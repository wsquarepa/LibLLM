//! Worldbook CRUD operations with JSON-serialized entry storage.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::session::now_iso8601;
use crate::worldinfo::WorldBook;

pub fn insert_worldbook(conn: &Connection, slug: &str, book: &WorldBook) -> Result<()> {
    let entry_count = book.entries.len();
    crate::timed_result!(
        tracing::Level::INFO,
        "db.worldbook.insert",
        slug = slug,
        entry_count = entry_count
        ; {
            let now = now_iso8601();
            let entries = serde_json::to_string(&book.entries)
                .context("failed to serialize worldbook entries")?;
            conn.execute(
                "INSERT INTO worldbooks (slug, name, entries, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![slug, book.name, entries, now, now],
            )
            .context("failed to insert worldbook")?;
            Ok(())
        }
    )
}

pub fn load_worldbook(conn: &Connection, slug: &str) -> Result<WorldBook> {
    crate::timed_result!(tracing::Level::INFO, "db.worldbook.load", slug = slug ; {
        let book = conn
            .query_row(
                "SELECT name, entries FROM worldbooks WHERE slug = ?1",
                params![slug],
                |row| {
                    let name: String = row.get(0)?;
                    let entries_json: String = row.get(1)?;
                    Ok((name, entries_json))
                },
            )
            .with_context(|| format!("worldbook not found: {slug}"))
            .and_then(|(name, entries_json)| {
                let entries = serde_json::from_str(&entries_json)
                    .context("failed to deserialize worldbook entries")?;
                Ok(WorldBook { name, entries })
            })?;
        tracing::info!(slug = slug, entry_count = book.entries.len(), "db.worldbook.load");
        Ok(book)
    })
}

pub fn list_worldbooks(conn: &Connection) -> Result<Vec<(String, String)>> {
    crate::timed_result!(tracing::Level::INFO, "db.worldbook.list", ; {
        let entries = super::query_slug_name_pairs(
            conn,
            "SELECT slug, name FROM worldbooks ORDER BY name",
            "failed to list worldbooks",
        )?;
        tracing::info!(count = entries.len(), "db.worldbook.list");
        Ok(entries)
    })
}

pub fn update_worldbook(conn: &Connection, slug: &str, book: &WorldBook) -> Result<()> {
    let entry_count = book.entries.len();
    crate::timed_result!(
        tracing::Level::INFO,
        "db.worldbook.update",
        slug = slug,
        entry_count = entry_count
        ; {
            let now = now_iso8601();
            let entries = serde_json::to_string(&book.entries)
                .context("failed to serialize worldbook entries")?;
            let affected = conn
                .execute(
                    "UPDATE worldbooks SET name = ?1, entries = ?2, updated_at = ?3 WHERE slug = ?4",
                    params![book.name, entries, now, slug],
                )
                .context("failed to update worldbook")?;
            tracing::info!(slug = slug, affected = affected, "db.worldbook.update");
            if affected == 0 {
                anyhow::bail!("worldbook not found: {slug}");
            }
            Ok(())
        }
    )
}

pub fn delete_worldbook(conn: &Connection, slug: &str) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.worldbook.delete", slug = slug ; {
        let affected = conn
            .execute("DELETE FROM worldbooks WHERE slug = ?1", params![slug])
            .context("failed to delete worldbook")?;
        tracing::info!(slug = slug, affected = affected, "db.worldbook.delete");
        if affected == 0 {
            anyhow::bail!("worldbook not found: {slug}");
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::db::schema::run_migrations;
    use crate::worldinfo::{Entry, WorldBook};

    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn make_worldbook() -> WorldBook {
        WorldBook {
            name: "Test Lore".to_owned(),
            entries: vec![Entry {
                keys: vec!["dragon".to_owned()],
                secondary_keys: vec!["fire".to_owned()],
                selective: true,
                content: "Dragons breathe fire.".to_owned(),
                constant: false,
                enabled: true,
                order: 10,
                depth: 4,
                case_sensitive: false,
            }],
        }
    }

    #[test]
    fn worldbook_round_trip() {
        let conn = setup_db();
        let book = make_worldbook();

        insert_worldbook(&conn, "test-lore", &book).unwrap();
        let loaded = load_worldbook(&conn, "test-lore").unwrap();

        assert_eq!(loaded.name, book.name);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0], book.entries[0]);
    }

    #[test]
    fn list_worldbooks_ordering() {
        let conn = setup_db();

        let mut book_z = make_worldbook();
        book_z.name = "Zetton Lore".to_owned();
        let mut book_a = make_worldbook();
        book_a.name = "Alpha Lore".to_owned();

        insert_worldbook(&conn, "zetton-lore", &book_z).unwrap();
        insert_worldbook(&conn, "alpha-lore", &book_a).unwrap();

        let list = list_worldbooks(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("alpha-lore".to_owned(), "Alpha Lore".to_owned()));
        assert_eq!(list[1], ("zetton-lore".to_owned(), "Zetton Lore".to_owned()));
    }

    #[test]
    fn update_and_delete_worldbook() {
        let conn = setup_db();
        let book = make_worldbook();
        insert_worldbook(&conn, "test-lore", &book).unwrap();

        let mut updated = book.clone();
        updated.name = "Updated Lore".to_owned();
        updated.entries = vec![];
        update_worldbook(&conn, "test-lore", &updated).unwrap();

        let loaded = load_worldbook(&conn, "test-lore").unwrap();
        assert_eq!(loaded.name, "Updated Lore");
        assert!(loaded.entries.is_empty());

        delete_worldbook(&conn, "test-lore").unwrap();
        assert!(load_worldbook(&conn, "test-lore").is_err());
    }
}
