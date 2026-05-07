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

use std::collections::HashMap;

use rand::{Rng, RngExt};

#[derive(Debug)]
pub struct TurnDecision {
    pub speaker_slug: String,
    pub updated_action_points: Vec<(String, f32)>,
    pub snapshot_before: HashMap<String, f32>,
}

pub fn decide_next_speaker(
    characters: &[CharacterAttachment],
    policy: ChatPolicy,
    rng: &mut impl Rng,
) -> Option<TurnDecision> {
    if characters.is_empty() {
        return None;
    }

    let snapshot_before: HashMap<String, f32> = characters
        .iter()
        .map(|a| (a.slug.clone(), a.action_points))
        .collect();

    let mut updated: Vec<(String, f32)> = characters
        .iter()
        .map(|a| (a.slug.clone(), a.action_points + a.talkativeness))
        .collect();

    let candidates: Vec<usize> = updated
        .iter()
        .enumerate()
        .filter(|(_, (_, ap))| *ap >= ACTION_POINT_THRESHOLD)
        .map(|(i, _)| i)
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let chosen_idx = match policy {
        ChatPolicy::RoundRobin => candidates[0],
        ChatPolicy::WeightedRandom => {
            if candidates.len() == 1 {
                candidates[0]
            } else {
                weighted_pick(&candidates, characters, rng)
            }
        }
    };

    let chosen_slug = updated[chosen_idx].0.clone();
    updated[chosen_idx].1 -= ACTION_POINT_COST;

    Some(TurnDecision {
        speaker_slug: chosen_slug,
        updated_action_points: updated,
        snapshot_before,
    })
}

fn weighted_pick(candidates: &[usize], characters: &[CharacterAttachment], rng: &mut impl Rng) -> usize {
    let weights: Vec<f32> = candidates
        .iter()
        .map(|&i| characters[i].talkativeness.max(0.0))
        .collect();
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return candidates[0];
    }
    let mut roll = rng.random::<f32>() * total;
    for (k, w) in weights.iter().enumerate() {
        if roll < *w {
            return candidates[k];
        }
        roll -= w;
    }
    *candidates.last().expect("non-empty by guard")
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn att(slug: &str, talk: f32, ap: f32) -> CharacterAttachment {
        CharacterAttachment { slug: slug.to_owned(), talkativeness: talk, action_points: ap }
    }

    #[test]
    fn decide_next_returns_none_when_no_one_over_threshold() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.4, 0.0), att("b", 0.4, 0.5)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng);
        assert!(d.is_none());
    }

    #[test]
    fn decide_next_picks_only_eligible_speaker() {
        let mut rng = StdRng::seed_from_u64(0);
        // a: 0.4 + 0.4 = 0.8 (under threshold)
        // b: 0.5 + 0.6 = 1.1 (over threshold)
        let cs = vec![att("a", 0.4, 0.4), att("b", 0.5, 0.6)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert_eq!(d.speaker_slug, "b");
        let new_b = d.updated_action_points.iter().find(|(s, _)| s == "b").unwrap().1;
        assert!((new_b - 0.1).abs() < 1e-5);
        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        assert!((new_a - 0.8).abs() < 1e-5);
    }

    #[test]
    fn decide_next_round_robin_breaks_tie_by_attach_index() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.6, 0.5), att("b", 0.6, 0.5), att("c", 0.6, 0.5)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert_eq!(d.speaker_slug, "a");
        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        let new_b = d.updated_action_points.iter().find(|(s, _)| s == "b").unwrap().1;
        let new_c = d.updated_action_points.iter().find(|(s, _)| s == "c").unwrap().1;
        assert!((new_a - 0.1).abs() < 1e-5);
        assert!((new_b - 1.1).abs() < 1e-5);
        assert!((new_c - 1.1).abs() < 1e-5);
    }

    #[test]
    fn decide_next_weighted_random_uses_seeded_rng() {
        let cs = vec![att("a", 0.6, 0.5), att("b", 0.9, 0.5)];
        let mut rng = StdRng::seed_from_u64(42);
        let d = decide_next_speaker(&cs, ChatPolicy::WeightedRandom, &mut rng).unwrap();
        assert!(d.speaker_slug == "a" || d.speaker_slug == "b");

        let mut counts = (0u32, 0u32);
        let cs = vec![att("a", 0.6, 0.5), att("b", 0.9, 0.5)];
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..2000 {
            let d = decide_next_speaker(&cs, ChatPolicy::WeightedRandom, &mut rng).unwrap();
            if d.speaker_slug == "a" { counts.0 += 1; } else { counts.1 += 1; }
        }
        assert!(counts.1 > counts.0, "expected b to win more often (talk 0.9 vs 0.6): a={}, b={}", counts.0, counts.1);
    }

    #[test]
    fn decide_next_zero_talkativeness_never_accumulates() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.0, 0.99), att("b", 0.6, 0.5)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert_eq!(d.speaker_slug, "b");
        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        assert!((new_a - 0.99).abs() < 1e-5, "talk=0 must not accumulate");
    }

    #[test]
    fn decide_next_snapshot_before_captures_pre_increment_state() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 0.5), att("b", 0.5, 0.7)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert!((d.snapshot_before["a"] - 0.5).abs() < 1e-5);
        assert!((d.snapshot_before["b"] - 0.7).abs() < 1e-5);
    }

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
