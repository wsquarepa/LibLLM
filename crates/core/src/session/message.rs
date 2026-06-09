//! Message types: [`Role`], [`Message`], and their parsing / display impls.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::time::now_iso8601;

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

/// Error returned when parsing a [`Role`] from an unrecognized string.
#[derive(Debug, thiserror::Error)]
#[error("unknown role: {0}")]
pub struct ParseRoleError(pub String);

impl std::str::FromStr for Role {
    type Err = ParseRoleError;

    fn from_str(s: &str) -> Result<Self, ParseRoleError> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "summary" => Ok(Self::Summary),
            _ => Err(ParseRoleError(s.to_owned())),
        }
    }
}

/// A single chat message with role, content text, and ISO-8601 timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_turn_action_points: Option<String>,
}

impl Message {
    pub fn new(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            timestamp: now_iso8601(),
            thought_seconds: None,
            speaker: None,
            pre_turn_action_points: None,
        }
    }

    pub fn with_thought_seconds(mut self, thought_seconds: Option<u32>) -> Self {
        self.thought_seconds = thought_seconds;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_new_has_no_speaker_or_action_points() {
        let m = Message::new(Role::Assistant, "hi".to_owned());
        assert!(m.speaker.is_none());
        assert!(m.pre_turn_action_points.is_none());
    }

    #[test]
    fn message_serde_round_trip_with_speaker() {
        let mut m = Message::new(Role::Assistant, "hi".to_owned());
        m.speaker = Some("alice".to_owned());
        m.pre_turn_action_points = Some(r#"{"alice":0.2,"bob":0.5}"#.to_owned());
        let json = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.speaker.as_deref(), Some("alice"));
        assert_eq!(
            back.pre_turn_action_points.as_deref(),
            Some(r#"{"alice":0.2,"bob":0.5}"#)
        );
    }

    #[test]
    fn message_serde_round_trip_without_optional_fields() {
        let json = r#"{"role":"user","content":"hello","timestamp":"2026-05-07T12:00:00Z"}"#;
        let m: Message = serde_json::from_str(json).unwrap();
        assert!(m.speaker.is_none());
        assert!(m.pre_turn_action_points.is_none());
    }
}
