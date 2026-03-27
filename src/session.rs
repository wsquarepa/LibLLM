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
}

pub struct SessionEntry {
    pub path: PathBuf,
    pub preview: String,
    pub filename: String,
    pub is_new_chat: bool,
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
}

impl MessageTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            head: None,
        }
    }

    pub fn head(&self) -> Option<NodeId> {
        self.head
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn push(&mut self, parent: Option<NodeId>, message: Message) -> NodeId {
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
        self.head = Some(id);
        id
    }

    pub fn branch_path(&self) -> Vec<&Message> {
        let path = self.branch_path_ids();
        path.iter().map(|&id| &self.nodes[id].message).collect()
    }

    pub fn branch_path_ids(&self) -> Vec<NodeId> {
        let Some(head) = self.head else {
            return Vec::new();
        };
        let mut path = Vec::new();
        let mut current = head;
        loop {
            path.push(current);
            match self.nodes[current].parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        path.reverse();
        path
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
                let roots: Vec<NodeId> = self.nodes.iter()
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
            current = self.nodes[current].children[0];
        }
        self.head = Some(current);
    }

    pub fn switch_sibling(&mut self, offset: isize) {
        if self.head.is_none() { return };

        let path = self.branch_path_ids();
        let branch_node = path.iter().rev()
            .find(|&&id| {
                let (_, total) = self.sibling_info(id);
                total > 1
            })
            .copied();

        let Some(node_id) = branch_node else { return };

        let parent = self.nodes[node_id].parent;
        let siblings = match parent {
            Some(pid) => self.nodes[pid].children.clone(),
            None => self.nodes.iter()
                .filter(|n| n.parent.is_none())
                .map(|n| n.id)
                .collect(),
        };

        let current_idx = siblings.iter().position(|&c| c == node_id).unwrap_or(0);
        let new_idx = (current_idx as isize + offset).rem_euclid(siblings.len() as isize) as usize;
        let new_node = siblings[new_idx];

        self.switch_to(new_node);
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
        }

        self.head = parent;
        Some(message)
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.head = None;
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

    pub fn deepest_branch_info(&self) -> Option<(usize, usize)> {
        self.head?;
        let path = self.branch_path_ids();
        path.iter().rev()
            .find(|&&id| {
                let (_, total) = self.sibling_info(id);
                total > 1
            })
            .map(|&id| self.sibling_info(id))
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
    pub fn pop_trailing_assistant(&mut self) {
        while self.tree.head()
            .is_some_and(|id| self.tree.node(id).is_some_and(|n| n.message.role == Role::Assistant))
        {
            self.tree.pop_head();
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
        Err(e) => return Err(e).context(format!("failed to read session file: {}", path.display())),
    };

    load_from_str(&contents)
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;
    std::fs::write(path, json).context(format!("failed to write session file: {}", path.display()))
}

pub fn save_encrypted(path: &Path, session: &Session, key: &DerivedKey) -> Result<()> {
    let json = serde_json::to_string(session).context("failed to serialize session")?;
    let blob = crate::crypto::encrypt(json.as_bytes(), key)?;
    std::fs::write(path, blob).context(format!("failed to write encrypted session: {}", path.display()))
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
        serde_json::from_str::<Session>(&json).context("failed to parse decrypted session")
    } else {
        let contents = String::from_utf8_lossy(&data);
        load_from_str(&contents)
    }
}

pub fn generate_session_name() -> String {
    let ts = now_iso8601();
    let name = ts.replace(':', "-").replace('T', "_").trim_end_matches('Z').to_owned();
    format!("{name}.session")
}

pub fn generate_session_name_for_character(character: &str) -> String {
    let ts = now_iso8601();
    let time_part = ts.replace(':', "-").replace('T', "_").trim_end_matches('Z').to_owned();
    let slug: String = character.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-");
    format!("{slug}_{time_part}.session")
}

pub fn list_session_paths(dir: &Path) -> Vec<SessionEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut sessions: Vec<SessionEntry> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "session"))
        .map(|path| {
            let filename = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            SessionEntry { path, preview: String::new(), filename, is_new_chat: false }
        })
        .collect();

    sessions.sort_by(|a, b| b.filename.cmp(&a.filename));
    sessions
}

pub fn load_preview(path: &Path, key: &DerivedKey) -> String {
    extract_preview(path, key)
}

fn extract_preview(path: &Path, key: &DerivedKey) -> String {
    let session = match load_encrypted(path, key) {
        Ok(s) => s,
        Err(_) => return "[encrypted]".to_owned(),
    };

    session
        .tree
        .branch_path()
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| {
            let truncated: String = m.content.chars().take(40).collect();
            if m.content.chars().count() > 40 {
                format!("{truncated}...")
            } else {
                truncated
            }
        })
        .unwrap_or_else(|| "[empty]".to_owned())
}

fn load_from_str(contents: &str) -> Result<Session> {
    if let Ok(session) = serde_json::from_str::<Session>(contents) {
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
