//! Session persistence: insert, load, list, delete, and incremental message updates.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::error::{DbError, Result};
use libllm_core::session::{Message, MessageTree, Node, NodeId, Role, Session, now_iso8601};

type SessionRow = (
    Option<String>, // model
    Option<String>, // template
    Option<String>, // system_prompt
    Option<String>, // character
    Option<String>, // persona
    Option<i64>,    // head_id
    Option<String>, // chat_mode
    Option<String>, // scenario
    Option<String>, // author_note
    i64,            // author_note_depth
    i64,            // author_note_at_top
);

pub struct SessionListEntry {
    pub id: String,
    pub display_name: String,
    pub message_count: usize,
    pub updated_at: String,
}

fn display_name_for_session(session: &Session) -> String {
    if session.characters.len() <= 1 {
        return compute_legacy_mirror(session).unwrap_or_else(|| "Assistant".to_owned());
    }
    let names: Vec<&str> = session.characters.iter().map(|c| c.slug.as_str()).collect();
    join_truncated_names(&names)
}

fn join_truncated_names(names: &[&str]) -> String {
    if names.len() <= 3 {
        names.join(", ")
    } else {
        format!(
            "{}, {}, {}, +{} more",
            names[0],
            names[1],
            names[2],
            names.len() - 3,
        )
    }
}

/// Slug to write into `sessions.character` for back-compat: solo sessions
/// (`characters.len() <= 1`) mirror their attachment slug; group sessions return None.
///
/// The `or_else(session.character.clone())` branch is a back-compat bridge for
/// sessions whose `characters` Vec was never populated (e.g. legacy in-memory state
/// before group-chat code wires `characters`). `load_session` synthesizes a single
/// attachment from `sessions.character` on the next read, so the round-trip is preserved.
fn compute_legacy_mirror(session: &Session) -> Option<String> {
    if session.characters.len() <= 1 {
        session
            .characters
            .first()
            .map(|a| a.slug.clone())
            .or_else(|| session.character.clone())
    } else {
        None
    }
}

fn author_note_columns(session: &Session) -> (Option<&str>, i64, i64) {
    match session.author_note.as_ref() {
        Some(note) => (
            Some(note.text.as_str()),
            note.depth as i64,
            note.at_top as i64,
        ),
        None => (None, libllm_core::author_note::DEFAULT_DEPTH as i64, 0),
    }
}

fn insert_session_row(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    let now = now_iso8601();
    let legacy_mirror = compute_legacy_mirror(session);
    let display_name = display_name_for_session(session);
    let head_id = session.tree.head().map(|h| h as i64);
    let (note_text, note_depth, note_at_top) = author_note_columns(session);

    conn.execute(
        "INSERT INTO sessions (id, display_name, model, template, system_prompt, character, persona, head_id, chat_mode, scenario, created_at, updated_at, author_note, author_note_depth, author_note_at_top)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            display_name,
            session.model,
            session.template,
            session.system_prompt,
            legacy_mirror,
            session.persona,
            head_id,
            session.chat_mode.as_db_str(),
            session.scenario,
            now,
            now,
            note_text,
            note_depth,
            note_at_top,
        ],
    )
    .map_err(|source| DbError::Query {
        context: "failed to insert session row".to_owned(),
        source,
    })?;
    Ok(())
}

/// Upsert the `sessions` row without deleting it.
/// Preserves the existing row id so `ON DELETE CASCADE` dependants
/// (messages, session_worldbooks, file_summaries) are not wiped.
fn upsert_session_row(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    let now = now_iso8601();
    let legacy_mirror = compute_legacy_mirror(session);
    let display_name = display_name_for_session(session);
    let head_id = session.tree.head().map(|h| h as i64);
    let (note_text, note_depth, note_at_top) = author_note_columns(session);

    conn.execute(
        "INSERT INTO sessions (id, display_name, model, template, system_prompt, character, persona, head_id, chat_mode, scenario, created_at, updated_at, author_note, author_note_depth, author_note_at_top)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET
            display_name = excluded.display_name,
            model = excluded.model,
            template = excluded.template,
            system_prompt = excluded.system_prompt,
            character = excluded.character,
            persona = excluded.persona,
            head_id = excluded.head_id,
            chat_mode = excluded.chat_mode,
            scenario = excluded.scenario,
            updated_at = excluded.updated_at,
            author_note = excluded.author_note,
            author_note_depth = excluded.author_note_depth,
            author_note_at_top = excluded.author_note_at_top",
        params![
            id,
            display_name,
            session.model,
            session.template,
            session.system_prompt,
            legacy_mirror,
            session.persona,
            head_id,
            session.chat_mode.as_db_str(),
            session.scenario,
            now,
            note_text,
            note_depth,
            note_at_top,
        ],
    )
    .map_err(|source| DbError::Query {
        context: "failed to upsert session row".to_owned(),
        source,
    })?;
    Ok(())
}

