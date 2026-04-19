//! Conversation session types with a branching message tree backed by an arena allocator.

use std::collections::HashMap;
use std::fmt;
#[cfg(debug_assertions)]
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::Database;

/// Controls whether and how a session is persisted to the database.
#[derive(Clone)]
pub enum SaveMode {
    /// Session is ephemeral and will not be saved.
    None,
    /// Session is actively persisted to the database under the given ID.
    Database { id: String },
    /// Session has a database ID but cannot be saved until a passkey is provided.
    PendingPasskey { id: String },
}

impl SaveMode {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Database { id } => Some(id),
            Self::PendingPasskey { id } => Some(id),
        }
    }

    pub fn set_id(&mut self, new_id: String) {
        match self {
            Self::None => {}
            Self::Database { id } => *id = new_id,
            Self::PendingPasskey { id } => *id = new_id,
        }
    }

    pub fn needs_passkey(&self) -> bool {
        matches!(self, Self::PendingPasskey { .. })
    }
}

/// Lightweight session metadata used for sidebar display and session switching.
pub struct SessionEntry {
    pub id: String,
    pub display_name: String,
    pub message_count: Option<usize>,
    pub last_assistant_preview: Option<String>,
    pub sidebar_label: String,
    pub sidebar_preview: Option<String>,
    pub is_new_chat: bool,
}

/// The speaker role for a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Summary,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Assistant => f.write_str("assistant"),
            Self::System => f.write_str("system"),
            Self::Summary => f.write_str("summary"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "summary" => Ok(Self::Summary),
            _ => anyhow::bail!("unknown role: {s}"),
        }
    }
}

/// A single chat message with role, content text, and ISO-8601 timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
}

impl Message {
    pub fn new(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            timestamp: now_iso8601(),
        }
    }
}

/// Index into the `MessageTree` arena, identifying a single node.
pub type NodeId = usize;

/// An arena-allocated node in the message tree, holding one message and its parent/child links.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub message: Message,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Default)]
struct CacheDebugState {
    rebuild_count: u64,
    total_rebuild_us: u128,
    last_rebuild_us: u128,
    branch_hits: std::cell::Cell<u64>,
    user_branch_hits: std::cell::Cell<u64>,
    deepest_hits: std::cell::Cell<u64>,
    first_preview_hits: std::cell::Cell<u64>,
}

#[derive(Debug, Clone, Default)]
struct TreeRuntimeCache {
    branch_ids: Vec<NodeId>,
    user_branch_ids: Vec<NodeId>,
    deepest_branch_info: Option<(usize, usize)>,
    last_assistant_preview: Option<String>,
    #[cfg(debug_assertions)]
    debug: CacheDebugState,
}

/// Arena-backed branching message tree where `/retry` and `/edit` create sibling branches.
///
/// Nodes are stored in a flat `Vec<Node>` indexed by `NodeId`. The `head` points to the
/// currently active leaf. `preferred_child` tracks which branch was last visited at each
/// fork so that `switch_to` can restore the user's previous path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTree {
    nodes: Vec<Node>,
    head: Option<NodeId>,
    #[serde(default)]
    preferred_child: HashMap<NodeId, NodeId>,
    #[serde(skip)]
    runtime: TreeRuntimeCache,
}

