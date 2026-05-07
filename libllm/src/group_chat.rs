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

use anyhow::{anyhow, ensure, Result};
use rand::{Rng, RngExt};

use crate::character::CharacterCard;
use crate::persona::PersonaFile;
use crate::preset::ContextPreset;
use crate::session::Session;

#[derive(Debug)]
pub struct TurnPrompt {
    pub system: String,
    pub prefill: String,
    pub stop_sequences: Vec<String>,
}

pub fn build_turn_prompt(
    session: &Session,
    cards: &HashMap<String, CharacterCard>,
    persona: Option<&PersonaFile>,
    template: Option<&ContextPreset>,
    speaker_slug: &str,
) -> Result<TurnPrompt> {
    ensure!(
        session.characters.iter().any(|a| a.slug == speaker_slug),
        "speaker {speaker_slug} is not attached to this session",
    );
    let active_card = cards
        .get(speaker_slug)
        .ok_or_else(|| anyhow!("missing card for speaker {speaker_slug}"))?;

    let live: Vec<(&str, &CharacterCard)> = session
        .characters
        .iter()
        .filter_map(|a| cards.get(&a.slug).map(|c| (a.slug.as_str(), c)))
        .collect();

    let user_name = persona.map(|p| p.name.as_str()).unwrap_or("User");
    let user_text = persona.map(|p| p.persona.as_str()).unwrap_or("");

    let other_names: Vec<&str> = live
        .iter()
        .filter(|(slug, _)| *slug != speaker_slug)
        .map(|(_, c)| c.name.as_str())
        .collect();

    let characters_block = render_characters_block(&live, speaker_slug, session.card_assembly);
    let roster_block = render_roster_block(&live, speaker_slug, session.card_assembly);
    let scene_block = render_scene_block(&live);
    let user_block = user_text.to_owned();
    let examples_block = render_examples_block(&live, speaker_slug, session.card_assembly);

    let opening = "You are running a group roleplay scene with multiple characters. On this turn you will reply as exactly one character, named below. Stay strictly in that character. Do not narrate, quote, or speak as any other character or as the user. Reply with one message in the named character's voice and stop.";
    let others_clause = if other_names.is_empty() {
        "any other character".to_owned()
    } else {
        other_names.join(", ")
    };
    let closing = format!(
        "You are now {active}. Reply as {active} in one message. Do not write as {others_clause} or {user_name}. Do not narrate other characters' actions or dialogue. End the message naturally; do not write another character's name on a new line.",
        active = active_card.name,
    );

    let system = if let Some(tpl) = template {
        let vars = crate::preset::ContextVars {
            system: opening.to_owned(),
            description: String::new(),
            personality: String::new(),
            scenario: scene_block.clone(),
            persona: user_block.clone(),
            wi_before: String::new(),
            wi_after: String::new(),
            mes_examples: examples_block.clone(),
            characters_block: characters_block.clone(),
            roster_block: roster_block.clone(),
            active_speaker: active_card.name.clone(),
            other_speakers: other_names.join(", "),
        };
        let body = tpl.render_story_string(&vars);
        let mut parts = vec![body];
        if !characters_block.is_empty() && !parts[0].contains(&characters_block) {
            parts.push(characters_block.clone());
        }
        if !roster_block.is_empty() && !parts[0].contains(&roster_block) {
            parts.push(roster_block.clone());
        }
        parts.push(format!("<active_speaker>{}</active_speaker>", active_card.name));
        parts.push(closing.clone());
        parts.join("\n\n")
    } else {
        let mut parts = vec![opening.to_owned()];
        if !scene_block.is_empty() {
            parts.push(format!("<scene>\n{scene_block}\n</scene>"));
        }
        parts.push(characters_block.clone());
        if !roster_block.is_empty() {
            parts.push(roster_block.clone());
        }
        if !user_block.is_empty() {
            parts.push(format!("<user name=\"{user_name}\">\n{user_block}\n</user>"));
        }
        if !examples_block.is_empty() {
            parts.push(format!("<examples>\n{examples_block}\n</examples>"));
        }
        parts.push(format!("<active_speaker>{}</active_speaker>", active_card.name));
        parts.push(closing);
        parts.join("\n\n")
    };

    let prefill = format!("{}: ", active_card.name);

    let mut stop_sequences: Vec<String> = Vec::new();
    for (slug, card) in &live {
        if *slug == speaker_slug {
            continue;
        }
        stop_sequences.push(format!("\n{}:", card.name));
        stop_sequences.push(format!("\n[{}]:", card.name));
    }
    stop_sequences.push(format!("\n{user_name}:"));
    stop_sequences.push(format!("\n[{user_name}]:"));
    stop_sequences.push("\n</".to_owned());

    Ok(TurnPrompt { system, prefill, stop_sequences })
}