fn write_messages_and_worldbooks(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    for node in session.tree.nodes() {
        let preferred_child_id = session
            .tree
            .preferred_child_map()
            .get(&node.id)
            .map(|&c| c as i64);
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp, thought_seconds, speaker_slug, pre_turn_action_points)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                node.id as i64,
                id,
                node.parent.map(|p| p as i64),
                preferred_child_id,
                node.message.role.to_string(),
                node.message.content,
                node.message.timestamp,
                node.message.thought_seconds.map(i64::from),
                node.message.speaker,
                node.message.pre_turn_action_points,
            ],
        )
        .map_err(|source| DbError::Query {
            context: "failed to insert message row".to_owned(),
            source,
        })?;
    }

    for worldbook_slug in &session.worldbooks {
        conn.execute(
            "INSERT INTO session_worldbooks (session_id, worldbook_slug) VALUES (?1, ?2)",
            params![id, worldbook_slug],
        )
        .map_err(|source| DbError::Query {
            context: "failed to insert session_worldbooks row".to_owned(),
            source,
        })?;
    }

    Ok(())
}

fn write_session_characters(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    // Wipe-and-rewrite: callers do not pre-delete (unlike messages/session_worldbooks
    // which are cleared in save_session before the corresponding writer is called).
    conn.execute(
        "DELETE FROM session_characters WHERE session_id = ?1",
        params![id],
    )
    .map_err(|source| DbError::Query {
        context: "failed to clear session_characters".to_owned(),
        source,
    })?;

    for (idx, attachment) in session.characters.iter().enumerate() {
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                attachment.slug,
                idx as i64,
                attachment.talkativeness as f64,
                attachment.action_points as f64,
            ],
        )
        .map_err(|source| DbError::Query {
            context: "failed to insert session_characters row".to_owned(),
            source,
        })?;
    }
    Ok(())
}

pub fn insert_session(conn: &mut Connection, id: &str, session: &Session) -> Result<()> {
    let node_count = session.tree.node_count();
    let worldbook_count = session.worldbooks.len();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "db.session.insert",
        session_id = id,
        node_count = node_count,
        worldbook_count = worldbook_count
        ; {
            let sp = conn.savepoint().map_err(|source| DbError::Query {
                context: "failed to begin savepoint".to_owned(),
                source,
            })?;
            insert_session_row(&sp, id, session)?;
            write_messages_and_worldbooks(&sp, id, session)?;
            write_session_characters(&sp, id, session)?;
            sp.commit().map_err(|source| DbError::Query {
                context: "failed to commit session insert".to_owned(),
                source,
            })?;
            Ok(())
        }
    )
}

pub fn save_session(conn: &mut Connection, id: &str, session: &Session) -> Result<()> {
    let node_count = session.tree.node_count();
    let worldbook_count = session.worldbooks.len();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "db.session.save",
        session_id = id,
        node_count = node_count,
        worldbook_count = worldbook_count
        ; {
            let sp = conn.savepoint().map_err(|source| DbError::Query {
                context: "failed to begin savepoint".to_owned(),
                source,
            })?;
            upsert_session_row(&sp, id, session)?;
            sp.execute("DELETE FROM messages WHERE session_id = ?1", params![id])
                .map_err(|source| DbError::Query {
                    context: "failed to clear messages".to_owned(),
                    source,
                })?;
            sp.execute(
                "DELETE FROM session_worldbooks WHERE session_id = ?1",
                params![id],
            )
            .map_err(|source| DbError::Query {
                context: "failed to clear session_worldbooks".to_owned(),
                source,
            })?;
            write_messages_and_worldbooks(&sp, id, session)?;
            write_session_characters(&sp, id, session)?;
            sp.commit().map_err(|source| DbError::Query {
                context: "failed to commit session save".to_owned(),
                source,
            })?;
            Ok(())
        }
    )
}

pub fn ids_matching_display_name(conn: &Connection, substring: &str) -> Result<Vec<String>> {
    let escaped = substring
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let mut stmt = conn
        .prepare(
            "SELECT id FROM sessions \
             WHERE display_name IS NOT NULL \
               AND display_name LIKE '%' || ?1 || '%' ESCAPE '\\' COLLATE NOCASE \
             ORDER BY id",
        )
        .map_err(|source| DbError::Query {
            context: "failed to prepare session lookup".to_owned(),
            source,
        })?;
    let rows = stmt
        .query_map(params![escaped], |row| row.get::<_, String>(0))
        .map_err(|source| DbError::Query {
            context: "failed to execute session lookup".to_owned(),
            source,
        })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::Sqlite)
}

pub fn session_exists(conn: &Connection, id: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|source| DbError::Query {
            context: "failed to check session existence".to_owned(),
            source,
        })?;
    tracing::debug!(
        session_id = id,
        result = "ok",
        found = count > 0,
        "db.session.exists"
    );
    Ok(count > 0)
}