impl MessageTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            head: None,
            preferred_child: HashMap::new(),
            runtime: TreeRuntimeCache::default(),
        }
    }

    pub fn from_parts(
        nodes: Vec<Node>,
        head: Option<NodeId>,
        preferred_child: HashMap<NodeId, NodeId>,
    ) -> Self {
        let mut tree = Self {
            nodes,
            head,
            preferred_child,
            runtime: TreeRuntimeCache::default(),
        };
        tree.rehydrate_runtime_state();
        tree
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn preferred_child_map(&self) -> &HashMap<NodeId, NodeId> {
        &self.preferred_child
    }

    #[cfg(debug_assertions)]
    fn bump_cache_hit(&self, accessor: &'static str, counter: &std::cell::Cell<u64>) {
        let hits = counter.get() + 1;
        counter.set(hits);
        if hits.is_power_of_two() {
            tracing::debug!(
                phase = "hit",
                accessor = accessor,
                hits = hits,
                rebuilds = self.runtime.debug.rebuild_count,
                "session.cache",
            );
        }
    }

    fn validate_preferred_children(&mut self) {
        let nodes = &self.nodes;
        self.preferred_child.retain(|&parent_id, child_id| {
            let child_id = *child_id;
            parent_id < nodes.len()
                && child_id < nodes.len()
                && nodes[parent_id].children.contains(&child_id)
        });
    }

    pub fn update_preferred_children(&mut self) {
        let Some(head) = self.head else { return };
        let mut current = head;
        while let Some(pid) = self.nodes[current].parent {
            self.preferred_child.insert(pid, current);
            current = pid;
        }
    }

    fn refresh_runtime_caches(&mut self) {
        #[cfg(debug_assertions)]
        let rebuild_start = Instant::now();
        self.runtime.branch_ids.clear();
        self.runtime.user_branch_ids.clear();
        self.runtime.deepest_branch_info = None;
        self.runtime.last_assistant_preview = None;

        let Some(head) = self.head else {
            return;
        };

        let max_steps = self.nodes.len();
        let mut current = head;
        for _ in 0..max_steps {
            self.runtime.branch_ids.push(current);
            match self.nodes.get(current).and_then(|n| n.parent) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.runtime.branch_ids.reverse();

        for &id in &self.runtime.branch_ids {
            let node = &self.nodes[id];
            if node.message.role == Role::User {
                self.runtime.user_branch_ids.push(id);
            }
            if node.message.role == Role::Assistant {
                self.runtime.last_assistant_preview = Some(node.message.content.clone());
            }
        }

        self.runtime.deepest_branch_info = self.runtime.branch_ids.iter().rev().find_map(|&id| {
            let info = self.sibling_info(id);
            (info.1 > 1).then_some(info)
        });

        #[cfg(debug_assertions)]
        {
            let elapsed_us = rebuild_start.elapsed().as_micros();
            self.runtime.debug.rebuild_count += 1;
            self.runtime.debug.total_rebuild_us += elapsed_us;
            self.runtime.debug.last_rebuild_us = elapsed_us;
            let elapsed_ms = elapsed_us as f64 / 1000.0;
            let total_elapsed_ms = self.runtime.debug.total_rebuild_us as f64 / 1000.0;
            tracing::debug!(
                phase = "rebuild",
                rebuilds = self.runtime.debug.rebuild_count,
                elapsed_ms = elapsed_ms,
                total_elapsed_ms = total_elapsed_ms,
                node_count = self.nodes.len(),
                branch_count = self.runtime.branch_ids.len(),
                user_branch_count = self.runtime.user_branch_ids.len(),
                branch_hits = self.runtime.debug.branch_hits.get(),
                user_branch_hits = self.runtime.debug.user_branch_hits.get(),
                deepest_hits = self.runtime.debug.deepest_hits.get(),
                first_preview_hits = self.runtime.debug.first_preview_hits.get(),
                "session.cache",
            );
        }
    }

    fn rehydrate_runtime_state(&mut self) {
        if let Some(head) = self.head
            && head >= self.nodes.len()
        {
            self.head = None;
        }
        self.validate_preferred_children();
        if self.preferred_child.is_empty() {
            self.update_preferred_children();
        }
        self.refresh_runtime_caches();
    }

    pub fn head(&self) -> Option<NodeId> {
        self.head
    }

    fn update_head(&mut self, new_head: Option<NodeId>) {
        self.head = new_head;
        self.update_preferred_children();
        self.refresh_runtime_caches();
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn set_message_content(&mut self, id: NodeId, content: String) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        node.message.content = content;
        self.refresh_runtime_caches();
        true
    }

    pub fn duplicate_subtree(&mut self, root_id: NodeId) -> Option<NodeId> {
        let parent = self.nodes.get(root_id)?.parent;
        let mut queue = std::collections::VecDeque::new();
        let mut id_map = HashMap::new();
        let new_root = self.insert(parent, self.nodes[root_id].message.clone());
        id_map.insert(root_id, new_root);
        queue.push_back((root_id, new_root));
        while let Some((orig, new_parent)) = queue.pop_front() {
            let children = self.nodes[orig].children.clone();
            for child_id in children {
                let new_child = self.insert(Some(new_parent), self.nodes[child_id].message.clone());
                id_map.insert(child_id, new_child);
                queue.push_back((child_id, new_child));
            }
        }

        for (&orig_id, &new_id) in &id_map {
            let Some(&orig_preferred_child) = self.preferred_child.get(&orig_id) else {
                continue;
            };
            let Some(&new_preferred_child) = id_map.get(&orig_preferred_child) else {
                continue;
            };
            self.preferred_child.insert(new_id, new_preferred_child);
        }

        self.refresh_runtime_caches();
        Some(new_root)
    }

    fn insert(&mut self, parent: Option<NodeId>, message: Message) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            id,
            parent,
            children: Vec::new(),
            message,
        });
        if let Some(parent_id) = parent {
            self.nodes[parent_id].children.push(id);
        }
        id
    }

    pub fn push(&mut self, parent: Option<NodeId>, message: Message) -> NodeId {
        let id = self.insert(parent, message);
        self.head = Some(id);
        self.update_preferred_children();
        self.refresh_runtime_caches();
        id
    }

    /// Splices a new node into the tree between `parent_id` and its current children, so
    /// that after the call `parent_id.children == [new_id]` and `new_id.children` is the
    /// former child list (with each reparented to the new node). Head is preserved — the
    /// descendants below the former children remain reachable from `head`.
    ///
    /// Used by auto-summarization to replace a prefix of the conversation with a single
    /// `Role::Summary` node while keeping the continuation in-path.
    pub fn splice_between(&mut self, parent_id: NodeId, message: Message) -> NodeId {
        let old_children = std::mem::take(&mut self.nodes[parent_id].children);
        let new_id = self.nodes.len();
        self.nodes.push(Node {
            id: new_id,
            parent: Some(parent_id),
            children: old_children.clone(),
            message,
        });
        self.nodes[parent_id].children = vec![new_id];
        for &child_id in &old_children {
            self.nodes[child_id].parent = Some(new_id);
        }
        self.preferred_child.remove(&parent_id);
        self.update_preferred_children();
        self.refresh_runtime_caches();
        new_id
    }

    pub fn branch_path(&self) -> Vec<&Message> {
        self.messages_for_ids(self.current_branch_ids())
    }

    pub fn current_branch_ids(&self) -> &[NodeId] {
        #[cfg(debug_assertions)]
        self.bump_cache_hit("current_branch_ids", &self.runtime.debug.branch_hits);
        &self.runtime.branch_ids
    }

    pub fn current_user_branch_ids(&self) -> &[NodeId] {
        #[cfg(debug_assertions)]
        self.bump_cache_hit(
            "current_user_branch_ids",
            &self.runtime.debug.user_branch_hits,
        );
        &self.runtime.user_branch_ids
    }

    pub fn current_deepest_branch_info(&self) -> Option<(usize, usize)> {
        #[cfg(debug_assertions)]
        self.bump_cache_hit(
            "current_deepest_branch_info",
            &self.runtime.debug.deepest_hits,
        );
        self.runtime.deepest_branch_info
    }

    pub fn current_last_assistant_preview(&self) -> Option<&str> {
        #[cfg(debug_assertions)]
        self.bump_cache_hit(
            "current_last_assistant_preview",
            &self.runtime.debug.first_preview_hits,
        );
        self.runtime.last_assistant_preview.as_deref()
    }

    pub fn sibling_info(&self, id: NodeId) -> (usize, usize) {
        let parent = self.nodes[id].parent;
        match parent {
            Some(pid) => {
                let siblings = &self.nodes[pid].children;
                let index = siblings.iter().position(|&c| c == id).unwrap_or(0);
                (index, siblings.len())
            }
            None => {
                let roots: Vec<NodeId> = self
                    .nodes
                    .iter()
                    .filter(|n| n.parent.is_none() && self.is_reachable(n.id))
                    .map(|n| n.id)
                    .collect();
                let index = roots.iter().position(|&r| r == id).unwrap_or(0);
                (index, roots.len())
            }
        }
    }

    pub fn switch_to(&mut self, id: NodeId) {
        if id >= self.nodes.len() {
            return;
        }
        let mut current = id;
        while !self.nodes[current].children.is_empty() {
            current = self
                .preferred_child
                .get(&current)
                .filter(|&&c| self.nodes[current].children.contains(&c))
                .copied()
                .unwrap_or(self.nodes[current].children[0]);
        }
        self.update_head(Some(current));
    }

    pub fn switch_sibling(&mut self, offset: isize) {
        if self.head.is_none() {
            return;
        };

        let branch_node = self
            .current_branch_ids()
            .iter()
            .rev()
            .find(|&&id| {
                let (_, total) = self.sibling_info(id);
                total > 1
            })
            .copied();

        let Some(node_id) = branch_node else { return };

        let parent = self.nodes[node_id].parent;
        let siblings = match parent {
            Some(pid) => self.nodes[pid].children.clone(),
            None => self
                .nodes
                .iter()
                .filter(|n| n.parent.is_none())
                .map(|n| n.id)
                .collect(),
        };

        let current_idx = siblings.iter().position(|&c| c == node_id).unwrap_or(0);
        let new_idx = (current_idx as isize + offset).rem_euclid(siblings.len() as isize) as usize;
        let new_node = siblings[new_idx];

        self.switch_to(new_node);
    }

    pub fn retreat_head(&mut self) -> Option<&Message> {
        let head = self.head?;
        self.update_head(self.nodes[head].parent);
        Some(&self.nodes[head].message)
    }

    pub fn set_head(&mut self, id: Option<NodeId>) {
        self.update_head(id);
    }

    pub fn pop_head(&mut self) -> Option<Message> {
        let head = self.head?;
        if !self.nodes[head].children.is_empty() {
            return None;
        }

        let node = &self.nodes[head];
        let parent = node.parent;
        let message = node.message.clone();

        if let Some(pid) = parent {
            self.nodes[pid].children.retain(|&c| c != head);
            if self.preferred_child.get(&pid) == Some(&head) {
                self.preferred_child.remove(&pid);
            }
        }

        self.preferred_child.remove(&head);

        self.update_head(parent);
        Some(message)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.head = None;
        self.preferred_child.clear();
        self.refresh_runtime_caches();
    }

    fn is_reachable(&self, id: NodeId) -> bool {
        if self.nodes[id].parent.is_some() {
            return false;
        }
        !self.nodes[id].children.is_empty() || self.head == Some(id)
    }

    pub fn siblings_of(&self, id: NodeId) -> Vec<NodeId> {
        match self.nodes.get(id).and_then(|n| n.parent) {
            Some(pid) => self.nodes[pid].children.clone(),
            None => self
                .nodes
                .iter()
                .filter(|n| n.parent.is_none() && self.is_reachable(n.id))
                .map(|n| n.id)
                .collect(),
        }
    }

    pub fn messages_for_ids<'a>(&'a self, ids: &[NodeId]) -> Vec<&'a Message> {
        ids.iter().map(|&id| &self.nodes[id].message).collect()
    }

    pub fn from_messages(messages: Vec<Message>) -> Self {
        let mut tree = Self::new();
        let mut parent: Option<NodeId> = None;
        for msg in messages {
            let id = tree.push(parent, msg);
            parent = Some(id);
        }
        tree
    }
}

