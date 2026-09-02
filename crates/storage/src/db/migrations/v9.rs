//! v9: Rename `chat_policy` → `chat_mode`, drop `card_assembly`, add `scenario` column,
//! and populate `scenario` from attached character cards for all existing sessions.

use rusqlite::{Connection, params};

use crate::error::Result;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let sessions_cols = super::v6::table_columns(conn, "sessions")?;
    let needs_rename = sessions_cols.iter().any(|c| c == "chat_policy");
    if needs_rename {
        conn.execute_batch("ALTER TABLE sessions RENAME COLUMN chat_policy TO chat_mode;")?;
    }

    let has_card_assembly = sessions_cols.iter().any(|c| c == "card_assembly");
    if has_card_assembly {
        conn.execute_batch("ALTER TABLE sessions DROP COLUMN card_assembly;")?;
    }

    let has_scenario = sessions_cols.iter().any(|c| c == "scenario");
    if !has_scenario {
        conn.execute_batch("ALTER TABLE sessions ADD COLUMN scenario TEXT;")?;
    }

    conn.execute(
        "UPDATE sessions \
         SET scenario = ( \
           SELECT c.scenario FROM characters c \
           JOIN session_characters sc ON sc.slug = c.slug \
           WHERE sc.session_id = sessions.id \
         ) \
         WHERE sessions.scenario IS NULL \
           AND ( \
             SELECT COUNT(*) FROM session_characters WHERE session_id = sessions.id \
           ) = 1",
        [],
    )?;

    let mut group_sessions = conn.prepare(
        "SELECT id FROM sessions \
         WHERE scenario IS NULL \
           AND ( \
             SELECT COUNT(*) FROM session_characters WHERE session_id = sessions.id \
           ) >= 2",
    )?;
    let ids: Vec<String> = group_sessions
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(group_sessions);

    for sid in ids {
        let mut rows = conn.prepare(
            "SELECT c.name, c.scenario \
             FROM session_characters sc \
             JOIN characters c ON c.slug = sc.slug \
             WHERE sc.session_id = ?1 \
             ORDER BY sc.attach_index",
        )?;
        let entries: Vec<(String, Option<String>)> = rows
            .query_map(params![&sid], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(rows);

        let parts: Vec<String> = entries
            .iter()
            .filter_map(|(name, scenario)| {
                let text = scenario.as_deref().filter(|s| !s.is_empty())?;
                Some(format!("[Scenario for {name}]\n{text}"))
            })
            .collect();

        if parts.is_empty() {
            continue;
        }

        let joined = parts.join("\n");
        conn.execute(
            "UPDATE sessions SET scenario = ?1 WHERE id = ?2",
            params![joined, sid],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{Connection, params};

    fn seed_v8(conn: &Connection) -> Result<()> {
        super::super::v1::migrate(conn)?;
        super::super::v2::migrate(conn)?;
        super::super::v3::migrate(conn)?;
        super::super::v4::migrate(conn)?;
        super::super::v5::migrate(conn)?;
        super::super::v6::migrate(conn)?;
        super::super::v7::migrate(conn)?;
        super::super::v8::migrate(conn)?;
        Ok(())
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let cols = super::super::v6::table_columns(conn, table)?;
        Ok(cols.iter().any(|c| c == column))
    }

    #[test]
    fn v9_renames_chat_policy_to_chat_mode() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        super::migrate(&conn)?;
        assert!(!column_exists(&conn, "sessions", "chat_policy")?);
        assert!(column_exists(&conn, "sessions", "chat_mode")?);
        Ok(())
    }

    #[test]
    fn v9_drops_card_assembly() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        super::migrate(&conn)?;
        assert!(!column_exists(&conn, "sessions", "card_assembly")?);
        Ok(())
    }

    #[test]
    fn v9_adds_scenario_column() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        super::migrate(&conn)?;
        assert!(column_exists(&conn, "sessions", "scenario")?);
        Ok(())
    }

    #[test]
    fn v9_populates_scenario_for_solo_session() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        conn.execute(
            "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) VALUES ('alice', 'Alice', '', '', 'A medieval tavern.', '', '', '', '', 'now', 'now')",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy) VALUES ('s1', 'solo', 'now', 'now', NULL, 'alice', 'round_robin')",
            [],
        )?;
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) VALUES ('s1', 'alice', 0, 1.0, 0.0)",
            [],
        )?;
        super::migrate(&conn)?;
        let scenario: Option<String> =
            conn.query_row("SELECT scenario FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })?;
        assert_eq!(scenario.as_deref(), Some("A medieval tavern."));
        Ok(())
    }

    #[test]
    fn v9_populates_scenario_for_group_session() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        for (slug, name, scenario) in [
            ("alice", "Alice", "Alice is hunting."),
            ("bob", "Bob", "Bob is brewing."),
        ] {
            conn.execute(
                "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) VALUES (?1, ?2, '', '', ?3, '', '', '', '', 'now', 'now')",
                params![slug, name, scenario],
            )?;
        }
        conn.execute(
            "INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy, card_assembly) VALUES ('g1', 'group', 'now', 'now', NULL, NULL, 'weighted_random', 'join_cards')",
            [],
        )?;
        for (i, slug) in ["alice", "bob"].iter().enumerate() {
            conn.execute(
                "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) VALUES ('g1', ?1, ?2, 0.5, 0.0)",
                params![slug, i as i64],
            )?;
        }
        super::migrate(&conn)?;
        let scenario: Option<String> =
            conn.query_row("SELECT scenario FROM sessions WHERE id = 'g1'", [], |row| {
                row.get(0)
            })?;
        let expected =
            "[Scenario for Alice]\nAlice is hunting.\n[Scenario for Bob]\nBob is brewing.";
        assert_eq!(scenario.as_deref(), Some(expected));
        Ok(())
    }

    #[test]
    fn v9_group_session_handles_null_card_scenario() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        conn.execute(
            "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) VALUES ('alice', 'Alice', '', '', NULL, '', '', '', '', 'now', 'now')",
            [],
        )?;
        conn.execute(
            "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) VALUES ('bob', 'Bob', '', '', 'Bob is brewing.', '', '', '', '', 'now', 'now')",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy, card_assembly) VALUES ('g1', 'group', 'now', 'now', NULL, NULL, 'weighted_random', 'join_cards')",
            [],
        )?;
        for (i, slug) in ["alice", "bob"].iter().enumerate() {
            conn.execute(
                "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) VALUES ('g1', ?1, ?2, 0.5, 0.0)",
                params![slug, i as i64],
            )?;
        }
        super::migrate(&conn)?;
        let scenario: Option<String> =
            conn.query_row("SELECT scenario FROM sessions WHERE id = 'g1'", [], |row| {
                row.get(0)
            })?;
        assert_eq!(
            scenario.as_deref(),
            Some("[Scenario for Bob]\nBob is brewing.")
        );
        Ok(())
    }

    #[test]
    fn v9_group_session_all_empty_scenarios_leaves_null() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        for slug in ["alice", "bob"] {
            conn.execute(
                "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) VALUES (?1, ?1, '', '', '', '', '', '', '', 'now', 'now')",
                params![slug],
            )?;
        }
        conn.execute(
            "INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy, card_assembly) VALUES ('g1', 'group', 'now', 'now', NULL, NULL, 'weighted_random', 'join_cards')",
            [],
        )?;
        for (i, slug) in ["alice", "bob"].iter().enumerate() {
            conn.execute(
                "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) VALUES ('g1', ?1, ?2, 0.5, 0.0)",
                params![slug, i as i64],
            )?;
        }
        super::migrate(&conn)?;
        let scenario: Option<String> =
            conn.query_row("SELECT scenario FROM sessions WHERE id = 'g1'", [], |row| {
                row.get(0)
            })?;
        assert_eq!(scenario, None);
        Ok(())
    }

    #[test]
    fn v9_solo_with_deleted_card_leaves_scenario_null() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        seed_v8(&conn)?;
        conn.execute(
            "INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy) VALUES ('s1', 'solo', 'now', 'now', NULL, 'gone', 'round_robin')",
            [],
        )?;
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) VALUES ('s1', 'gone', 0, 1.0, 0.0)",
            [],
        )?;
        super::migrate(&conn)?;
        let scenario: Option<String> =
            conn.query_row("SELECT scenario FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })?;
        assert_eq!(scenario, None);
        Ok(())
    }
}
