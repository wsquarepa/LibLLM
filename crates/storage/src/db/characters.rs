//! Character card CRUD operations against the SQLite characters table.

use rusqlite::{Connection, params};

use crate::error::{DbError, Result};
use libllm_core::character::CharacterCard;
use libllm_core::session::now_iso8601;

/// Column tuple selected by `load_character`, in `SELECT` order. The trailing
/// three columns are the author-note text, depth, and at-top flag.
type CharacterRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
);

fn author_note_columns(card: &CharacterCard) -> (Option<&str>, i64, i64) {
    match card.author_note.as_ref() {
        Some(note) => (
            Some(note.text.as_str()),
            note.depth as i64,
            note.at_top as i64,
        ),
        None => (None, libllm_core::author_note::DEFAULT_DEPTH as i64, 0),
    }
}

pub fn insert_character(conn: &Connection, slug: &str, card: &CharacterCard) -> Result<()> {
    let alternate_greetings_count = card.alternate_greetings.len();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "db.character.insert",
        slug = slug,
        alternate_greetings_count = alternate_greetings_count
        ; {
            let now = now_iso8601();
            let alternate_greetings =
                serde_json::to_string(&card.alternate_greetings)
                    .map_err(|source| DbError::Json {
                        context: "failed to serialize alternate_greetings".to_owned(),
                        source,
                    })?;
            let (note_text, note_depth, note_at_top) = author_note_columns(card);
            conn.execute(
                "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, alternate_greetings, created_at, updated_at, author_note, author_note_depth, author_note_at_top)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    slug,
                    card.name,
                    card.description,
                    card.personality,
                    card.scenario,
                    card.first_mes,
                    card.mes_example,
                    card.system_prompt,
                    card.post_history_instructions,
                    alternate_greetings,
                    now,
                    now,
                    note_text,
                    note_depth,
                    note_at_top,
                ],
            )
            .map_err(|source| DbError::Query {
                context: "failed to insert character".to_owned(),
                source,
            })?;
            Ok(())
        }
    )
}

pub fn load_character(conn: &Connection, slug: &str) -> Result<CharacterCard> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.character.load", slug = slug ; {
        let (
            name,
            description,
            personality,
            scenario,
            first_mes,
            mes_example,
            system_prompt,
            post_history_instructions,
            alternate_greetings_json,
            author_note_text,
            author_note_depth,
            author_note_at_top,
        ): CharacterRow = conn.query_row(
            "SELECT name, description, personality, scenario, first_mes, mes_example,
                    system_prompt, post_history_instructions, alternate_greetings,
                    author_note, author_note_depth, author_note_at_top
             FROM characters WHERE slug = ?1",
            params![slug],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                ))
            },
        )
        .map_err(|source| DbError::Query {
            context: format!("character not found: {slug}"),
            source,
        })?;

        let alternate_greetings: Vec<String> =
            serde_json::from_str(&alternate_greetings_json)
                .map_err(|source| DbError::Json {
                    context: "failed to deserialize alternate_greetings".to_owned(),
                    source,
                })?;
        let depth = u32::try_from(author_note_depth).unwrap_or_else(|_| {
            tracing::warn!(
                slug = slug,
                raw = author_note_depth,
                "db.character.load: author_note_depth out of range, defaulting to {}",
                libllm_core::author_note::DEFAULT_DEPTH
            );
            libllm_core::author_note::DEFAULT_DEPTH
        });
        let author_note = libllm_core::author_note::AuthorNote::from_row_parts(
            author_note_text,
            depth,
            author_note_at_top != 0,
        );
        Ok(CharacterCard {
            name,
            description,
            personality,
            scenario,
            first_mes,
            mes_example,
            system_prompt,
            post_history_instructions,
            alternate_greetings,
            author_note,
        })
    })
}

pub fn list_characters(conn: &Connection) -> Result<Vec<(String, String)>> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.character.list", ; {
        let entries = super::query_slug_name_pairs(
            conn,
            "SELECT slug, name FROM characters ORDER BY name",
            "failed to list characters",
        )?;
        tracing::info!(count = entries.len(), "db.character.list");
        Ok(entries)
    })
}

pub fn update_character(conn: &Connection, slug: &str, card: &CharacterCard) -> Result<()> {
    let alternate_greetings_count = card.alternate_greetings.len();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "db.character.update",
        slug = slug,
        alternate_greetings_count = alternate_greetings_count
        ; {
            let now = now_iso8601();
            let alternate_greetings =
                serde_json::to_string(&card.alternate_greetings)
                    .map_err(|source| DbError::Json {
                        context: "failed to serialize alternate_greetings".to_owned(),
                        source,
                    })?;
            let (note_text, note_depth, note_at_top) = author_note_columns(card);
            let affected = conn
                .execute(
                    "UPDATE characters SET name = ?1, description = ?2, personality = ?3, scenario = ?4, first_mes = ?5, mes_example = ?6, system_prompt = ?7, post_history_instructions = ?8, alternate_greetings = ?9, updated_at = ?10, author_note = ?11, author_note_depth = ?12, author_note_at_top = ?13 WHERE slug = ?14",
                    params![
                        card.name,
                        card.description,
                        card.personality,
                        card.scenario,
                        card.first_mes,
                        card.mes_example,
                        card.system_prompt,
                        card.post_history_instructions,
                        alternate_greetings,
                        now,
                        note_text,
                        note_depth,
                        note_at_top,
                        slug,
                    ],
                )
                .map_err(|source| DbError::Query {
                    context: "failed to update character".to_owned(),
                    source,
                })?;
            tracing::info!(slug = slug, affected = affected, "db.character.update");
            if affected == 0 {
                return Err(DbError::CharacterNotFound { slug: slug.to_owned() });
            }
            Ok(())
        }
    )
}