impl Default for MessageTree {
    fn default() -> Self {
        Self::new()
    }
}

/// A conversation session: a message tree plus metadata (model, character, worldbooks, etc.).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub tree: MessageTree,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub character: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub worldbooks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub persona: Option<String>,
}

impl Session {
    pub fn retreat_trailing_assistant(&mut self) {
        while self.tree.head().is_some_and(|id| {
            self.tree
                .node(id)
                .is_some_and(|n| n.message.role == Role::Assistant)
        }) {
            self.tree.retreat_head();
        }
    }

    pub fn maybe_save(&self, mode: &SaveMode, db: Option<&mut Database>) -> Result<()> {
        match mode {
            SaveMode::None | SaveMode::PendingPasskey { .. } => Ok(()),
            SaveMode::Database { id } => {
                let db = db.ok_or_else(|| anyhow::anyhow!("database not available for save"))?;
                db.save_session(id, self)
            }
        }
    }
}

pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn wall_clock_parts() -> (u64, u64, u64, u64, u64, u64) {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let (year, month, day) = days_to_ymd(secs / 86400);
    (year, month, day, hours, minutes, seconds)
}

pub fn now_compact() -> String {
    let (year, month, day, hours, minutes, seconds) = wall_clock_parts();
    format!("{year:04}{month:02}{day:02}-{hours:02}{minutes:02}{seconds:02}")
}

