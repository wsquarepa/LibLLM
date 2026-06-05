//! Persona profile CRUD operations against the SQLite personas table.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::persona::PersonaFile;
use crate::session::now_iso8601;

pub fn insert_persona(conn: &Connection, slug: &str, persona: &PersonaFile) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.persona.insert", slug = slug ; {
        let now = now_iso8601();
        conn.execute(
            "INSERT INTO personas (slug, name, persona, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![slug, persona.name, persona.persona, now, now],
        )
        .context("failed to insert persona")?;
        Ok(())
    })
}

pub fn load_persona(conn: &Connection, slug: &str) -> Result<PersonaFile> {
    crate::timed_result!(tracing::Level::INFO, "db.persona.load", slug = slug ; {
        conn.query_row(
            "SELECT name, persona FROM personas WHERE slug = ?1",
            params![slug],
            |row| {
                let name: String = row.get(0)?;
                let persona: String = row.get(1)?;
                Ok(PersonaFile { name, persona })
            },
        )
        .with_context(|| format!("persona not found: {slug}"))
    })
}

pub fn list_personas(conn: &Connection) -> Result<Vec<(String, String)>> {
    crate::timed_result!(tracing::Level::INFO, "db.persona.list", ; {
        let entries = super::query_slug_name_pairs(
            conn,
            "SELECT slug, name FROM personas ORDER BY name",
            "failed to list personas",
        )?;
        tracing::info!(count = entries.len(), "db.persona.list");
        Ok(entries)
    })
}

pub fn update_persona(conn: &Connection, slug: &str, persona: &PersonaFile) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.persona.update", slug = slug ; {
        let now = now_iso8601();
        let affected = conn
            .execute(
                "UPDATE personas SET name = ?1, persona = ?2, updated_at = ?3 WHERE slug = ?4",
                params![persona.name, persona.persona, now, slug],
            )
            .context("failed to update persona")?;
        tracing::info!(slug = slug, affected = affected, "db.persona.update");
        if affected == 0 {
            anyhow::bail!("persona not found: {slug}");
        }
        Ok(())
    })
}

pub fn delete_persona(conn: &Connection, slug: &str) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.persona.delete", slug = slug ; {
        let affected = conn
            .execute("DELETE FROM personas WHERE slug = ?1", params![slug])
            .context("failed to delete persona")?;
        tracing::info!(slug = slug, affected = affected, "db.persona.delete");
        if affected == 0 {
            anyhow::bail!("persona not found: {slug}");
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::db::migrations::run_migrations;
    use crate::persona::PersonaFile;

    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn persona_round_trip() {
        let conn = setup_db();
        let persona = PersonaFile {
            name: "Alice".to_owned(),
            persona: "A curious explorer.".to_owned(),
        };

        insert_persona(&conn, "alice", &persona).unwrap();
        let loaded = load_persona(&conn, "alice").unwrap();

        assert_eq!(loaded.name, persona.name);
        assert_eq!(loaded.persona, persona.persona);
    }

    #[test]
    fn list_personas_ordering() {
        let conn = setup_db();

        let persona_z = PersonaFile {
            name: "Zara".to_owned(),
            persona: "A wise sage.".to_owned(),
        };
        let persona_a = PersonaFile {
            name: "Alice".to_owned(),
            persona: "A curious explorer.".to_owned(),
        };

        insert_persona(&conn, "zara", &persona_z).unwrap();
        insert_persona(&conn, "alice", &persona_a).unwrap();

        let list = list_personas(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("alice".to_owned(), "Alice".to_owned()));
        assert_eq!(list[1], ("zara".to_owned(), "Zara".to_owned()));
    }

    #[test]
    fn update_and_delete_persona() {
        let conn = setup_db();
        let persona = PersonaFile {
            name: "Alice".to_owned(),
            persona: "A curious explorer.".to_owned(),
        };
        insert_persona(&conn, "alice", &persona).unwrap();

        let updated = PersonaFile {
            name: "Alice Updated".to_owned(),
            persona: "A seasoned adventurer.".to_owned(),
        };
        update_persona(&conn, "alice", &updated).unwrap();

        let loaded = load_persona(&conn, "alice").unwrap();
        assert_eq!(loaded.name, "Alice Updated");
        assert_eq!(loaded.persona, "A seasoned adventurer.");

        delete_persona(&conn, "alice").unwrap();
        assert!(load_persona(&conn, "alice").is_err());
    }
}