fn render_characters_block(
    live: &[(&str, &CharacterCard)],
    speaker_slug: &str,
    mode: CardAssembly,
) -> String {
    let included: Vec<&(&str, &CharacterCard)> = match mode {
        CardAssembly::JoinCards => live.iter().collect(),
        CardAssembly::SwapCards => live.iter().filter(|(slug, _)| *slug == speaker_slug).collect(),
    };
    let mut out = String::from("<characters>\n");
    for (_slug, card) in included {
        out.push_str(&format!("  <character name=\"{}\">\n", card.name));
        if !card.description.is_empty() {
            out.push_str(&format!("    <description>{}</description>\n", card.description));
        }
        if !card.personality.is_empty() {
            out.push_str(&format!("    <personality>{}</personality>\n", card.personality));
        }
        out.push_str("  </character>\n");
    }
    out.push_str("</characters>");
    out
}

fn render_roster_block(
    live: &[(&str, &CharacterCard)],
    speaker_slug: &str,
    mode: CardAssembly,
) -> String {
    if !matches!(mode, CardAssembly::SwapCards) {
        return String::new();
    }
    let others: Vec<&str> = live
        .iter()
        .filter(|(slug, _)| *slug != speaker_slug)
        .map(|(_, c)| c.name.as_str())
        .collect();
    if others.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("<roster>\nOther characters in this scene (do not speak as them):\n");
    for name in others {
        out.push_str(&format!("- {name}\n"));
    }
    out.push_str("</roster>");
    out
}

fn render_scene_block(live: &[(&str, &CharacterCard)]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut parts = Vec::new();
    for (_, c) in live {
        if !c.scenario.is_empty() && seen.insert(c.scenario.as_str()) {
            parts.push(c.scenario.clone());
        }
    }
    parts.join("\n\n")
}