pub fn now_iso8601() -> String {
    let (year, month, day, hours, minutes, seconds) = wall_clock_parts();
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0u64;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u64 + 1;
            break;
        }
        days -= md;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::derive_key;

    struct BranchingIds {
        branch_parent: NodeId,
        branch_leaf: NodeId,
    }

    fn build_branching_session() -> (Session, BranchingIds) {
        let mut session = Session::default();
        let root = session
            .tree
            .push(None, Message::new(Role::User, "root".to_owned()));
        let intro = session.tree.push(
            Some(root),
            Message::new(Role::Assistant, "intro".to_owned()),
        );
        let branch_parent = session.tree.push(
            Some(intro),
            Message::new(Role::User, "branch here".to_owned()),
        );

        session.tree.push(
            Some(branch_parent),
            Message::new(Role::Assistant, "left branch".to_owned()),
        );
        session.tree.set_head(Some(branch_parent));

        let right_branch = session.tree.push(
            Some(branch_parent),
            Message::new(Role::Assistant, "right branch".to_owned()),
        );
        let right_user = session.tree.push(
            Some(right_branch),
            Message::new(Role::User, "right follow-up".to_owned()),
        );
        let right_leaf = session.tree.push(
            Some(right_user),
            Message::new(Role::Assistant, "right leaf".to_owned()),
        );

        (
            session,
            BranchingIds {
                branch_parent,
                branch_leaf: right_leaf,
            },
        )
    }

    #[test]
    fn persists_preferred_branch_choices_via_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let mut db = crate::db::Database::open(&db_path, None).unwrap();

        let (session, ids) = build_branching_session();
        db.insert_session("branch-test", &session).unwrap();
        let mut loaded = db.load_session("branch-test").unwrap();

        loaded.tree.switch_to(ids.branch_parent);
        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }

    #[test]
    fn duplicate_subtree_preserves_nested_branch_selection() {
        let (mut session, ids) = build_branching_session();

        let new_root = session
            .tree
            .duplicate_subtree(ids.branch_parent)
            .expect("subtree should duplicate");
        session.tree.switch_to(new_root);

        let head = session.tree.head().expect("head should exist");
        let head_message = &session
            .tree
            .node(head)
            .expect("head node should exist")
            .message
            .content;

        assert_eq!(head_message, "right leaf");
    }

    #[test]
    fn clear_drops_runtime_caches_and_preferred_branches() {
        let (mut session, _) = build_branching_session();

        session.tree.clear();

        assert!(session.tree.current_branch_ids().is_empty());
        assert!(session.tree.current_user_branch_ids().is_empty());
        assert!(session.tree.preferred_child.is_empty());
        assert_eq!(session.tree.current_deepest_branch_info(), None);
        assert_eq!(session.tree.current_last_assistant_preview(), None);
    }

    #[test]
    fn encrypted_db_load_rehydrates_preferred_branch_choices() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("encrypted.db");
        let salt = [7u8; 16];
        let key = derive_key("passkey", &salt).expect("key derivation should succeed");

        let (session, ids) = build_branching_session();
        let mut db = crate::db::Database::open(&db_path, Some(&key)).unwrap();
        db.insert_session("enc-branch-test", &session).unwrap();
        drop(db);

        let db = crate::db::Database::open(&db_path, Some(&key)).unwrap();
        let mut loaded = db.load_session("enc-branch-test").unwrap();

        loaded.tree.switch_to(ids.branch_parent);
        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }

    #[test]
    fn splice_between_preserves_head_and_keeps_continuation_in_branch() {
        let mut tree = MessageTree::new();
        let m1 = tree.push(None, Message::new(Role::User, "m1".to_owned()));
        let m2 = tree.push(Some(m1), Message::new(Role::Assistant, "m2".to_owned()));
        let m3 = tree.push(Some(m2), Message::new(Role::User, "m3".to_owned()));
        let m4 = tree.push(Some(m3), Message::new(Role::Assistant, "m4".to_owned()));
        let m5 = tree.push(Some(m4), Message::new(Role::User, "m5".to_owned()));
        let m6 = tree.push(Some(m5), Message::new(Role::Assistant, "m6".to_owned()));

        let summary_id = tree.splice_between(
            m3,
            Message::new(Role::Summary, "prefix summary".to_owned()),
        );

        assert_eq!(tree.head(), Some(m6), "head must remain the original leaf");
        assert_eq!(
            tree.current_branch_ids(),
            &[m1, m2, m3, summary_id, m4, m5, m6],
            "summary must appear in-path between parent and former children"
        );
        assert_eq!(tree.node(m3).unwrap().children, vec![summary_id]);
        assert_eq!(tree.node(summary_id).unwrap().children, vec![m4]);
        assert_eq!(tree.node(m4).unwrap().parent, Some(summary_id));
    }

    #[test]
    fn splice_between_produces_no_sibling_pagination_on_new_node() {
        let mut tree = MessageTree::new();
        let m1 = tree.push(None, Message::new(Role::User, "m1".to_owned()));
        let m2 = tree.push(Some(m1), Message::new(Role::Assistant, "m2".to_owned()));
        let _m3 = tree.push(Some(m2), Message::new(Role::User, "m3".to_owned()));

        let summary_id = tree.splice_between(
            m1,
            Message::new(Role::Summary, "sole summary".to_owned()),
        );

        let (_, total) = tree.sibling_info(summary_id);
        assert_eq!(
            total, 1,
            "spliced summary must be the sole child of its parent"
        );
    }

    #[test]
    fn splice_between_reparents_all_existing_children_including_alt_branches() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".to_owned()));
        let alt_a = tree.push(
            Some(root),
            Message::new(Role::Assistant, "alt a".to_owned()),
        );
        let alt_b = tree.push(
            Some(root),
            Message::new(Role::Assistant, "alt b".to_owned()),
        );
        tree.set_head(Some(alt_a));

        let summary_id =
            tree.splice_between(root, Message::new(Role::Summary, "sum".to_owned()));

        assert_eq!(tree.node(root).unwrap().children, vec![summary_id]);
        assert_eq!(
            tree.node(summary_id).unwrap().children,
            vec![alt_a, alt_b],
            "both alternates must reparent to the new node"
        );
        assert_eq!(tree.node(alt_a).unwrap().parent, Some(summary_id));
        assert_eq!(tree.node(alt_b).unwrap().parent, Some(summary_id));
        assert_eq!(tree.head(), Some(alt_a));
    }

    #[test]
    fn remove_node_on_leaf_moves_head_to_parent() {
        let mut tree = MessageTree::new();
        let m1 = tree.push(None, Message::new(Role::User, "m1".to_owned()));
        let m2 = tree.push(Some(m1), Message::new(Role::Assistant, "m2".to_owned()));

        let removed = tree.remove_node(m2);
        assert!(removed);
        assert_eq!(tree.head(), Some(0), "head must move to remapped parent");
        assert_eq!(tree.nodes().len(), 1);
        assert_eq!(tree.node(0).unwrap().message.content, "m1");
        assert!(tree.node(0).unwrap().children.is_empty());
    }

    #[test]
    fn remove_node_in_middle_reparents_children_to_grandparent() {
        let mut tree = MessageTree::new();
        let m1 = tree.push(None, Message::new(Role::User, "m1".to_owned()));
        let m2 = tree.push(Some(m1), Message::new(Role::Assistant, "m2".to_owned()));
        let m3 = tree.push(Some(m2), Message::new(Role::User, "m3".to_owned()));
        let m4 = tree.push(Some(m3), Message::new(Role::Assistant, "m4".to_owned()));

        let removed = tree.remove_node(m2);
        assert!(removed);
        assert_eq!(tree.nodes().len(), 3);
        assert_eq!(tree.head(), Some(2));

        let branch = tree.current_branch_ids();
        assert_eq!(branch.len(), 3);
        let contents: Vec<&str> = branch
            .iter()
            .map(|&id| tree.node(id).unwrap().message.content.as_str())
            .collect();
        assert_eq!(contents, vec!["m1", "m3", "m4"]);

        let _ = (m2, m3, m4);
    }

    #[test]
    fn remove_node_on_root_with_single_child_promotes_child() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".to_owned()));
        let child = tree.push(Some(root), Message::new(Role::Assistant, "child".to_owned()));

        let removed = tree.remove_node(root);
        assert!(removed);
        assert_eq!(tree.nodes().len(), 1);
        assert_eq!(tree.node(0).unwrap().message.content, "child");
        assert_eq!(tree.node(0).unwrap().parent, None);
        assert_eq!(tree.head(), Some(0));

        let _ = child;
    }

    #[test]
    fn remove_node_on_root_with_multiple_children_leaves_multiple_roots() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".to_owned()));
        let alt_a = tree.push(Some(root), Message::new(Role::Assistant, "alt_a".to_owned()));
        let _alt_b = tree.push(Some(root), Message::new(Role::Assistant, "alt_b".to_owned()));
        tree.set_head(Some(alt_a));

        let removed = tree.remove_node(root);
        assert!(removed);
        assert_eq!(tree.nodes().len(), 2);
        for node in tree.nodes() {
            assert_eq!(node.parent, None, "both survivors must be roots");
        }
        assert!(tree.head().is_some());
        let head_content = tree
            .node(tree.head().unwrap())
            .unwrap()
            .message
            .content
            .clone();
        assert_eq!(head_content, "alt_a");
    }

    #[test]
    fn remove_node_drops_preferred_child_entries_pointing_at_removed_id() {
        let mut tree = MessageTree::new();
        let m1 = tree.push(None, Message::new(Role::User, "m1".to_owned()));
        let m2 = tree.push(Some(m1), Message::new(Role::Assistant, "m2".to_owned()));
        let _m3 = tree.push(Some(m1), Message::new(Role::Assistant, "m3".to_owned()));
        tree.set_head(Some(m2));

        let removed = tree.remove_node(m2);
        assert!(removed);
        for (&parent_id, &child_id) in tree.preferred_child_map() {
            assert!(
                tree.node(parent_id).is_some(),
                "preferred_child parent must be a live node"
            );
            assert!(
                tree.node(child_id).is_some(),
                "preferred_child must point at a live node"
            );
        }
    }

    #[test]
    fn remove_node_returns_false_for_unknown_id() {
        let mut tree = MessageTree::new();
        tree.push(None, Message::new(Role::User, "m1".to_owned()));
        assert!(!tree.remove_node(42));
    }

    #[test]
    fn save_mode_id_returns_none_for_none() {
        assert_eq!(SaveMode::None.id(), None);
    }

    #[test]
    fn save_mode_id_returns_some_for_database() {
        assert_eq!(SaveMode::Database { id: "abc".into() }.id(), Some("abc"));
    }

    #[test]
    fn save_mode_set_id_updates_database() {
        let mut mode = SaveMode::Database { id: "old".into() };
        mode.set_id("new".into());
        assert_eq!(mode.id(), Some("new"));
    }

    #[test]
    fn save_mode_set_id_noop_for_none() {
        let mut mode = SaveMode::None;
        mode.set_id("ignored".into());
        assert_eq!(mode.id(), None);
    }

    #[test]
    fn save_mode_needs_passkey() {
        assert!(SaveMode::PendingPasskey { id: "x".into() }.needs_passkey());
        assert!(!SaveMode::None.needs_passkey());
        assert!(!SaveMode::Database { id: "x".into() }.needs_passkey());
    }

    #[test]
    fn current_branch_ids_linear_path() {
        let mut tree = MessageTree::new();
        let a = tree.push(None, Message::new(Role::User, "a".into()));
        let b = tree.push(Some(a), Message::new(Role::Assistant, "b".into()));
        let c = tree.push(Some(b), Message::new(Role::User, "c".into()));

        assert_eq!(tree.current_branch_ids(), &[a, b, c]);
    }

    #[test]
    fn current_branch_ids_after_branch() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".into()));
        let left = tree.push(Some(root), Message::new(Role::Assistant, "left".into()));
        tree.set_head(Some(root));
        let right = tree.push(Some(root), Message::new(Role::Assistant, "right".into()));

        assert_eq!(tree.current_branch_ids(), &[root, right]);
        let _ = left;
    }

    #[test]
    fn current_user_branch_ids_filters_roles() {
        let mut tree = MessageTree::new();
        let u1 = tree.push(None, Message::new(Role::User, "u1".into()));
        let a1 = tree.push(Some(u1), Message::new(Role::Assistant, "a1".into()));
        let u2 = tree.push(Some(a1), Message::new(Role::User, "u2".into()));

        assert_eq!(tree.current_user_branch_ids(), &[u1, u2]);
    }

    #[test]
    fn current_deepest_branch_info_no_branching() {
        let mut tree = MessageTree::new();
        let a = tree.push(None, Message::new(Role::User, "a".into()));
        tree.push(Some(a), Message::new(Role::Assistant, "b".into()));

        assert_eq!(tree.current_deepest_branch_info(), None);
    }

    #[test]
    fn current_deepest_branch_info_with_branches() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".into()));
        tree.push(
            Some(root),
            Message::new(Role::Assistant, "sibling a".into()),
        );
        tree.set_head(Some(root));
        tree.push(
            Some(root),
            Message::new(Role::Assistant, "sibling b".into()),
        );

        let info = tree.current_deepest_branch_info();
        assert!(info.is_some());
        let (_, total) = info.unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn current_last_assistant_preview_present() {
        let mut tree = MessageTree::new();
        let u = tree.push(None, Message::new(Role::User, "hello".into()));
        tree.push(Some(u), Message::new(Role::Assistant, "world".into()));

        assert_eq!(tree.current_last_assistant_preview(), Some("world"));
    }

    #[test]
    fn current_last_assistant_preview_absent() {
        let mut tree = MessageTree::new();
        tree.push(None, Message::new(Role::User, "hello".into()));

        assert_eq!(tree.current_last_assistant_preview(), None);
    }

    #[test]
    fn switch_sibling_wraps_around() {
        let mut tree = MessageTree::new();
        let root = tree.push(None, Message::new(Role::User, "root".into()));
        let first = tree.push(Some(root), Message::new(Role::Assistant, "first".into()));
        tree.set_head(Some(root));
        let second = tree.push(Some(root), Message::new(Role::Assistant, "second".into()));

        assert_eq!(tree.head(), Some(second));
        tree.switch_sibling(1);
        assert_eq!(tree.head(), Some(first));
        tree.switch_sibling(-1);
        assert_eq!(tree.head(), Some(second));
    }

    #[test]
    fn from_messages_builds_linear_tree() {
        let messages = vec![
            Message::new(Role::User, "first".into()),
            Message::new(Role::Assistant, "second".into()),
            Message::new(Role::User, "third".into()),
        ];
        let tree = MessageTree::from_messages(messages);

        let ids = tree.current_branch_ids();
        assert_eq!(ids.len(), 3);
        assert_eq!(tree.node(ids[0]).unwrap().message.content, "first");
        assert_eq!(tree.node(ids[1]).unwrap().message.content, "second");
        assert_eq!(tree.node(ids[2]).unwrap().message.content, "third");
        assert_eq!(tree.head(), Some(ids[2]));
    }
}
