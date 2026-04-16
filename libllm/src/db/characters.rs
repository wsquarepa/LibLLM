//! Character card CRUD operations against the SQLite characters table.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::character::CharacterCard;
use crate::debug_log;
use crate::session::now_iso8601;

pub fn insert_character(conn: &Connection, slug: &str, card: &CharacterCard) -> Result<()> {
    debug_log::timed_result(
        "db.character.insert",
        &[
            debug_log::field("slug", slug),
            debug_log::field("alternate_greetings_count", card.alternate_greetings.len()),
        ],
        || {
            let now = now_iso8601();
            let alternate_greetings =
                serde_json::to_string(&card.alternate_greetings)
                    .context("failed to serialize alternate_greetings")?;
            conn.execute(
                "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, alternate_greetings, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                ],
            )
            .context("failed to insert character")?;
            Ok(())
        },
    )
}

pub fn load_character(conn: &Connection, slug: &str) -> Result<CharacterCard> {
    debug_log::timed_result(
        "db.character.load",
        &[debug_log::field("slug", slug)],
        || {
            conn.query_row(
                "SELECT name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, alternate_greetings
                 FROM characters WHERE slug = ?1",
                params![slug],
                |row| {
                    let name: String = row.get(0)?;
                    let description: String = row.get(1)?;
                    let personality: String = row.get(2)?;
                    let scenario: String = row.get(3)?;
                    let first_mes: String = row.get(4)?;
                    let mes_example: String = row.get(5)?;
                    let system_prompt: String = row.get(6)?;
                    let post_history_instructions: String = row.get(7)?;
                    let alternate_greetings_json: String = row.get(8)?;
                    Ok((
                        name,
                        description,
                        personality,
                        scenario,
                        first_mes,
                        mes_example,
                        system_prompt,
                        post_history_instructions,
                        alternate_greetings_json,
                    ))
                },
            )
            .with_context(|| format!("character not found: {slug}"))
            .and_then(
                |(
                    name,
                    description,
                    personality,
                    scenario,
                    first_mes,
                    mes_example,
                    system_prompt,
                    post_history_instructions,
                    alternate_greetings_json,
                )| {
                    let alternate_greetings: Vec<String> =
                        serde_json::from_str(&alternate_greetings_json)
                            .context("failed to deserialize alternate_greetings")?;
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
                    })
                },
            )
        },
    )
}

pub fn list_characters(conn: &Connection) -> Result<Vec<(String, String)>> {
    debug_log::timed_result("db.character.list", &[], || {
        let entries = super::query_slug_name_pairs(
            conn,
            "SELECT slug, name FROM characters ORDER BY name",
            "failed to list characters",
        )?;
        debug_log::log_kv(
            "db.character.list",
            &[
                debug_log::field("phase", "summary"),
                debug_log::field("count", entries.len()),
            ],
        );
        Ok(entries)
    })
}

pub fn update_character(conn: &Connection, slug: &str, card: &CharacterCard) -> Result<()> {
    debug_log::timed_result(
        "db.character.update",
        &[
            debug_log::field("slug", slug),
            debug_log::field("alternate_greetings_count", card.alternate_greetings.len()),
        ],
        || {
            let now = now_iso8601();
            let alternate_greetings =
                serde_json::to_string(&card.alternate_greetings)
                    .context("failed to serialize alternate_greetings")?;
            let affected = conn
                .execute(
                    "UPDATE characters SET name = ?1, description = ?2, personality = ?3, scenario = ?4, first_mes = ?5, mes_example = ?6, system_prompt = ?7, post_history_instructions = ?8, alternate_greetings = ?9, updated_at = ?10 WHERE slug = ?11",
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
                        slug,
                    ],
                )
                .context("failed to update character")?;
            debug_log::log_kv(
                "db.character.update",
                &[
                    debug_log::field("phase", "summary"),
                    debug_log::field("slug", slug),
                    debug_log::field("affected", affected),
                ],
            );
            if affected == 0 {
                anyhow::bail!("character not found: {slug}");
            }
            Ok(())
        },
    )
}

pub fn delete_character(conn: &Connection, slug: &str) -> Result<()> {
    debug_log::timed_result(
        "db.character.delete",
        &[debug_log::field("slug", slug)],
        || {
            let affected = conn
                .execute("DELETE FROM characters WHERE slug = ?1", params![slug])
                .context("failed to delete character")?;
            debug_log::log_kv(
                "db.character.delete",
                &[
                    debug_log::field("phase", "summary"),
                    debug_log::field("slug", slug),
                    debug_log::field("affected", affected),
                ],
            );
            if affected == 0 {
                anyhow::bail!("character not found: {slug}");
            }
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::character::CharacterCard;
    use crate::db::schema::run_migrations;

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
        assert_eq!(loaded.post_history_instructions, card.post_history_instructions);
        assert_eq!(loaded.alternate_greetings, card.alternate_greetings);
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
}
