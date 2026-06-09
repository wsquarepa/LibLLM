//! Conversation session types with a branching message tree backed by an arena allocator.

mod message;
mod time;
mod tree;

pub use message::{Message, ParseRoleError, Role};
pub use time::{now_compact, now_iso8601};
pub use tree::{MessageTree, Node, NodeId};

use serde::{Deserialize, Serialize};

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
    pub updated_at: Option<String>,
    pub sidebar_label: String,
    pub sidebar_preview: Option<String>,
    pub is_new_chat: bool,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scenario: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub characters: Vec<crate::group_chat::CharacterAttachment>,
    #[serde(default)]
    pub chat_mode: crate::group_chat::ChatMode,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author_note: Option<crate::author_note::AuthorNote>,
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
}

pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn session_default_has_empty_characters_and_action_value_mode() {
        let s = Session::default();
        assert!(s.characters.is_empty());
        assert!(matches!(
            s.chat_mode,
            crate::group_chat::ChatMode::ActionValue
        ));
        assert!(s.scenario.is_none());
    }

    #[test]
    fn session_serde_round_trip_with_characters() {
        let s = Session {
            characters: vec![
                crate::group_chat::CharacterAttachment {
                    slug: "alice".to_owned(),
                    talkativeness: 0.7,
                    action_points: 0.3,
                    spoke_this_round: false,
                },
                crate::group_chat::CharacterAttachment {
                    slug: "bob".to_owned(),
                    talkativeness: 0.4,
                    action_points: 0.0,
                    spoke_this_round: false,
                },
            ],
            chat_mode: crate::group_chat::ChatMode::WeightedRandom,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.characters.len(), 2);
        assert_eq!(back.characters[0].slug, "alice");
        assert!((back.characters[0].talkativeness - 0.7).abs() < f32::EPSILON);
        assert!((back.characters[0].action_points - 0.3).abs() < f32::EPSILON);
        assert!(matches!(
            back.chat_mode,
            crate::group_chat::ChatMode::WeightedRandom
        ));
    }

    #[test]
    fn session_deserializes_legacy_json_without_new_fields() {
        let json = r#"{"tree":{"nodes":[],"head":null,"preferred_child":{}}}"#;
        let s: Session = serde_json::from_str(json).unwrap();
        assert!(s.characters.is_empty());
        assert!(matches!(
            s.chat_mode,
            crate::group_chat::ChatMode::ActionValue
        ));
        assert!(s.scenario.is_none());
    }
}