/// Loads all message rows for `session_id` and assembles the arena tree.
/// `head_id` comes from the sessions row.
fn load_message_tree(
    conn: &Connection,
    session_id: &str,
    head_id: Option<i64>,
) -> Result<MessageTree> {
    let mut stmt = conn
        .prepare(
            "SELECT id, parent_id, preferred_child_id, role, content, timestamp, thought_seconds, speaker_slug, pre_turn_action_points
             FROM messages WHERE session_id = ?1 ORDER BY id",
        )
        .map_err(|source| DbError::Query {
            context: "failed to prepare message query".to_owned(),
            source,
        })?;

    let mut nodes: Vec<Node> = Vec::new();
    let mut preferred_child: HashMap<NodeId, NodeId> = HashMap::new();

    let message_rows = stmt
        .query_map(params![session_id], |row| {
            let msg_id: i64 = row.get(0)?;
            let parent_id: Option<i64> = row.get(1)?;
            let preferred_child_id: Option<i64> = row.get(2)?;
            let role_str: String = row.get(3)?;
            let content: String = row.get(4)?;
            let timestamp: String = row.get(5)?;
            let thought_seconds: Option<i64> = row.get(6)?;
            let speaker: Option<String> = row.get(7)?;
            let pre_turn_action_points: Option<String> = row.get(8)?;
            Ok((
                msg_id,
                parent_id,
                preferred_child_id,
                role_str,
                content,
                timestamp,
                thought_seconds,
                speaker,
                pre_turn_action_points,
            ))
        })
        .map_err(|source| DbError::Query {
            context: "failed to query messages".to_owned(),
            source,
        })?;

    for row in message_rows {
        let (
            msg_id,
            parent_id,
            preferred_child_id,
            role_str,
            content,
            timestamp,
            thought_seconds,
            speaker,
            pre_turn_action_points,
        ) = row.map_err(|source| DbError::Query {
            context: "failed to read message row".to_owned(),
            source,
        })?;

        let role: Role = role_str.parse().map_err(|_| DbError::Query {
            context: format!("invalid role in message {msg_id}: {role_str}"),
            source: rusqlite::Error::InvalidColumnType(
                3,
                role_str.clone(),
                rusqlite::types::Type::Text,
            ),
        })?;
        let thought_seconds: Option<u32> =
            thought_seconds.and_then(|seconds| u32::try_from(seconds).ok());

        let node = Node {
            id: msg_id as usize,
            parent: parent_id.map(|p| p as usize),
            children: Vec::new(),
            message: Message {
                role,
                content,
                timestamp,
                thought_seconds,
                speaker,
                pre_turn_action_points,
            },
        };

        if let Some(child_id) = preferred_child_id {
            preferred_child.insert(msg_id as usize, child_id as usize);
        }

        nodes.push(node);
    }

    for i in 0..nodes.len() {
        if let Some(parent_id) = nodes[i].parent {
            let child_id = nodes[i].id;
            if let Some(parent_node) = nodes.get_mut(parent_id) {
                parent_node.children.push(child_id);
            }
        }
    }

    let head = head_id.map(|h| h as usize);
    Ok(MessageTree::from_parts(nodes, head, preferred_child))
}

/// Worldbook slugs attached to the session.
fn load_session_worldbooks(conn: &Connection, session_id: &str) -> Result<Vec<String>> {
    let mut wb_stmt = conn
        .prepare("SELECT worldbook_slug FROM session_worldbooks WHERE session_id = ?1")
        .map_err(|source| DbError::Query {
            context: "failed to prepare worldbooks query".to_owned(),
            source,
        })?;
    let wb_rows = wb_stmt
        .query_map(params![session_id], |row| row.get(0))
        .map_err(|source| DbError::Query {
            context: "failed to query worldbooks".to_owned(),
            source,
        })?;
    wb_rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|source| DbError::Query {
            context: "failed to read worldbook row".to_owned(),
            source,
        })
}

/// Group-chat character attachments ordered by `attach_index`. When the
/// session predates group chat (no attachment rows), falls back to a single
/// attachment built from the legacy `character` column.
fn load_session_characters(
    conn: &Connection,
    session_id: &str,
    legacy_character: Option<&str>,
) -> Result<Vec<libllm_core::group_chat::CharacterAttachment>> {
    let mut ch_stmt = conn
        .prepare(
            "SELECT slug, talkativeness, action_points
             FROM session_characters WHERE session_id = ?1
             ORDER BY attach_index",
        )
        .map_err(|source| DbError::Query {
            context: "failed to prepare session_characters query".to_owned(),
            source,
        })?;
    let ch_rows = ch_stmt
        .query_map(params![session_id], |row| {
            let slug: String = row.get(0)?;
            let talkativeness: f64 = row.get(1)?;
            let action_points: f64 = row.get(2)?;
            Ok(libllm_core::group_chat::CharacterAttachment {
                slug,
                talkativeness: talkativeness as f32,
                action_points: action_points as f32,
                spoke_this_round: false,
            })
        })
        .map_err(|source| DbError::Query {
            context: "failed to query session_characters".to_owned(),
            source,
        })?;
    let mut characters: Vec<libllm_core::group_chat::CharacterAttachment> = ch_rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|source| DbError::Query {
            context: "failed to read session_characters row".to_owned(),
            source,
        })?;

    if characters.is_empty()
        && let Some(slug) = legacy_character
    {
        characters.push(libllm_core::group_chat::CharacterAttachment {
            slug: slug.to_owned(),
            talkativeness: 1.0,
            action_points: 0.0,
            spoke_this_round: false,
        });
    }

    Ok(characters)
}

