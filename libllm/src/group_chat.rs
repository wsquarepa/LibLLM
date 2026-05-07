//! Group-chat runtime: per-session character attachments, action-point turn-order engine,
//! and per-turn prompt assembly. Pure logic, no I/O.

use serde::{Deserialize, Serialize};

pub const MAX_GROUP_SIZE: usize = 8;
pub const DEFAULT_TALKATIVENESS: f32 = 0.5;
pub const ACTION_POINT_THRESHOLD: f32 = 1.0;
pub const ACTION_POINT_COST: f32 = 1.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAttachment {
    pub slug: String,
    pub talkativeness: f32,
    pub action_points: f32,
}

impl CharacterAttachment {
    pub fn new(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            talkativeness: DEFAULT_TALKATIVENESS,
            action_points: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatPolicy {
    #[default]
    RoundRobin,
    WeightedRandom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardAssembly {
    #[default]
    JoinCards,
    SwapCards,
}

impl ChatPolicy {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::WeightedRandom => "weighted_random",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "round_robin" => Some(Self::RoundRobin),
            "weighted_random" => Some(Self::WeightedRandom),
            _ => None,
        }
    }
}

impl CardAssembly {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::JoinCards => "join_cards",
            Self::SwapCards => "swap_cards",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "join_cards" => Some(Self::JoinCards),
            "swap_cards" => Some(Self::SwapCards),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_attachment_default_talkativeness() {
        let a = CharacterAttachment::new("alice");
        assert_eq!(a.slug, "alice");
        assert!((a.talkativeness - DEFAULT_TALKATIVENESS).abs() < f32::EPSILON);
        assert_eq!(a.action_points, 0.0);
    }

    #[test]
    fn chat_policy_serde_round_trip() {
        let s = serde_json::to_string(&ChatPolicy::WeightedRandom).unwrap();
        assert_eq!(s, "\"weighted_random\"");
        let back: ChatPolicy = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ChatPolicy::WeightedRandom));

        let s = serde_json::to_string(&ChatPolicy::RoundRobin).unwrap();
        assert_eq!(s, "\"round_robin\"");
        let back: ChatPolicy = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ChatPolicy::RoundRobin));
    }

    #[test]
    fn card_assembly_serde_round_trip() {
        let s = serde_json::to_string(&CardAssembly::JoinCards).unwrap();
        assert_eq!(s, "\"join_cards\"");
        let back: CardAssembly = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, CardAssembly::JoinCards));

        let s = serde_json::to_string(&CardAssembly::SwapCards).unwrap();
        assert_eq!(s, "\"swap_cards\"");
        let back: CardAssembly = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, CardAssembly::SwapCards));
    }

    #[test]
    fn character_attachment_serde_round_trip() {
        let a = CharacterAttachment { slug: "alice".to_owned(), talkativeness: 0.7, action_points: 0.3 };
        let s = serde_json::to_string(&a).unwrap();
        let back: CharacterAttachment = serde_json::from_str(&s).unwrap();
        assert_eq!(back.slug, "alice");
        assert!((back.talkativeness - 0.7).abs() < f32::EPSILON);
        assert!((back.action_points - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn chat_policy_default_is_round_robin() {
        let p = ChatPolicy::default();
        assert!(matches!(p, ChatPolicy::RoundRobin));
    }

    #[test]
    fn card_assembly_default_is_join() {
        let a = CardAssembly::default();
        assert!(matches!(a, CardAssembly::JoinCards));
    }

    #[test]
    fn chat_policy_db_str_round_trip() {
        for v in [ChatPolicy::RoundRobin, ChatPolicy::WeightedRandom] {
            assert_eq!(ChatPolicy::from_db_str(v.as_db_str()), Some(v));
        }
        assert_eq!(ChatPolicy::from_db_str("bogus"), None);
    }

    #[test]
    fn card_assembly_db_str_round_trip() {
        for v in [CardAssembly::JoinCards, CardAssembly::SwapCards] {
            assert_eq!(CardAssembly::from_db_str(v.as_db_str()), Some(v));
        }
        assert_eq!(CardAssembly::from_db_str("bogus"), None);
    }
}
