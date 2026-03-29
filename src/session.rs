use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

#[derive(Clone)]
pub enum SaveMode {
    None,
    Plaintext(PathBuf),
    Encrypted { path: PathBuf, key: Arc<DerivedKey> },
    PendingPasskey(PathBuf),
}

impl SaveMode {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::None => None,
            Self::Plaintext(p) => Some(p),
            Self::Encrypted { path, .. } => Some(path),
            Self::PendingPasskey(p) => Some(p),
        }
    }

    pub fn set_path(&mut self, new_path: PathBuf) {
        match self {
            Self::None => {}
            Self::Plaintext(p) => *p = new_path,
            Self::Encrypted { path, .. } => *path = new_path,
            Self::PendingPasskey(p) => *p = new_path,
        }
    }

    pub fn needs_passkey(&self) -> bool {
        matches!(self, Self::PendingPasskey(_))
    }

    pub fn key(&self) -> Option<&DerivedKey> {
        match self {
            Self::Encrypted { key, .. } => Some(key),
            _ => None,
        }
    }
}

pub struct SessionEntry {
    pub path: PathBuf,
    pub filename: String,
    pub display_name: String,
    pub message_count: Option<usize>,
    pub first_message: Option<String>,
    pub sidebar_label: String,
    pub sidebar_preview: Option<String>,
    pub is_new_chat: bool,
}

pub struct SessionMetadata {
    pub character: Option<String>,
    pub message_count: usize,
    pub first_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Assistant => f.write_str("assistant"),
            Self::System => f.write_str("system"),
        }
    }
}

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

pub type NodeId = usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTree {
    nodes: Vec<Node>,
    head: Option<NodeId>,
    #[serde(default)]
    preferred_child: HashMap<NodeId, NodeId>,
    #[serde(skip)]
    current_branch_ids: Vec<NodeId>,
    #[serde(skip)]
    current_user_branch_ids: Vec<NodeId>,
    #[serde(skip)]
    current_deepest_branch_info: Option<(usize, usize)>,
    #[serde(skip)]
    current_first_user_preview: Option<String>,
}

impl MessageTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            head: None,
            preferred_child: HashMap::new(),
            current_branch_ids: Vec::new(),
            current_user_branch_ids: Vec::new(),
            current_deepest_branch_info: None,
            current_first_user_preview: None,
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
        self.current_branch_ids.clear();
        self.current_user_branch_ids.clear();
        self.current_deepest_branch_info = None;
        self.current_first_user_preview = None;

        let Some(head) = self.head else {
            return;
        };

        let mut current = head;
        loop {
            self.current_branch_ids.push(current);
            match self.nodes[current].parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.current_branch_ids.reverse();

        for &id in &self.current_branch_ids {
            let node = &self.nodes[id];
            if node.message.role == Role::User {
                self.current_user_branch_ids.push(id);
                if self.current_first_user_preview.is_none() {
                    self.current_first_user_preview = Some(node.message.content.clone());
                }
            }
        }

        self.current_deepest_branch_info = self.current_branch_ids.iter().rev().find_map(|&id| {
            let info = self.sibling_info(id);
            (info.1 > 1).then_some(info)
        });
    }

    fn rehydrate_runtime_state(&mut self) {
        self.validate_preferred_children();
        if self.preferred_child.is_empty() {
            self.update_preferred_children();
        }
        self.refresh_runtime_caches();
    }

    pub fn head(&self) -> Option<NodeId> {
        self.head
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

    pub fn branch_path(&self) -> Vec<&Message> {
        self.messages_for_ids(self.current_branch_ids())
    }

    pub fn branch_path_ids(&self) -> Vec<NodeId> {
        self.current_branch_ids.clone()
    }

    pub fn current_branch_ids(&self) -> &[NodeId] {
        &self.current_branch_ids
    }

    pub fn current_user_branch_ids(&self) -> &[NodeId] {
        &self.current_user_branch_ids
    }

    pub fn current_deepest_branch_info(&self) -> Option<(usize, usize)> {
        self.current_deepest_branch_info
    }

    pub fn current_first_user_preview(&self) -> Option<&str> {
        self.current_first_user_preview.as_deref()
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
        self.head = Some(current);
        self.update_preferred_children();
        self.refresh_runtime_caches();
    }

    pub fn switch_sibling(&mut self, offset: isize) {
        if self.head.is_none() {
            return;
        };

        let path = self.branch_path_ids();
        let branch_node = path
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
        self.head = self.nodes[head].parent;
        self.update_preferred_children();
        self.refresh_runtime_caches();
        Some(&self.nodes[head].message)
    }

    pub fn set_head(&mut self, id: Option<NodeId>) {
        self.head = id;
        self.update_preferred_children();
        self.refresh_runtime_caches();
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

        self.head = parent;
        self.update_preferred_children();
        self.refresh_runtime_caches();
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

#[derive(Debug, Serialize, Deserialize)]
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
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tree: MessageTree::new(),
            model: None,
            template: None,
            system_prompt: None,
            character: None,
            worldbooks: Vec::new(),
        }
    }
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

    pub fn maybe_save(&self, mode: &SaveMode) -> Result<()> {
        match mode {
            SaveMode::None | SaveMode::PendingPasskey(_) => Ok(()),
            SaveMode::Plaintext(path) => save(path, self),
            SaveMode::Encrypted { path, key } => save_encrypted(path, self, key),
        }
    }
}

#[derive(Deserialize)]
struct FlatSession {
    messages: Vec<Message>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    worldbooks: Vec<String>,
}

#[derive(Deserialize)]
struct LegacySession {
    prompt_history: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    template: Option<String>,
}