pub fn load_session(conn: &Connection, id: &str) -> Result<Session> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.session.load", session_id = id ; {
            let (
                model,
                template,
                system_prompt,
                character,
                persona,
                head_id,
                chat_mode_str,
                scenario,
                author_note_text,
                author_note_depth,
                author_note_at_top,
            ): SessionRow = conn
                .query_row(
                    "SELECT model, template, system_prompt, character, persona, head_id,
                            chat_mode, scenario,
                            author_note, author_note_depth, author_note_at_top
                     FROM sessions WHERE id = ?1",
                    params![id],
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
                        ))
                    },
                )
                .map_err(|source| DbError::Query {
                    context: format!("session not found: {id}"),
                    source,
                })?;

            let chat_mode = libllm_core::group_chat::ChatMode::from_db_str(
                chat_mode_str.as_deref().unwrap_or(""),
            )
            .unwrap_or_default();

            let tree = load_message_tree(conn, id, head_id)?;
            let worldbooks = load_session_worldbooks(conn, id)?;
            let characters = load_session_characters(conn, id, character.as_deref())?;

            let depth = u32::try_from(author_note_depth).unwrap_or_else(|_| {
                tracing::warn!(
                    session_id = id,
                    raw = author_note_depth,
                    "db.session.load: author_note_depth out of range, defaulting to {}",
                    libllm_core::author_note::DEFAULT_DEPTH
                );
                libllm_core::author_note::DEFAULT_DEPTH
            });
            let author_note = libllm_core::author_note::AuthorNote::from_row_parts(
                author_note_text,
                depth,
                author_note_at_top != 0,
            );

            Ok(Session {
                tree,
                model,
                template,
                system_prompt,
                character,
                worldbooks,
                persona,
                scenario,
                characters,
                chat_mode,
                author_note,
            })
    })
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionListEntry>> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.session.list", ; {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.display_name, s.updated_at,
                        COUNT(m.id) AS message_count
                 FROM sessions s
                 LEFT JOIN messages m ON m.session_id = s.id
                 GROUP BY s.id
                 ORDER BY s.updated_at DESC",
            )
            .map_err(|source| DbError::Query {
                context: "failed to prepare list_sessions query".to_owned(),
                source,
            })?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let display_name: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
                let updated_at: String = row.get(2)?;
                let message_count: i64 = row.get(3)?;
                Ok(SessionListEntry {
                    id,
                    display_name,
                    message_count: message_count as usize,
                    updated_at,
                })
            })
            .map_err(|source| DbError::Query {
                context: "failed to query sessions".to_owned(),
                source,
            })?;

        let entries: Vec<SessionListEntry> = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| DbError::Query {
                context: "failed to read session row".to_owned(),
                source,
            })?;
        tracing::debug!(session_count = entries.len(), "db.session.list");
        Ok(entries)
    })
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "db.session.delete", session_id = id ; {
        let affected = conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|source| DbError::Query {
                context: "failed to delete session".to_owned(),
                source,
            })?;
        tracing::debug!(session_id = id, affected = affected, "db.session.delete");
        if affected == 0 {
            return Err(DbError::SessionNotFound { id: id.to_owned() });
        }
        Ok(())
    })
}

pub fn upsert_message(conn: &mut Connection, session_id: &str, node: &Node) -> Result<()> {
    let node_id = node.id;
    let role = node.message.role.to_string();
    let content_bytes = node.message.content.len();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "db.message.upsert",
        session_id = session_id,
        node_id = node_id,
        role = role,
        content_bytes = content_bytes
        ; {
            let sp = conn.savepoint().map_err(|source| DbError::Query {
                context: "failed to open savepoint for upsert_message".to_owned(),
                source,
            })?;
            sp.execute(
                "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id, node.id as i64],
            )
            .map_err(|source| DbError::Query {
                context: "failed to delete message before upsert".to_owned(),
                source,
            })?;
            sp.execute(
                "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp, thought_seconds, speaker_slug, pre_turn_action_points)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    node.id as i64,
                    session_id,
                    node.parent.map(|p| p as i64),
                    Option::<i64>::None,
                    node.message.role.to_string(),
                    node.message.content,
                    node.message.timestamp,
                    node.message.thought_seconds.map(i64::from),
                    node.message.speaker,
                    node.message.pre_turn_action_points,
                ],
            )
            .map_err(|source| DbError::Query {
                context: "failed to insert message during upsert".to_owned(),
                source,
            })?;
            sp.commit().map_err(|source| DbError::Query {
                context: "failed to commit upsert_message savepoint".to_owned(),
                source,
            })?;
            Ok(())
        }
    )
}

pub fn update_head(conn: &Connection, session_id: &str, head_id: Option<NodeId>) -> Result<()> {
    let now = now_iso8601();
    let head_id_display = head_id
        .map(|h| h.to_string())
        .unwrap_or_else(|| "none".to_owned());
    let result = conn
        .execute(
            "UPDATE sessions SET head_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![head_id.map(|h| h as i64), now, session_id],
        )
        .map_err(|source| DbError::Query {
            context: "failed to update session head".to_owned(),
            source,
        });
    match &result {
        Ok(affected) => tracing::debug!(
            session_id = session_id,
            head_id = head_id_display,
            result = "ok",
            affected = affected,
            "db.session.head"
        ),
        Err(err) => tracing::error!(
            session_id = session_id,
            result = "error",
            error = %err,
            "db.session.head"
        ),
    }
    result.map(|_| ())
}

