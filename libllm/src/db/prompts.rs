//! System prompt CRUD operations with builtin prompt seeding.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::debug_log;
use crate::session::now_iso8601;
use crate::system_prompt::{BUILTIN_ASSISTANT, BUILTIN_ROLEPLAY, SystemPromptFile};

pub struct PromptListEntry {
    pub slug: String,
    pub name: String,
    pub builtin: bool,
}

pub fn insert_prompt(
    conn: &Connection,
    slug: &str,
    prompt: &SystemPromptFile,
    builtin: bool,
) -> Result<()> {
    debug_log::timed_result(
        "db.prompt.insert",
        &[
            debug_log::field("slug", slug),
            debug_log::field("builtin", builtin),
            debug_log::field("content_bytes", prompt.content.len()),
        ],
        || {
            let now = now_iso8601();
            conn.execute(
                "INSERT INTO system_prompts (slug, name, content, builtin, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![slug, prompt.name, prompt.content, builtin as i64, now, now],
            )
            .context("failed to insert system prompt")?;
            Ok(())
        },
    )
}

pub fn load_prompt(conn: &Connection, slug: &str) -> Result<SystemPromptFile> {
    debug_log::timed_result(
        "db.prompt.load",
        &[debug_log::field("slug", slug)],
        || {
            conn.query_row(
                "SELECT name, content FROM system_prompts WHERE slug = ?1",
                params![slug],
                |row| {
                    let name: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    Ok(SystemPromptFile { name, content })
                },
            )
            .with_context(|| format!("system prompt not found: {slug}"))
        },
    )
}

pub fn list_prompts(conn: &Connection) -> Result<Vec<PromptListEntry>> {
    debug_log::timed_result("db.prompt.list", &[], || {
        let mut stmt = conn
            .prepare("SELECT slug, name, builtin FROM system_prompts ORDER BY builtin DESC, name")
            .context("failed to prepare list_prompts query")?;

        let rows = stmt
            .query_map([], |row| {
                let slug: String = row.get(0)?;
                let name: String = row.get(1)?;
                let builtin: i64 = row.get(2)?;
                Ok(PromptListEntry {
                    slug,
                    name,
                    builtin: builtin != 0,
                })
            })
            .context("failed to query system prompts")?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.context("failed to read system prompt row")?);
        }
        let builtin_count = entries.iter().filter(|e| e.builtin).count();
        debug_log::log_kv(
            "db.prompt.list",
            &[
                debug_log::field("phase", "summary"),
                debug_log::field("count", entries.len()),
                debug_log::field("builtin_count", builtin_count),
            ],
        );
        Ok(entries)
    })
}

pub fn update_prompt(conn: &Connection, slug: &str, prompt: &SystemPromptFile) -> Result<()> {
    debug_log::timed_result(
        "db.prompt.update",
        &[
            debug_log::field("slug", slug),
            debug_log::field("content_bytes", prompt.content.len()),
        ],
        || {
            let now = now_iso8601();
            let affected = conn
                .execute(
                    "UPDATE system_prompts SET name = ?1, content = ?2, updated_at = ?3 WHERE slug = ?4",
                    params![prompt.name, prompt.content, now, slug],
                )
                .context("failed to update system prompt")?;
            debug_log::log_kv(
                "db.prompt.update",
                &[
                    debug_log::field("phase", "summary"),
                    debug_log::field("slug", slug),
                    debug_log::field("affected", affected),
                ],
            );
            if affected == 0 {
                anyhow::bail!("system prompt not found: {slug}");
            }
            Ok(())
        },
    )
}

pub fn delete_prompt(conn: &Connection, slug: &str) -> Result<()> {
    debug_log::timed_result(
        "db.prompt.delete",
        &[debug_log::field("slug", slug)],
        || {
            let affected = conn
                .execute("DELETE FROM system_prompts WHERE slug = ?1", params![slug])
                .context("failed to delete system prompt")?;
            debug_log::log_kv(
                "db.prompt.delete",
                &[
                    debug_log::field("phase", "summary"),
                    debug_log::field("slug", slug),
                    debug_log::field("affected", affected),
                ],
            );
            if affected == 0 {
                anyhow::bail!("system prompt not found: {slug}");
            }
            Ok(())
        },
    )
}

pub fn ensure_builtins(conn: &Connection) -> Result<()> {
    debug_log::timed_result("db.prompt.ensure_builtins", &[], || {
        let mut inserted = 0usize;
        let mut existed = 0usize;
        for slug in [BUILTIN_ASSISTANT, BUILTIN_ROLEPLAY] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM system_prompts WHERE slug = ?1",
                    params![slug],
                    |row| row.get(0),
                )
                .context("failed to check builtin prompt existence")?;

            if !exists {
                let prompt = SystemPromptFile {
                    name: slug.to_owned(),
                    content: String::new(),
                };
                insert_prompt(conn, slug, &prompt, true)?;
                inserted += 1;
            } else {
                existed += 1;
            }
        }
        debug_log::log_kv(
            "db.prompt.ensure_builtins",
            &[
                debug_log::field("phase", "summary"),
                debug_log::field("inserted", inserted),
                debug_log::field("existed", existed),
            ],
        );
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::db::schema::run_migrations;
    use crate::system_prompt::{BUILTIN_ASSISTANT, BUILTIN_ROLEPLAY, SystemPromptFile};

    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn prompt_round_trip() {
        let conn = setup_db();
        let prompt = SystemPromptFile {
            name: "My Prompt".to_owned(),
            content: "You are a helpful assistant.".to_owned(),
        };

        insert_prompt(&conn, "my-prompt", &prompt, false).unwrap();
        let loaded = load_prompt(&conn, "my-prompt").unwrap();

        assert_eq!(loaded.name, prompt.name);
        assert_eq!(loaded.content, prompt.content);
    }

    #[test]
    fn list_prompts_includes_builtin_flag() {
        let conn = setup_db();

        let builtin = SystemPromptFile {
            name: BUILTIN_ASSISTANT.to_owned(),
            content: String::new(),
        };
        insert_prompt(&conn, BUILTIN_ASSISTANT, &builtin, true).unwrap();

        let custom = SystemPromptFile {
            name: "Custom".to_owned(),
            content: "Custom content.".to_owned(),
        };
        insert_prompt(&conn, "custom", &custom, false).unwrap();

        let list = list_prompts(&conn).unwrap();
        assert_eq!(list.len(), 2);

        let builtin_entry = list.iter().find(|e| e.slug == BUILTIN_ASSISTANT).unwrap();
        assert!(builtin_entry.builtin);

        let custom_entry = list.iter().find(|e| e.slug == "custom").unwrap();
        assert!(!custom_entry.builtin);
    }

    #[test]
    fn ensure_builtins_is_idempotent() {
        let conn = setup_db();

        ensure_builtins(&conn).unwrap();
        ensure_builtins(&conn).unwrap();

        let list = list_prompts(&conn).unwrap();
        let assistant_count = list.iter().filter(|e| e.slug == BUILTIN_ASSISTANT).count();
        let roleplay_count = list.iter().filter(|e| e.slug == BUILTIN_ROLEPLAY).count();

        assert_eq!(assistant_count, 1);
        assert_eq!(roleplay_count, 1);

        let assistant = load_prompt(&conn, BUILTIN_ASSISTANT).unwrap();
        assert_eq!(assistant.name, BUILTIN_ASSISTANT);
        let roleplay = load_prompt(&conn, BUILTIN_ROLEPLAY).unwrap();
        assert_eq!(roleplay.name, BUILTIN_ROLEPLAY);
    }
}
