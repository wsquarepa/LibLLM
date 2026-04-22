//! Session persistence: insert, load, list, delete, and incremental message updates.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::session::{Message, MessageTree, Node, NodeId, Role, Session, now_iso8601};

type SessionRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

pub struct SessionListEntry {
    pub id: String,
    pub display_name: String,
    pub message_count: usize,
    pub updated_at: String,
}

fn display_name_from_character(character: Option<&str>) -> String {
    character.unwrap_or("Assistant").to_owned()
}

fn insert_session_row(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    let now = now_iso8601();
    let display_name = display_name_from_character(session.character.as_deref());
    let head_id = session.tree.head().map(|h| h as i64);

    conn.execute(
        "INSERT INTO sessions (id, display_name, model, template, system_prompt, character, persona, head_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            display_name,
            session.model,
            session.template,
            session.system_prompt,
            session.character,
            session.persona,
            head_id,
            now,
            now,
        ],
    )
    .context("failed to insert session row")?;
    Ok(())
}

/// Upsert the `sessions` row without deleting it.
/// Preserves the existing row id so `ON DELETE CASCADE` dependants
/// (messages, session_worldbooks, file_summaries) are not wiped.
fn upsert_session_row(conn: &Connection, id: &str, session: &Session) -> Result<()> {
    let now = now_iso8601();
    let display_name = display_name_from_character(session.character.as_deref());
    let head_id = session.tree.head().map(|h| h as i64);

    conn.execute(
        "INSERT INTO sessions (id, display_name, model, template, system_prompt, character, persona, head_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(id) DO UPDATE SET
            display_name = excluded.display_name,
            model = excluded.model,
            template = excluded.template,
            system_prompt = excluded.system_prompt,
            character = excluded.character,
            persona = excluded.persona,
            head_id = excluded.head_id,
            updated_at = excluded.updated_at",
        params![
            id,
            display_name,
            session.model,
            session.template,
            session.system_prompt,
            session.character,
            session.persona,
            head_id,
            now,
        ],
    )
    .context("failed to upsert session row")?;
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
            "INSERT INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                node.id as i64,
                id,
                node.parent.map(|p| p as i64),
                preferred_child_id,
                node.message.role.to_string(),
                node.message.content,
                node.message.timestamp,
            ],
        )
        .context("failed to insert message row")?;
    }

    for worldbook_slug in &session.worldbooks {
        conn.execute(
            "INSERT INTO session_worldbooks (session_id, worldbook_slug) VALUES (?1, ?2)",
            params![id, worldbook_slug],
        )
        .context("failed to insert session_worldbooks row")?;
    }

    Ok(())
}

pub fn insert_session(conn: &mut Connection, id: &str, session: &Session) -> Result<()> {
    let node_count = session.tree.node_count();
    let worldbook_count = session.worldbooks.len();
    crate::timed_result!(
        tracing::Level::INFO,
        "db.session.insert",
        session_id = id,
        node_count = node_count,
        worldbook_count = worldbook_count
        ; {
            let sp = conn.savepoint().context("failed to begin savepoint")?;
            insert_session_row(&sp, id, session)?;
            write_messages_and_worldbooks(&sp, id, session)?;
            sp.commit().context("failed to commit session insert")?;
            Ok(())
        }
    )
}

pub fn save_session(conn: &mut Connection, id: &str, session: &Session) -> Result<()> {
    let node_count = session.tree.node_count();
    let worldbook_count = session.worldbooks.len();
    crate::timed_result!(
        tracing::Level::INFO,
        "db.session.save",
        session_id = id,
        node_count = node_count,
        worldbook_count = worldbook_count
        ; {
            let sp = conn.savepoint().context("failed to begin savepoint")?;
            upsert_session_row(&sp, id, session)?;
            sp.execute("DELETE FROM messages WHERE session_id = ?1", params![id])
                .context("failed to clear messages")?;
            sp.execute(
                "DELETE FROM session_worldbooks WHERE session_id = ?1",
                params![id],
            )
            .context("failed to clear session_worldbooks")?;
            write_messages_and_worldbooks(&sp, id, session)?;
            sp.commit().context("failed to commit session save")?;
            Ok(())
        }
    )
}

pub fn session_exists(conn: &Connection, id: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .context("failed to check session existence")?;
    tracing::info!(
        session_id = id,
        result = "ok",
        found = count > 0,
        "db.session.exists"
    );
    Ok(count > 0)
}