pub fn update_preferred_child(
    conn: &Connection,
    session_id: &str,
    parent_id: NodeId,
    child_id: NodeId,
) -> Result<()> {
    let result = conn
        .execute(
            "UPDATE messages SET preferred_child_id = ?1 WHERE session_id = ?2 AND id = ?3",
            params![child_id as i64, session_id, parent_id as i64],
        )
        .map_err(|source| DbError::Query {
            context: "failed to update preferred_child".to_owned(),
            source,
        });
    match &result {
        Ok(affected) => tracing::debug!(
            session_id = session_id,
            parent_id = parent_id,
            child_id = child_id,
            result = "ok",
            affected = affected,
            "db.session.preferred_child"
        ),
        Err(err) => tracing::error!(
            session_id = session_id,
            parent_id = parent_id,
            child_id = child_id,
            result = "error",
            error = %err,
            "db.session.preferred_child"
        ),
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use crate::db::migrations::run_migrations;
    use libllm_core::session::{Message, MessageTree, Node, Role, Session};

    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn make_session_with_messages() -> Session {
        let nodes = vec![
            Node {
                id: 0,
                parent: None,
                children: vec![1],
                message: Message {
                    role: Role::User,
                    content: "Hello".to_owned(),
                    timestamp: "2026-01-01T00:00:00Z".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
            Node {
                id: 1,
                parent: Some(0),
                children: vec![],
                message: Message {
                    role: Role::Assistant,
                    content: "Hi there!".to_owned(),
                    timestamp: "2026-01-01T00:00:01Z".to_owned(),
                    thought_seconds: Some(7),
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
        ];
        let tree = MessageTree::from_parts(nodes, Some(1), HashMap::new());
        Session {
            tree,
            model: Some("test-model".to_owned()),
            template: Some("chatml".to_owned()),
            system_prompt: Some("You are helpful.".to_owned()),
            character: Some("TestChar".to_owned()),
            worldbooks: vec!["book1".to_owned(), "book2".to_owned()],
            persona: Some("TestUser".to_owned()),
            scenario: None,
            characters: Vec::new(),
            chat_mode: libllm_core::group_chat::ChatMode::default(),
            author_note: None,
        }
    }

    #[test]
    fn negative_thought_seconds_in_row_loads_as_none() {
        let mut conn = setup_db();
        let session = make_session_with_messages();
        insert_session(&mut conn, "sess-neg", &session).unwrap();

        conn.execute(
            "UPDATE messages SET thought_seconds = -1 WHERE session_id = 'sess-neg' AND id = 1",
            [],
        )
        .unwrap();

        let loaded = load_session(&conn, "sess-neg").unwrap();
        let node1 = loaded.tree.node(1).unwrap();
        assert_eq!(node1.message.thought_seconds, None);
    }

    #[test]
    fn session_round_trip() {
        let mut conn = setup_db();
        let session = make_session_with_messages();

        insert_session(&mut conn, "sess-1", &session).unwrap();
        let loaded = load_session(&conn, "sess-1").unwrap();

        assert_eq!(loaded.model, session.model);
        assert_eq!(loaded.template, session.template);
        assert_eq!(loaded.system_prompt, session.system_prompt);
        assert_eq!(loaded.character, session.character);
        assert_eq!(loaded.persona, session.persona);
        assert_eq!(loaded.worldbooks, session.worldbooks);
        assert_eq!(loaded.tree.head(), Some(1));
        assert_eq!(loaded.tree.node_count(), 2);

        let node0 = loaded.tree.node(0).unwrap();
        assert_eq!(node0.message.content, "Hello");
        assert_eq!(node0.message.role, Role::User);
        assert_eq!(node0.parent, None);
        assert_eq!(node0.children, vec![1]);

        let node1 = loaded.tree.node(1).unwrap();
        assert_eq!(node1.message.content, "Hi there!");
        assert_eq!(node1.message.role, Role::Assistant);
        assert_eq!(node1.parent, Some(0));
        assert_eq!(node1.message.thought_seconds, Some(7));
    }

    #[test]
    fn branching_tree_round_trip() {
        let mut conn = setup_db();

        let mut preferred_child = HashMap::new();
        preferred_child.insert(0usize, 2usize);

        let nodes = vec![
            Node {
                id: 0,
                parent: None,
                children: vec![1, 2],
                message: Message {
                    role: Role::User,
                    content: "Hello".to_owned(),
                    timestamp: "2026-01-01T00:00:00Z".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
            Node {
                id: 1,
                parent: Some(0),
                children: vec![],
                message: Message {
                    role: Role::Assistant,
                    content: "Response A".to_owned(),
                    timestamp: "2026-01-01T00:00:01Z".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
            Node {
                id: 2,
                parent: Some(0),
                children: vec![3],
                message: Message {
                    role: Role::Assistant,
                    content: "Response B".to_owned(),
                    timestamp: "2026-01-01T00:00:02Z".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
            Node {
                id: 3,
                parent: Some(2),
                children: vec![],
                message: Message {
                    role: Role::User,
                    content: "Follow up".to_owned(),
                    timestamp: "2026-01-01T00:00:03Z".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
        ];

        let tree = MessageTree::from_parts(nodes, Some(3), preferred_child);
        let session = Session {
            tree,
            model: None,
            template: None,
            system_prompt: None,
            character: None,
            worldbooks: vec![],
            persona: None,
            scenario: None,
            characters: Vec::new(),
            chat_mode: libllm_core::group_chat::ChatMode::default(),
            author_note: None,
        };

        insert_session(&mut conn, "branching", &session).unwrap();
        let loaded = load_session(&conn, "branching").unwrap();

        assert_eq!(loaded.tree.head(), Some(3));
        assert_eq!(loaded.tree.node_count(), 4);

        let root = loaded.tree.node(0).unwrap();
        assert_eq!(root.children.len(), 2);
        assert!(root.children.contains(&1));
        assert!(root.children.contains(&2));

        let node2 = loaded.tree.node(2).unwrap();
        assert_eq!(node2.children, vec![3]);
        assert_eq!(node2.parent, Some(0));

        assert_eq!(loaded.tree.preferred_child_map().get(&0), Some(&2),);
    }

    #[test]
    fn list_sessions_ordering_and_fields() {
        let mut conn = setup_db();

        let session1 = make_session_with_messages();
        insert_session(&mut conn, "sess-1", &session1).unwrap();

        let session2 = Session {
            tree: MessageTree::new(),
            model: None,
            template: None,
            system_prompt: None,
            character: None,
            worldbooks: vec![],
            persona: None,
            scenario: None,
            characters: Vec::new(),
            chat_mode: libllm_core::group_chat::ChatMode::default(),
            author_note: None,
        };
        insert_session(&mut conn, "sess-2", &session2).unwrap();

        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = 'sess-1'",
            params!["2026-01-01T00:00:00Z"],
        )
        .unwrap();
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = 'sess-2'",
            params!["2026-01-02T00:00:00Z"],
        )
        .unwrap();

        let entries = list_sessions(&conn).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "sess-2");
        assert_eq!(entries[0].display_name, "Assistant");
        assert_eq!(entries[0].message_count, 0);
        assert_eq!(entries[0].updated_at, "2026-01-02T00:00:00Z");

        assert_eq!(entries[1].id, "sess-1");
        assert_eq!(entries[1].display_name, "TestChar");
        assert_eq!(entries[1].message_count, 2);
        assert_eq!(entries[1].updated_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn delete_session_cascades() {
        let mut conn = setup_db();
        let session = make_session_with_messages();
        insert_session(&mut conn, "to-delete", &session).unwrap();

        delete_session(&conn, "to-delete").unwrap();

        assert!(load_session(&conn, "to-delete").is_err());

        let msg_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                params!["to-delete"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(msg_count, 0);

        let wb_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_worldbooks WHERE session_id = ?1",
                params!["to-delete"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wb_count, 0);
    }

    #[test]
    fn save_session_preserves_file_summaries() {
        let mut conn = setup_db();
        let session = make_session_with_messages();
        insert_session(&mut conn, "sess-fs", &session).unwrap();

        conn.execute(
            "INSERT INTO file_summaries
             (session_id, content_hash, basename, summary, status, created_at, updated_at)
             VALUES ('sess-fs', 'hash-a', 'a.md', '', 'pending', 'now', 'now')",
            [],
        )
        .unwrap();

        save_session(&mut conn, "sess-fs", &session).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_summaries
                 WHERE session_id = 'sess-fs' AND content_hash = 'hash-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "autosave must not cascade-delete file_summaries");
    }

    #[test]
    fn save_and_load_group_session_round_trips_attachments_and_settings() {
        use libllm_core::group_chat::{CharacterAttachment, ChatMode};
        use libllm_core::session::{MessageTree, Session};

        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();

        let session = Session {
            tree: MessageTree::new(),
            model: None,
            template: None,
            system_prompt: None,
            character: None,
            characters: vec![
                CharacterAttachment {
                    slug: "alice".to_owned(),
                    talkativeness: 0.7,
                    action_points: 0.3,
                    spoke_this_round: false,
                },
                CharacterAttachment {
                    slug: "bob".to_owned(),
                    talkativeness: 0.4,
                    action_points: 0.0,
                    spoke_this_round: false,
                },
                CharacterAttachment {
                    slug: "charlie".to_owned(),
                    talkativeness: 0.6,
                    action_points: 0.9,
                    spoke_this_round: false,
                },
            ],
            chat_mode: ChatMode::WeightedRandom,
            scenario: None,
            worldbooks: vec![],
            persona: None,
            author_note: None,
        };

        insert_session(&mut conn, "g1", &session).unwrap();
        let loaded = load_session(&conn, "g1").unwrap();

        assert_eq!(loaded.characters.len(), 3);
        assert_eq!(loaded.characters[0].slug, "alice");
        assert!((loaded.characters[0].talkativeness - 0.7).abs() < 1e-6);
        assert!((loaded.characters[0].action_points - 0.3).abs() < 1e-6);
        assert_eq!(loaded.characters[2].slug, "charlie");
        assert!((loaded.characters[2].action_points - 0.9).abs() < 1e-6);
        assert!(matches!(loaded.chat_mode, ChatMode::WeightedRandom));
    }

    #[test]
    fn save_solo_session_mirrors_character_column() {
        use libllm_core::group_chat::CharacterAttachment;
        use libllm_core::session::{MessageTree, Session};

        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();

        let session = Session {
            tree: MessageTree::new(),
            characters: vec![CharacterAttachment::new("alice")],
            ..Default::default()
        };
        insert_session(&mut conn, "s1", &session).unwrap();

        let mirror: Option<String> = conn
            .query_row(
                "SELECT character FROM sessions WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mirror.as_deref(), Some("alice"));

        let loaded = load_session(&conn, "s1").unwrap();
        assert_eq!(loaded.characters.len(), 1);
        assert_eq!(loaded.characters[0].slug, "alice");
    }

    #[test]
    fn save_group_session_clears_character_mirror() {
        use libllm_core::group_chat::CharacterAttachment;
        use libllm_core::session::{MessageTree, Session};

        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();

        let session = Session {
            tree: MessageTree::new(),
            characters: vec![
                CharacterAttachment::new("alice"),
                CharacterAttachment::new("bob"),
            ],
            ..Default::default()
        };
        insert_session(&mut conn, "g2", &session).unwrap();

        let mirror: Option<String> = conn
            .query_row(
                "SELECT character FROM sessions WHERE id = 'g2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            mirror.is_none(),
            "character column should be NULL for group sessions"
        );
    }

    #[test]
    fn save_session_replaces_attachment_set_on_re_save() {
        use libllm_core::group_chat::CharacterAttachment;
        use libllm_core::session::{MessageTree, Session};

        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();

        let mut session = Session {
            tree: MessageTree::new(),
            characters: vec![
                CharacterAttachment::new("alice"),
                CharacterAttachment::new("bob"),
            ],
            ..Default::default()
        };
        insert_session(&mut conn, "g3", &session).unwrap();

        session.characters = vec![CharacterAttachment::new("alice")];
        save_session(&mut conn, "g3", &session).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_characters WHERE session_id = 'g3'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let loaded = load_session(&conn, "g3").unwrap();
        assert_eq!(loaded.characters.len(), 1);
        assert_eq!(loaded.characters[0].slug, "alice");
    }

    #[test]
    fn load_session_synthesizes_attachment_from_legacy_character_column() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, character, created_at, updated_at)
             VALUES ('legacy', 'alice', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM session_characters WHERE session_id = 'legacy'",
            [],
        )
        .unwrap();

        let loaded = load_session(&conn, "legacy").unwrap();
        assert_eq!(loaded.characters.len(), 1);
        assert_eq!(loaded.characters[0].slug, "alice");
        assert!((loaded.characters[0].talkativeness - 1.0).abs() < 1e-6);
    }

    #[test]
    fn save_and_load_message_round_trips_speaker_and_action_points() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn).unwrap();

        let mut tree = MessageTree::new();
        let user_msg = Message::new(Role::User, "hello".to_owned());
        let user_id = tree.push(None, user_msg);

        let mut alice_msg = Message::new(Role::Assistant, "Alice: hi".to_owned());
        alice_msg.speaker = Some("alice".to_owned());
        alice_msg.pre_turn_action_points = Some(r#"{"alice":0.2,"bob":0.5}"#.to_owned());
        tree.push(Some(user_id), alice_msg);

        let session = Session {
            tree,
            ..Default::default()
        };
        insert_session(&mut conn, "m1", &session).unwrap();

        let loaded = super::load_session(&conn, "m1").unwrap();
        let nodes = loaded.tree.nodes();
        let assistant = nodes
            .iter()
            .find(|n| matches!(n.message.role, Role::Assistant))
            .unwrap();
        assert_eq!(assistant.message.speaker.as_deref(), Some("alice"));
        assert_eq!(
            assistant.message.pre_turn_action_points.as_deref(),
            Some(r#"{"alice":0.2,"bob":0.5}"#)
        );

        let user = nodes
            .iter()
            .find(|n| matches!(n.message.role, Role::User))
            .unwrap();
        assert!(user.message.speaker.is_none());
        assert!(user.message.pre_turn_action_points.is_none());
    }

    #[test]
    fn upsert_message_and_update_head() {
        let mut conn = setup_db();
        let session = make_session_with_messages();
        insert_session(&mut conn, "sess-upsert", &session).unwrap();

        let new_node = Node {
            id: 2,
            parent: Some(1),
            children: vec![],
            message: Message {
                role: Role::User,
                content: "Another message".to_owned(),
                timestamp: "2026-01-01T00:00:05Z".to_owned(),
                thought_seconds: None,
                speaker: None,
                pre_turn_action_points: None,
            },
        };

        upsert_message(&mut conn, "sess-upsert", &new_node).unwrap();
        update_head(&conn, "sess-upsert", Some(2)).unwrap();

        let loaded = load_session(&conn, "sess-upsert").unwrap();
        assert_eq!(loaded.tree.head(), Some(2));
        assert_eq!(loaded.tree.node_count(), 3);

        let added = loaded.tree.node(2).unwrap();
        assert_eq!(added.message.content, "Another message");
        assert_eq!(added.parent, Some(1));
    }

    #[test]
    fn session_author_note_round_trip_some() {
        let mut conn = setup_db();
        let mut session = make_session_with_messages();
        session.author_note = Some(libllm_core::author_note::AuthorNote {
            text: "Steer dramatic.".to_owned(),
            depth: 6,
            at_top: false,
        });

        insert_session(&mut conn, "sess-an", &session).unwrap();
        let loaded = load_session(&conn, "sess-an").unwrap();

        assert_eq!(loaded.author_note, session.author_note);
    }

    #[test]
    fn session_author_note_round_trip_none() {
        let mut conn = setup_db();
        let session = make_session_with_messages();
        assert!(session.author_note.is_none());

        insert_session(&mut conn, "sess-no-an", &session).unwrap();
        let loaded = load_session(&conn, "sess-no-an").unwrap();

        assert_eq!(loaded.author_note, None);
    }

    #[test]
    fn session_author_note_save_round_trip_after_edit() {
        let mut conn = setup_db();
        let mut session = make_session_with_messages();
        insert_session(&mut conn, "sess-edit", &session).unwrap();

        session.author_note = Some(libllm_core::author_note::AuthorNote {
            text: "later note".to_owned(),
            depth: 2,
            at_top: true,
        });
        save_session(&mut conn, "sess-edit", &session).unwrap();

        let loaded = load_session(&conn, "sess-edit").unwrap();
        assert_eq!(loaded.author_note, session.author_note);
    }

    #[test]
    fn upsert_message_replace_does_not_leave_stale_fts_terms() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();

        super::upsert_message(
            &mut conn,
            "s1",
            &libllm_core::session::Node {
                id: 0,
                parent: None,
                children: vec![],
                message: libllm_core::session::Message {
                    role: libllm_core::session::Role::User,
                    content: "uniqueold".to_owned(),
                    timestamp: "now".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
        )
        .unwrap();

        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'uniqueold'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, 1, "first upsert should index term");

        super::upsert_message(
            &mut conn,
            "s1",
            &libllm_core::session::Node {
                id: 0,
                parent: None,
                children: vec![],
                message: libllm_core::session::Message {
                    role: libllm_core::session::Role::User,
                    content: "uniquenew".to_owned(),
                    timestamp: "now".to_owned(),
                    thought_seconds: None,
                    speaker: None,
                    pre_turn_action_points: None,
                },
            },
        )
        .unwrap();

        let stale: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'uniqueold'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "stale FTS term should be removed after replace");

        let fresh: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'uniquenew'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fresh, 1, "new FTS term should be indexed after replace");
    }

    #[test]
    fn load_session_characters_falls_back_to_legacy_column() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('leg', 'now', 'now')",
            [],
        )
        .unwrap();

        let characters = super::load_session_characters(&conn, "leg", Some("alice")).unwrap();

        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].slug, "alice");
        assert!((characters[0].talkativeness - 1.0).abs() < 1e-6);
        assert!((characters[0].action_points - 0.0).abs() < 1e-6);
        assert!(!characters[0].spoke_this_round);
    }

    #[test]
    fn load_session_characters_prefers_attachment_rows() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('gc', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points)
             VALUES ('gc', 'bob', 0, 0.8, 0.2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points)
             VALUES ('gc', 'alice', 1, 0.5, 0.0)",
            [],
        )
        .unwrap();

        let characters = super::load_session_characters(&conn, "gc", Some("legacy")).unwrap();

        assert_eq!(characters.len(), 2);
        assert!(!characters.iter().any(|c| c.slug == "legacy"));
        assert_eq!(characters[0].slug, "bob");
        assert_eq!(characters[1].slug, "alice");
    }

    #[test]
    fn load_session_worldbooks_returns_attached_slugs() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('wb', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_worldbooks (session_id, worldbook_slug) VALUES ('wb', 'alpha')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_worldbooks (session_id, worldbook_slug) VALUES ('wb', 'beta')",
            [],
        )
        .unwrap();

        let worldbooks = super::load_session_worldbooks(&conn, "wb").unwrap();

        assert_eq!(worldbooks, vec!["alpha".to_owned(), "beta".to_owned()]);
    }

    #[test]
    fn load_message_tree_reconstructs_branches() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('br', 'now', 'now')",
            [],
        )
        .unwrap();
        // parent node (id=0)
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (0, 'br', NULL, 2, 'user', 'root', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        // child A (id=1)
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (1, 'br', 0, NULL, 'assistant', 'branch-a', '2026-01-01T00:00:01Z')",
            [],
        )
        .unwrap();
        // child B (id=2) — preferred child of root, head of the tree
        conn.execute(
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (2, 'br', 0, NULL, 'assistant', 'branch-b', '2026-01-01T00:00:02Z')",
            [],
        )
        .unwrap();

        let tree = super::load_message_tree(&conn, "br", Some(2)).unwrap();

        assert_eq!(tree.head(), Some(2));
        assert_eq!(tree.node_count(), 3);

        let root = tree.node(0).unwrap();
        assert_eq!(root.children.len(), 2);
        assert!(root.children.contains(&1));
        assert!(root.children.contains(&2));

        assert_eq!(tree.preferred_child_map().get(&0), Some(&2));
    }
}