fn render_examples_block(
    live: &[(&str, &CharacterCard)],
    speaker_slug: &str,
    mode: CardAssembly,
) -> String {
    let included: Vec<&(&str, &CharacterCard)> = match mode {
        CardAssembly::JoinCards => live.iter().collect(),
        CardAssembly::SwapCards => live.iter().filter(|(slug, _)| *slug == speaker_slug).collect(),
    };
    let mut parts = Vec::new();
    for (_, card) in included {
        if card.mes_example.is_empty() {
            continue;
        }
        let prefixed: String = card
            .mes_example
            .lines()
            .map(|l| {
                if l.is_empty() {
                    String::new()
                } else {
                    format!("{}: {}", card.name, l)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(prefixed);
    }
    parts.join("\n\n")
}

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

pub fn force_step(
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

    let max_ap = updated
        .iter()
        .map(|(_, ap)| *ap)
        .fold(f32::NEG_INFINITY, f32::max);

    let candidates: Vec<usize> = updated
        .iter()
        .enumerate()
        .filter(|(_, (_, ap))| (*ap - max_ap).abs() < 1e-6)
        .map(|(i, _)| i)
        .collect();

    let chosen = if candidates.len() == 1 {
        candidates[0]
    } else {
        match policy {
            ChatPolicy::RoundRobin => candidates[0],
            ChatPolicy::WeightedRandom => weighted_pick(&candidates, characters, rng),
        }
    };

    let chosen_slug = updated[chosen].0.clone();
    updated[chosen].1 -= ACTION_POINT_COST;

    Some(TurnDecision { speaker_slug: chosen_slug, updated_action_points: updated, snapshot_before })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn card(
        name: &str,
        desc: &str,
        personality: &str,
        scenario: &str,
        examples: &str,
    ) -> CharacterCard {
        CharacterCard {
            name: name.to_owned(),
            description: desc.to_owned(),
            personality: personality.to_owned(),
            scenario: scenario.to_owned(),
            first_mes: String::new(),
            mes_example: examples.to_owned(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            alternate_greetings: vec![],
            author_note: None,
        }
    }

    fn cards_map(items: &[(&str, CharacterCard)]) -> HashMap<String, CharacterCard> {
        items.iter().map(|(s, c)| ((*s).to_owned(), c.clone())).collect()
    }

    fn fixture_session(slugs: &[&str], assembly: CardAssembly) -> crate::session::Session {
        crate::session::Session {
            characters: slugs.iter().map(|s| CharacterAttachment::new(*s)).collect(),
            card_assembly: assembly,
            ..Default::default()
        }
    }

    #[test]
    fn build_turn_prompt_join_two_matches_fixture() {
        let cards = cards_map(&[
            (
                "alice",
                card("Alice", "A wandering bard.", "Cheerful.", "A tavern.", "Hi!\nGood evening."),
            ),
            ("bob", card("Bob", "A grumpy dwarf.", "Stoic.", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let p = build_turn_prompt(&session, &cards, None, None, "alice").unwrap();
        let expected = include_str!("group_chat_fixtures/join_two.txt");
        assert_eq!(p.system.trim(), expected.trim(), "system prompt mismatch");
        assert_eq!(p.prefill, "Alice: ");
        assert!(p.stop_sequences.contains(&"\nBob:".to_owned()));
        assert!(p.stop_sequences.contains(&"\n[Bob]:".to_owned()));
        assert!(p.stop_sequences.contains(&"\nUser:".to_owned()));
        assert!(p.stop_sequences.contains(&"\n</".to_owned()));
        assert!(!p.stop_sequences.iter().any(|s| s == "\nAlice:"));
    }

    #[test]
    fn build_turn_prompt_join_three_with_persona_matches_fixture() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "A tavern.", "")),
            ("bob", card("Bob", "Dwarf.", "Stoic.", "", "")),
            ("charlie", card("Charlie", "Wizard.", "Curious.", "A tavern.", "")),
        ]);
        let mut session = fixture_session(&["alice", "bob", "charlie"], CardAssembly::JoinCards);
        session.persona = Some("me".to_owned());
        let persona = crate::persona::PersonaFile {
            name: "Trav".to_owned(),
            persona: "A traveler from the north.".to_owned(),
        };
        let p = build_turn_prompt(&session, &cards, Some(&persona), None, "bob").unwrap();
        let expected = include_str!("group_chat_fixtures/join_three.txt");
        assert_eq!(
            p.system.trim(),
            expected.trim(),
            "system prompt mismatch:\n--- got ---\n{}\n--- want ---\n{}",
            p.system,
            expected
        );
        assert_eq!(p.prefill, "Bob: ");
        assert!(p.stop_sequences.contains(&"\nTrav:".to_owned()));
    }

    #[test]
    fn build_turn_prompt_speaker_not_attached_errors() {
        let cards = cards_map(&[("alice", card("Alice", "", "", "", ""))]);
        let session = fixture_session(&["alice"], CardAssembly::JoinCards);
        let err = build_turn_prompt(&session, &cards, None, None, "ghost").unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn build_turn_prompt_missing_card_errors() {
        let cards = cards_map(&[]);
        let session = fixture_session(&["alice"], CardAssembly::JoinCards);
        let err = build_turn_prompt(&session, &cards, None, None, "alice").unwrap_err();
        assert!(err.to_string().contains("missing card"));
    }

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
    fn force_step_advances_even_when_no_one_eligible() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.3, 0.0), att("b", 0.1, 0.0)];
        let d = force_step(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert_eq!(d.speaker_slug, "a");
        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        assert!((new_a - (-0.7)).abs() < 1e-5, "a paid 1.0 from 0.3 → -0.7");
    }

    #[test]
    fn force_step_returns_none_for_empty_characters() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs: Vec<CharacterAttachment> = vec![];
        assert!(force_step(&cs, ChatPolicy::RoundRobin, &mut rng).is_none());
    }

    #[test]
    fn force_step_picks_highest_ap_candidate() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.2, 0.1), att("b", 0.5, 0.6)];
        // After increment: a=0.3, b=1.1. Max is b.
        let d = force_step(&cs, ChatPolicy::RoundRobin, &mut rng).unwrap();
        assert_eq!(d.speaker_slug, "b");
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

    fn run_rounds(
        characters: Vec<CharacterAttachment>,
        policy: ChatPolicy,
        rounds: usize,
        seed: u64,
    ) -> Vec<String> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = characters;
        let mut order = Vec::new();
        for _ in 0..rounds {
            let Some(d) = decide_next_speaker(&state, policy, &mut rng) else { break; };
            order.push(d.speaker_slug.clone());
            for (slug, ap) in d.updated_action_points {
                if let Some(c) = state.iter_mut().find(|c| c.slug == slug) {
                    c.action_points = ap;
                }
            }
        }
        order
    }

    #[test]
    fn round_robin_produces_predictable_alternation_with_equal_talkativeness() {
        // Starting AP 0.5 each: iter 1 pushes both to 1.0 (tie → a), iter 2 pushes a to 0.5 / b to 1.5 (only b).
        let cs = vec![att("a", 0.5, 0.5), att("b", 0.5, 0.5)];
        let order = run_rounds(cs, ChatPolicy::RoundRobin, 8, 0);
        assert_eq!(order, vec!["a", "b", "a", "b", "a", "b", "a", "b"]);
    }

    #[test]
    fn high_talkativeness_speaks_more_often() {
        // Starting AP 0.5 each so loud crosses threshold immediately on iter 1.
        let cs = vec![att("loud", 0.9, 0.5), att("quiet", 0.2, 0.5)];
        let order = run_rounds(cs, ChatPolicy::RoundRobin, 20, 0);
        let loud = order.iter().filter(|s| *s == "loud").count();
        let quiet = order.iter().filter(|s| *s == "quiet").count();
        assert!(loud > quiet * 2, "loud={loud}, quiet={quiet}");
    }

    #[test]
    fn build_turn_prompt_swap_three_includes_only_active_card_and_roster() {
        let cards = cards_map(&[
            ("alice",   card("Alice",   "Bard.",   "Cheerful.", "A tavern.", "")),
            ("bob",     card("Bob",     "Dwarf.",  "Stoic.",    "",          "")),
            ("charlie", card("Charlie", "Wizard.", "Curious.",  "",          "")),
        ]);
        let session = fixture_session(&["alice", "bob", "charlie"], CardAssembly::SwapCards);
        let p = build_turn_prompt(&session, &cards, None, None, "alice").unwrap();
        let expected = include_str!("group_chat_fixtures/swap_three.txt");
        assert_eq!(p.system.trim(), expected.trim(),
            "swap-three mismatch:\n--- got ---\n{}\n--- want ---\n{}", p.system, expected);

        assert!(p.system.contains("<character name=\"Alice\">"));
        assert!(!p.system.contains("<character name=\"Bob\">"));
        assert!(!p.system.contains("<character name=\"Charlie\">"));
        assert!(p.system.contains("<roster>"));
        assert!(p.system.contains("- Bob"));
        assert!(p.system.contains("- Charlie"));
    }

    #[test]
    fn build_turn_prompt_excludes_attachments_with_missing_cards() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "A tavern.", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob", "ghost"], CardAssembly::JoinCards);
        let p = build_turn_prompt(&session, &cards, None, None, "alice").unwrap();
        assert!(!p.system.contains("ghost"));
        assert!(!p.stop_sequences.iter().any(|s| s.contains("ghost")));
        assert!(!p.stop_sequences.iter().any(|s| s.contains("Ghost")));
    }

    #[test]
    fn build_turn_prompt_template_without_characters_block_still_injects_it() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "A tavern.", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.", "",          "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let template = crate::preset::ContextPreset {
            name: "no-roster".to_owned(),
            story_string:
                "{{#if system}}{{system}}\n{{/if}}{{#if persona}}{{persona}}\n{{/if}}{{trim}}"
                    .to_owned(),
            example_separator: String::new(),
            chat_start: String::new(),
            story_string_position: 0,
            story_string_depth: 0,
            story_string_role: 0,
        };
        let p = build_turn_prompt(&session, &cards, None, Some(&template), "alice").unwrap();
        assert!(
            p.system.contains("<character name=\"Alice\">"),
            "characters block must appear even when template omits it: {}",
            p.system,
        );
        assert!(
            p.system.contains("<character name=\"Bob\">"),
            "characters block must include all attached cards: {}",
            p.system,
        );
        assert!(
            p.system.contains("<active_speaker>Alice</active_speaker>"),
            "active speaker tag missing: {}",
            p.system,
        );
    }

    #[test]
    fn build_turn_prompt_template_with_characters_block_does_not_duplicate_it() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.",   "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let template = crate::preset::ContextPreset {
            name: "with-block".to_owned(),
            story_string: "{{characters_block}}".to_owned(),
            example_separator: String::new(),
            chat_start: String::new(),
            story_string_position: 0,
            story_string_depth: 0,
            story_string_role: 0,
        };
        let p = build_turn_prompt(&session, &cards, None, Some(&template), "alice").unwrap();
        assert_eq!(
            p.system.matches("<character name=\"Alice\">").count(),
            1,
            "characters block should not be duplicated: {}",
            p.system,
        );
    }

    #[test]
    fn build_turn_prompt_template_with_persona_renders_persona_text() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.",   "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let persona = crate::persona::PersonaFile {
            name: "Trav".to_owned(),
            persona: "A traveler from the north.".to_owned(),
        };
        let template = crate::preset::ContextPreset {
            name: "default-like".to_owned(),
            story_string: "{{#if system}}{{system}}\n{{/if}}{{#if persona}}{{persona}}\n{{/if}}{{trim}}"
                .to_owned(),
            example_separator: String::new(),
            chat_start: String::new(),
            story_string_position: 0,
            story_string_depth: 0,
            story_string_role: 0,
        };
        let p = build_turn_prompt(&session, &cards, Some(&persona), Some(&template), "alice")
            .unwrap();
        assert!(
            p.system.contains("A traveler from the north."),
            "persona text missing from system prompt: {}",
            p.system,
        );
    }
}