pub fn load(path: &Path) -> Result<Session> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
        Err(e) => {
            return Err(e).context(format!("failed to read session file: {}", path.display()));
        }
    };

    load_from_str(&contents)
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;
    crate::crypto::write_atomic(path, json.as_bytes())
        .context(format!("failed to write session file: {}", path.display()))
}

pub fn save_encrypted(path: &Path, session: &Session, key: &DerivedKey) -> Result<()> {
    let json = serde_json::to_string(session).context("failed to serialize session")?;
    let blob = crate::crypto::encrypt(json.as_bytes(), key)?;
    crate::crypto::write_atomic(path, &blob).context(format!(
        "failed to write encrypted session: {}",
        path.display()
    ))
}

pub fn load_encrypted(path: &Path, key: &DerivedKey) -> Result<Session> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
        Err(e) => return Err(e).context(format!("failed to read session: {}", path.display())),
    };

    if crate::crypto::is_encrypted(&data) {
        let plaintext = crate::crypto::decrypt(&data, key)?;
        let json = String::from_utf8(plaintext).context("decrypted session is not valid UTF-8")?;
        load_from_str(&json)
    } else {
        let contents = String::from_utf8_lossy(&data);
        load_from_str(&contents)
    }
}

pub fn generate_session_name() -> String {
    let id = uuid::Uuid::new_v4();
    format!("{id}.session")
}

pub fn list_session_paths(dir: &Path) -> Result<Vec<SessionEntry>> {
    let entries = std::fs::read_dir(dir).context(format!(
        "failed to read sessions directory: {}",
        dir.display()
    ))?;

    let mut sessions: Vec<SessionEntry> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "session"))
        .map(|path| {
            let filename = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            SessionEntry {
                path,
                filename,
                display_name: "Assistant".to_owned(),
                message_count: None,
                first_message: None,
                sidebar_label: String::new(),
                sidebar_preview: None,
                is_new_chat: false,
            }
        })
        .collect();

    sessions.sort_by(|a, b| {
        let mtime = |p: &Path| {
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        };
        mtime(&b.path).cmp(&mtime(&a.path))
    });
    Ok(sessions)
}

pub fn load_metadata(path: &Path, key: &DerivedKey) -> Option<SessionMetadata> {
    let session = load_encrypted(path, key).ok()?;
    let first_message = session.tree.current_first_user_preview().map(str::to_owned);
    Some(SessionMetadata {
        character: session.character,
        message_count: session.tree.node_count(),
        first_message,
    })
}

fn load_from_str(contents: &str) -> Result<Session> {
    if let Ok(mut session) = serde_json::from_str::<Session>(contents) {
        session.tree.rehydrate_runtime_state();
        return Ok(session);
    }

    if let Ok(flat) = serde_json::from_str::<FlatSession>(contents) {
        return Ok(Session {
            tree: MessageTree::from_messages(flat.messages),
            model: flat.model,
            template: flat.template,
            system_prompt: flat.system_prompt,
            character: flat.character,
            worldbooks: flat.worldbooks,
        });
    }

    if let Ok(legacy) = serde_json::from_str::<LegacySession>(contents) {
        return Ok(Session {
            tree: MessageTree::from_messages(vec![Message::new(Role::User, legacy.prompt_history)]),
            model: legacy.model,
            template: legacy.template,
            system_prompt: None,
            character: None,
            worldbooks: Vec::new(),
        });
    }

    Ok(Session {
        tree: MessageTree::from_messages(vec![Message::new(Role::User, contents.to_owned())]),
        ..Session::default()
    })
}

fn now_iso8601() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_ymd(secs / 86400);
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
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::derive_key;
    use serde_json::json;

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
    fn persists_preferred_branch_choices_across_reload() {
        let (session, ids) = build_branching_session();

        let json = serde_json::to_string(&session).expect("session should serialize");
        assert!(json.contains("preferred_child"));

        let mut loaded = load_from_str(&json).expect("session should deserialize");
        loaded.tree.switch_to(ids.branch_parent);

        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }

    #[test]
    fn seeds_preferred_branch_choices_for_old_sessions() {
        let (session, ids) = build_branching_session();

        let mut value = serde_json::to_value(&session).expect("session should serialize");
        value["tree"]
            .as_object_mut()
            .expect("tree should be an object")
            .remove("preferred_child");

        let mut loaded = load_from_str(&value.to_string()).expect("session should deserialize");
        assert!(!loaded.tree.preferred_child.is_empty());

        loaded.tree.switch_to(ids.branch_parent);

        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }

    #[test]
    fn repairs_invalid_preferred_branch_choices_on_load() {
        let (session, ids) = build_branching_session();

        let mut value = serde_json::to_value(&session).expect("session should serialize");
        value["tree"]["preferred_child"] = json!({"999": 1000, "2": 999});

        let mut loaded = load_from_str(&value.to_string()).expect("session should deserialize");
        assert!(loaded.tree.preferred_child.iter().all(|(&parent, &child)| {
            parent < loaded.tree.nodes.len()
                && child < loaded.tree.nodes.len()
                && loaded.tree.nodes[parent].children.contains(&child)
        }));

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
        assert_eq!(session.tree.current_first_user_preview(), None);
    }

    #[test]
    fn encrypted_load_rehydrates_preferred_branch_choices() {
        let (session, ids) = build_branching_session();
        let path = std::env::temp_dir().join(format!("{}.session", uuid::Uuid::new_v4()));
        let salt = [7u8; 16];
        let key = derive_key("passkey", &salt).expect("key derivation should succeed");

        save_encrypted(&path, &session, &key).expect("session should save");
        let mut loaded = load_encrypted(&path, &key).expect("session should load");
        std::fs::remove_file(&path).expect("temp file should be removed");

        loaded.tree.switch_to(ids.branch_parent);

        assert_eq!(loaded.tree.head(), Some(ids.branch_leaf));
    }
}