pub fn delete_character(conn: &Connection, slug: &str) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.character.delete", slug = slug ; {
        let affected = conn
            .execute("DELETE FROM characters WHERE slug = ?1", params![slug])
            .map_err(|source| DbError::Query {
                context: "failed to delete character".to_owned(),
                source,
            })?;
        tracing::info!(slug = slug, affected = affected, "db.character.delete");
        if affected == 0 {
            return Err(DbError::CharacterNotFound { slug: slug.to_owned() });
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::db::migrations::run_migrations;
    use libllm_core::character::CharacterCard;

    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn make_card() -> CharacterCard {
        CharacterCard {
            name: "Aria".to_owned(),
            description: "A helpful AI companion.".to_owned(),
            personality: "Curious and kind.".to_owned(),
            scenario: "Fantasy world.".to_owned(),
            first_mes: "Hello, traveler!".to_owned(),
            mes_example: "Example dialogue here.".to_owned(),
            system_prompt: "You are Aria.".to_owned(),
            post_history_instructions: "Stay in character.".to_owned(),
            alternate_greetings: vec!["Greetings!".to_owned(), "Welcome!".to_owned()],
            author_note: None,
        }
    }

    #[test]
    fn character_round_trip() {
        let conn = setup_db();
        let card = make_card();

        insert_character(&conn, "aria", &card).unwrap();
        let loaded = load_character(&conn, "aria").unwrap();

        assert_eq!(loaded.name, card.name);
        assert_eq!(loaded.description, card.description);
        assert_eq!(loaded.personality, card.personality);
        assert_eq!(loaded.scenario, card.scenario);
        assert_eq!(loaded.first_mes, card.first_mes);
        assert_eq!(loaded.mes_example, card.mes_example);
        assert_eq!(loaded.system_prompt, card.system_prompt);
        assert_eq!(
            loaded.post_history_instructions,
            card.post_history_instructions
        );
        assert_eq!(loaded.alternate_greetings, card.alternate_greetings);
        assert_eq!(loaded.author_note, card.author_note);
    }

    #[test]
    fn list_characters_ordering() {
        let conn = setup_db();

        let mut card_b = make_card();
        card_b.name = "Zara".to_owned();
        let mut card_a = make_card();
        card_a.name = "Aria".to_owned();

        insert_character(&conn, "zara", &card_b).unwrap();
        insert_character(&conn, "aria", &card_a).unwrap();

        let list = list_characters(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("aria".to_owned(), "Aria".to_owned()));
        assert_eq!(list[1], ("zara".to_owned(), "Zara".to_owned()));
    }

    #[test]
    fn update_and_delete_character() {
        let conn = setup_db();
        let card = make_card();
        insert_character(&conn, "aria", &card).unwrap();

        let mut updated = card.clone();
        updated.name = "Aria Updated".to_owned();
        updated.alternate_greetings = vec!["New greeting".to_owned()];
        update_character(&conn, "aria", &updated).unwrap();

        let loaded = load_character(&conn, "aria").unwrap();
        assert_eq!(loaded.name, "Aria Updated");
        assert_eq!(loaded.alternate_greetings, vec!["New greeting".to_owned()]);

        delete_character(&conn, "aria").unwrap();
        assert!(load_character(&conn, "aria").is_err());
    }

    #[test]
    fn character_author_note_round_trip_some() {
        let conn = setup_db();
        let mut card = make_card();
        card.author_note = Some(libllm_core::author_note::AuthorNote {
            text: "Stay in scene.".to_owned(),
            depth: 3,
            at_top: false,
        });

        insert_character(&conn, "aria", &card).unwrap();
        let loaded = load_character(&conn, "aria").unwrap();

        assert_eq!(loaded.author_note, card.author_note);
    }

    #[test]
    fn character_author_note_round_trip_none() {
        let conn = setup_db();
        let card = make_card();
        assert!(card.author_note.is_none());

        insert_character(&conn, "aria", &card).unwrap();
        let loaded = load_character(&conn, "aria").unwrap();

        assert_eq!(loaded.author_note, None);
    }

    #[test]
    fn character_author_note_update_round_trip() {
        let conn = setup_db();
        let card = make_card();
        insert_character(&conn, "aria", &card).unwrap();

        let mut updated = card.clone();
        updated.author_note = Some(libllm_core::author_note::AuthorNote {
            text: "edit later".to_owned(),
            depth: 1,
            at_top: true,
        });
        update_character(&conn, "aria", &updated).unwrap();

        let loaded = load_character(&conn, "aria").unwrap();
        assert_eq!(loaded.author_note, updated.author_note);
    }
}