pub fn load_session(conn: &Connection, id: &str) -> Result<Session> {
    crate::timed_result!(tracing::Level::INFO, "db.session.load", session_id = id ; {
            let (model, template, system_prompt, character, persona, head_id): SessionRow = conn
                .query_row(
                    "SELECT model, template, system_prompt, character, persona, head_id
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
                        ))
                    },
                )
                .with_context(|| format!("session not found: {id}"))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, preferred_child_id, role, content, timestamp
                     FROM messages WHERE session_id = ?1 ORDER BY id",
                )
                .context("failed to prepare message query")?;

            let mut nodes: Vec<Node> = Vec::new();
            let mut preferred_child: HashMap<NodeId, NodeId> = HashMap::new();

            let rows = stmt
                .query_map(params![id], |row| {
                    let msg_id: i64 = row.get(0)?;
                    let parent_id: Option<i64> = row.get(1)?;
                    let preferred_child_id: Option<i64> = row.get(2)?;
                    let role_str: String = row.get(3)?;
                    let content: String = row.get(4)?;
                    let timestamp: String = row.get(5)?;
                    Ok((msg_id, parent_id, preferred_child_id, role_str, content, timestamp))
                })
                .context("failed to query messages")?;

            for row in rows {
                let (msg_id, parent_id, preferred_child_id, role_str, content, timestamp) =
                    row.context("failed to read message row")?;

                let role: Role = role_str
                    .parse()
                    .with_context(|| format!("invalid role in message {msg_id}: {role_str}"))?;

                let node = Node {
                    id: msg_id as usize,
                    parent: parent_id.map(|p| p as usize),
                    children: Vec::new(),
                    message: Message { role, content, timestamp },
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
            let tree = MessageTree::from_parts(nodes, head, preferred_child);

            let mut worldbooks: Vec<String> = Vec::new();
            let mut wb_stmt = conn
                .prepare("SELECT worldbook_slug FROM session_worldbooks WHERE session_id = ?1")
                .context("failed to prepare worldbooks query")?;
            let wb_rows = wb_stmt
                .query_map(params![id], |row| row.get(0))
                .context("failed to query worldbooks")?;
            for wb in wb_rows {
                worldbooks.push(wb.context("failed to read worldbook row")?);
            }

            Ok(Session {
                tree,
                model,
                template,
                system_prompt,
                character,
                worldbooks,
                persona,
            })
    })
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionListEntry>> {
    crate::timed_result!(tracing::Level::INFO, "db.session.list", ; {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.display_name, s.updated_at,
                        COUNT(m.id) AS message_count
                 FROM sessions s
                 LEFT JOIN messages m ON m.session_id = s.id
                 GROUP BY s.id
                 ORDER BY s.updated_at DESC",
            )
            .context("failed to prepare list_sessions query")?;

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
            .context("failed to query sessions")?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.context("failed to read session row")?);
        }
        tracing::info!(session_count = entries.len(), "db.session.list");
        Ok(entries)
    })
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<()> {
    crate::timed_result!(tracing::Level::INFO, "db.session.delete", session_id = id ; {
        let affected = conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .context("failed to delete session")?;
        tracing::info!(session_id = id, affected = affected, "db.session.delete");
        if affected == 0 {
            anyhow::bail!("session not found: {id}");
        }
        Ok(())
    })
}

pub fn upsert_message(conn: &Connection, session_id: &str, node: &Node) -> Result<()> {
    let node_id = node.id;
    let role = node.message.role.to_string();
    let content_bytes = node.message.content.len();
    crate::timed_result!(
        tracing::Level::INFO,
        "db.message.upsert",
        session_id = session_id,
        node_id = node_id,
        role = role,
        content_bytes = content_bytes
        ; {
            conn.execute(
                "INSERT OR REPLACE INTO messages (id, session_id, parent_id, preferred_child_id, role, content, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    node.id as i64,
                    session_id,
                    node.parent.map(|p| p as i64),
                    Option::<i64>::None,
                    node.message.role.to_string(),
                    node.message.content,
                    node.message.timestamp,
                ],
            )
            .context("failed to upsert message")?;
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
        .context("failed to update session head");
    match &result {
        Ok(affected) => tracing::info!(
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
        .context("failed to update preferred_child");
    match &result {
        Ok(affected) => tracing::info!(
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
    use rusqlite::Connection;

    use crate::db::migrations::run_migrations;
    use crate::session::{Message, MessageTree, Node, Role, Session};

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
        }
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
            },
        };

        upsert_message(&conn, "sess-upsert", &new_node).unwrap();
        update_head(&conn, "sess-upsert", Some(2)).unwrap();

        let loaded = load_session(&conn, "sess-upsert").unwrap();
        assert_eq!(loaded.tree.head(), Some(2));
        assert_eq!(loaded.tree.node_count(), 3);

        let added = loaded.tree.node(2).unwrap();
        assert_eq!(added.message.content, "Another message");
        assert_eq!(added.parent, Some(1));
    }
}
