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
    /// Optional short system message to inject between the last chat message and the
    /// assistant prefill (e.g. `[Write the next reply only as Alice.]`). Mirrors
    /// SillyTavern's `group_nudge_prompt` (openai.js:114). The renderer is responsible
    /// for emitting this as a `Role::System` block immediately before the assistant turn.
    pub nudge: Option<String>,
}

/// Inputs to `build_turn_prompt`. Bundled into a struct so callers don't have to track a
/// long positional argument list, and so future additions don't churn every call site.
pub struct TurnPromptInputs<'a> {
    pub session: &'a Session,
    pub cards: &'a HashMap<String, CharacterCard>,
    pub persona: Option<&'a PersonaFile>,
    pub template: Option<&'a ContextPreset>,
    pub speaker_slug: &'a str,
    /// User-configured roleplay prompt, used as the `{{system}}` slot in the context
    /// preset template (the same role `power_user.sysprompt.content` plays in
    /// SillyTavern). `None` or empty leaves the system slot empty.
    pub base_system_prompt: Option<&'a str>,
    /// Group nudge template with `{{char}}` / `{{user}}` macros.
    /// `None` or empty disables the nudge entirely.
    pub nudge_template: Option<&'a str>,
}

/// Builds the per-turn prompt for one group-chat speaker.
///
/// Structure mirrors SillyTavern's text-completion flow (see script.js:5073-5145 +
/// group-chats.js:497-571): the user's roleplay prompt fills the `{{system}}` slot,
/// the active speaker's card (or all members joined for `JoinCards`) fills
/// `{{description}}` / `{{personality}}` / `{{scenario}}` / `{{mesExamples}}`, and
/// the persona text fills `{{persona}}`. A short nudge naming the active speaker is
/// returned separately so the caller can inject it as a system message immediately
/// before the assistant turn opens. `{{char}}` and `{{user}}` macros are substituted
/// throughout.
pub fn build_turn_prompt(inputs: TurnPromptInputs<'_>) -> Result<TurnPrompt> {
    ensure!(
        inputs
            .session
            .characters
            .iter()
            .any(|a| a.slug == inputs.speaker_slug),
        "speaker {} is not attached to this session",
        inputs.speaker_slug,
    );
    let active_card = inputs
        .cards
        .get(inputs.speaker_slug)
        .ok_or_else(|| anyhow!("missing card for speaker {}", inputs.speaker_slug))?;

    let live: Vec<(&str, &CharacterCard)> = inputs
        .session
        .characters
        .iter()
        .filter_map(|a| inputs.cards.get(&a.slug).map(|c| (a.slug.as_str(), c)))
        .collect();

    let user_name = inputs.persona.map(|p| p.name.as_str()).unwrap_or("User");
    let user_text = inputs.persona.map(|p| p.persona.as_str()).unwrap_or("");
    let active_name = active_card.name.as_str();

    // Resolve description / personality / scenario / mes_examples per card-assembly mode.
    // Mirrors SillyTavern's `getGroupCharacterCardsLazy` (group-chats.js:497):
    // SwapCards (~ ST SWAP) uses the active speaker's card alone; JoinCards (~ ST APPEND)
    // joins each member's field with a `[<Label> for <Name>]` header on its own line,
    // then the field content. The header is required: without name binding the model
    // can't tell which trait belongs to which character, and tends to fall into omniscient
    // narration of the group rather than speaking as the active character.
    // SillyTavern exposes this header as the user-configurable
    // `generation_mode_join_prefix` (default empty); we hardcode a sensible default.
    // mes_example is joined raw — character-name prefixing inside dialogue lines is
    // the card author's responsibility, matching the TavernAI v2 card spec.
    let (description, personality, scenario, mes_examples) = match inputs.session.card_assembly {
        CardAssembly::SwapCards => (
            active_card.description.clone(),
            active_card.personality.clone(),
            active_card.scenario.clone(),
            active_card.mes_example.clone(),
        ),
        CardAssembly::JoinCards => (
            join_field_labeled(&live, "Description", |c| c.description.as_str()),
            join_field_labeled(&live, "Personality", |c| c.personality.as_str()),
            join_field_labeled(&live, "Scenario", |c| c.scenario.as_str()),
            join_field_raw(&live, |c| c.mes_example.as_str()),
        ),
    };

    let subst = |s: &str| crate::template::apply_template_vars(s, active_name, user_name);

    let system_text = inputs.base_system_prompt.map(subst).unwrap_or_default();

    let body = if let Some(tpl) = inputs.template {
        let other_names: Vec<&str> = live
            .iter()
            .filter(|(s, _)| *s != inputs.speaker_slug)
            .map(|(_, c)| c.name.as_str())
            .collect();
        let vars = crate::preset::ContextVars {
            system: system_text,
            description: subst(&description),
            personality: subst(&personality),
            scenario: subst(&scenario),
            persona: subst(user_text),
            wi_before: String::new(),
            wi_after: String::new(),
            mes_examples: subst(&mes_examples),
            // Legacy XML slots intentionally left empty: pre-rewrite presets that
            // referenced {{characters_block}} / {{roster_block}} now resolve to nothing.
            characters_block: String::new(),
            roster_block: String::new(),
            active_speaker: active_name.to_owned(),
            other_speakers: other_names.join(", "),
        };
        tpl.render_story_string(&vars)
    } else {
        // No template: simple newline-joined sections. Each section is independently
        // optional so empty fields don't introduce blank lines.
        let mut parts: Vec<String> = Vec::new();
        if !system_text.is_empty() {
            parts.push(system_text);
        }
        if !description.is_empty() {
            parts.push(subst(&description));
        }
        if !personality.is_empty() {
            parts.push(subst(&personality));
        }
        if !scenario.is_empty() {
            parts.push(subst(&scenario));
        }
        if !user_text.is_empty() {
            parts.push(subst(user_text));
        }
        if !mes_examples.is_empty() {
            parts.push(subst(&mes_examples));
        }
        parts.join("\n")
    };

    let prefill = format!("{active_name}: ");

    let mut stop_sequences: Vec<String> = Vec::new();
    for (slug, card) in &live {
        if *slug == inputs.speaker_slug {
            continue;
        }
        stop_sequences.push(format!("\n{}:", card.name));
        stop_sequences.push(format!("\n[{}]:", card.name));
    }
    stop_sequences.push(format!("\n{user_name}:"));
    stop_sequences.push(format!("\n[{user_name}]:"));

    let nudge = inputs.nudge_template.and_then(|t| {
        let s = subst(t);
        if s.is_empty() { None } else { Some(s) }
    });

    Ok(TurnPrompt {
        system: body,
        prefill,
        stop_sequences,
        nudge,
    })
}

fn join_field_raw<F>(live: &[(&str, &CharacterCard)], get: F) -> String
where
    F: Fn(&CharacterCard) -> &str,
{
    live.iter()
        .map(|(_, c)| get(c).trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn join_field_labeled<F>(live: &[(&str, &CharacterCard)], label: &str, get: F) -> String
where
    F: Fn(&CharacterCard) -> &str,
{
    live.iter()
        .filter_map(|(_, c)| {
            let v = get(c).trim();
            if v.is_empty() {
                None
            } else {
                Some(format!("[{label} for {}]\n{v}", c.name))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

/// Uniformly random fallback when no character is over the action-point threshold.
///
/// Mirrors `decide_next_speaker`'s arithmetic (apply +talkativeness, subtract one
/// `ACTION_POINT_COST` from the chosen speaker), but selects the speaker uniformly
/// at random instead of by AP. Callers must yield to the user after running the
/// returned turn — chaining this into another `decide_next_speaker` call lets the
/// non-chosen characters' freshly-incremented AP cross the threshold and produce a
/// second back-to-back turn from a single user message.
pub fn pick_random_speaker(
    characters: &[CharacterAttachment],
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

    let chosen = rng.random_range(0..characters.len());

    let chosen_slug = updated[chosen].0.clone();
    updated[chosen].1 -= ACTION_POINT_COST;

    Some(TurnDecision {
        speaker_slug: chosen_slug,
        updated_action_points: updated,
        snapshot_before,
    })
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

    fn inputs<'a>(
        session: &'a Session,
        cards: &'a HashMap<String, CharacterCard>,
        speaker: &'a str,
    ) -> TurnPromptInputs<'a> {
        TurnPromptInputs {
            session,
            cards,
            persona: None,
            template: None,
            speaker_slug: speaker,
            base_system_prompt: None,
            nudge_template: None,
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
        let p = build_turn_prompt(inputs(&session, &cards, "alice")).unwrap();
        let expected = include_str!("group_chat_fixtures/join_two.txt");
        assert_eq!(p.system.trim(), expected.trim(), "system prompt mismatch");
        assert_eq!(p.prefill, "Alice: ");
        assert!(p.stop_sequences.contains(&"\nBob:".to_owned()));
        assert!(p.stop_sequences.contains(&"\n[Bob]:".to_owned()));
        assert!(p.stop_sequences.contains(&"\nUser:".to_owned()));
        assert!(!p.stop_sequences.iter().any(|s| s == "\nAlice:"));
        assert!(p.nudge.is_none(), "no nudge template configured");
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
        let p = build_turn_prompt(TurnPromptInputs {
            persona: Some(&persona),
            ..inputs(&session, &cards, "bob")
        })
        .unwrap();
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
        let err = build_turn_prompt(inputs(&session, &cards, "ghost")).unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn build_turn_prompt_missing_card_errors() {
        let cards = cards_map(&[]);
        let session = fixture_session(&["alice"], CardAssembly::JoinCards);
        let err = build_turn_prompt(inputs(&session, &cards, "alice")).unwrap_err();
        assert!(err.to_string().contains("missing card"));
    }

    #[test]
    fn build_turn_prompt_emits_nudge_with_macro_substitution() {
        let cards = cards_map(&[
            ("alice", card("Alice", "", "", "", "")),
            ("bob", card("Bob", "", "", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let p = build_turn_prompt(TurnPromptInputs {
            nudge_template: Some("[Write the next reply only as {{char}}.]"),
            ..inputs(&session, &cards, "alice")
        })
        .unwrap();
        assert_eq!(
            p.nudge.as_deref(),
            Some("[Write the next reply only as Alice.]"),
        );
    }

    #[test]
    fn build_turn_prompt_empty_nudge_template_yields_none() {
        let cards = cards_map(&[
            ("alice", card("Alice", "", "", "", "")),
            ("bob", card("Bob", "", "", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let p = build_turn_prompt(TurnPromptInputs {
            nudge_template: Some(""),
            ..inputs(&session, &cards, "alice")
        })
        .unwrap();
        assert!(p.nudge.is_none(), "empty nudge template should not emit a message");
    }

    #[test]
    fn build_turn_prompt_substitutes_macros_in_system_prompt() {
        let cards = cards_map(&[
            ("alice", card("Alice", "", "", "", "")),
            ("bob", card("Bob", "", "", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let persona = crate::persona::PersonaFile {
            name: "Trav".to_owned(),
            persona: String::new(),
        };
        let p = build_turn_prompt(TurnPromptInputs {
            persona: Some(&persona),
            base_system_prompt: Some(
                "You are {{char}} in a roleplay with {{user}}.",
            ),
            ..inputs(&session, &cards, "alice")
        })
        .unwrap();
        assert_eq!(p.system.trim(), "You are Alice in a roleplay with Trav.");
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
    fn pick_random_speaker_returns_none_for_empty_characters() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs: Vec<CharacterAttachment> = vec![];
        assert!(pick_random_speaker(&cs, &mut rng).is_none());
    }

    #[test]
    fn pick_random_speaker_applies_increment_and_cost() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.3, 0.0), att("b", 0.1, 0.0)];
        let d = pick_random_speaker(&cs, &mut rng).unwrap();

        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        let new_b = d.updated_action_points.iter().find(|(s, _)| s == "b").unwrap().1;

        match d.speaker_slug.as_str() {
            "a" => {
                assert!((new_a - (-0.7)).abs() < 1e-5, "a should pay 1.0 from 0.3 → -0.7");
                assert!((new_b - 0.1).abs() < 1e-5, "b should only get +0.1 increment");
            }
            "b" => {
                assert!((new_a - 0.3).abs() < 1e-5, "a should only get +0.3 increment");
                assert!((new_b - (-0.9)).abs() < 1e-5, "b should pay 1.0 from 0.1 → -0.9");
            }
            other => panic!("unexpected speaker: {other}"),
        }
    }

    #[test]
    fn pick_random_speaker_snapshot_before_captures_pre_increment_state() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 0.5), att("b", 0.5, 0.7)];
        let d = pick_random_speaker(&cs, &mut rng).unwrap();
        assert!((d.snapshot_before["a"] - 0.5).abs() < 1e-5);
        assert!((d.snapshot_before["b"] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn pick_random_speaker_distribution_is_uniform() {
        let cs = vec![att("a", 0.5, 0.0), att("b", 0.5, 0.0), att("c", 0.5, 0.0)];
        let mut rng = StdRng::seed_from_u64(7);
        let mut counts = [0u32; 3];
        for _ in 0..3000 {
            let d = pick_random_speaker(&cs, &mut rng).unwrap();
            match d.speaker_slug.as_str() {
                "a" => counts[0] += 1,
                "b" => counts[1] += 1,
                "c" => counts[2] += 1,
                other => panic!("unexpected speaker: {other}"),
            }
        }
        for &n in &counts {
            assert!(
                (800..=1200).contains(&n),
                "expected ~1000 each for uniform pick, got {counts:?}"
            );
        }
    }

    #[test]
    fn pick_random_speaker_ignores_talkativeness_weighting() {
        // Loud character has 4x talkativeness but should still be picked uniformly.
        let cs = vec![att("loud", 0.9, 0.0), att("quiet", 0.2, 0.0)];
        let mut rng = StdRng::seed_from_u64(1234);
        let mut loud_count = 0;
        for _ in 0..2000 {
            let d = pick_random_speaker(&cs, &mut rng).unwrap();
            if d.speaker_slug == "loud" {
                loud_count += 1;
            }
        }
        assert!(
            (800..=1200).contains(&loud_count),
            "uniform pick should be ~1000/2000 regardless of talkativeness, got {loud_count}"
        );
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
    fn build_turn_prompt_swap_includes_only_active_card_fields() {
        let cards = cards_map(&[
            ("alice",   card("Alice",   "Bard.",   "Cheerful.", "A tavern.", "")),
            ("bob",     card("Bob",     "Dwarf.",  "Stoic.",    "",          "")),
            ("charlie", card("Charlie", "Wizard.", "Curious.",  "",          "")),
        ]);
        let session = fixture_session(&["alice", "bob", "charlie"], CardAssembly::SwapCards);
        let p = build_turn_prompt(inputs(&session, &cards, "alice")).unwrap();
        let expected = include_str!("group_chat_fixtures/swap_three.txt");
        assert_eq!(
            p.system.trim(),
            expected.trim(),
            "swap-three mismatch:\n--- got ---\n{}\n--- want ---\n{}",
            p.system,
            expected
        );

        assert!(p.system.contains("Bard."));
        assert!(p.system.contains("Cheerful."));
        // SwapCards uses ONLY the active card; other members' fields must not appear.
        assert!(!p.system.contains("Dwarf."));
        assert!(!p.system.contains("Stoic."));
        assert!(!p.system.contains("Wizard."));
        assert!(!p.system.contains("Curious."));
    }

    #[test]
    fn build_turn_prompt_join_combines_fields_across_members() {
        let cards = cards_map(&[
            ("alice",   card("Alice",   "Bard.",   "Cheerful.", "", "")),
            ("bob",     card("Bob",     "Dwarf.",  "Stoic.",    "", "")),
            ("charlie", card("Charlie", "Wizard.", "Curious.",  "", "")),
        ]);
        let session = fixture_session(&["alice", "bob", "charlie"], CardAssembly::JoinCards);
        let p = build_turn_prompt(inputs(&session, &cards, "alice")).unwrap();
        // Every member's description and personality is present.
        for term in ["Bard.", "Dwarf.", "Wizard.", "Cheerful.", "Stoic.", "Curious."] {
            assert!(p.system.contains(term), "missing {term:?} in: {}", p.system);
        }
    }

    #[test]
    fn build_turn_prompt_excludes_attachments_with_missing_cards() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "A tavern.", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob", "ghost"], CardAssembly::JoinCards);
        let p = build_turn_prompt(inputs(&session, &cards, "alice")).unwrap();
        assert!(!p.system.contains("ghost"));
        assert!(!p.stop_sequences.iter().any(|s| s.contains("ghost")));
        assert!(!p.stop_sequences.iter().any(|s| s.contains("Ghost")));
    }

    #[test]
    fn build_turn_prompt_default_template_renders_persona_and_card_fields() {
        let cards = cards_map(&[
            ("alice", card("Alice", "Bard.", "Cheerful.", "A tavern.", "")),
            ("bob",   card("Bob",   "Dwarf.", "Stoic.",   "",          "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::JoinCards);
        let persona = crate::persona::PersonaFile {
            name: "Trav".to_owned(),
            persona: "A traveler from the north.".to_owned(),
        };
        // Mirrors the production "Default" template (story_string with `{{system}}`
        // / `{{description}}` / `{{personality}}` / `{{scenario}}` / `{{persona}}`).
        let template = crate::preset::ContextPreset {
            name: "default-like".to_owned(),
            story_string: "{{#if system}}{{system}}\n{{/if}}\
                {{#if description}}{{description}}\n{{/if}}\
                {{#if personality}}{{personality}}\n{{/if}}\
                {{#if scenario}}{{scenario}}\n{{/if}}\
                {{#if persona}}{{persona}}\n{{/if}}{{trim}}"
                .to_owned(),
            example_separator: String::new(),
            chat_start: String::new(),
            story_string_position: 0,
            story_string_depth: 0,
            story_string_role: 0,
        };
        let p = build_turn_prompt(TurnPromptInputs {
            persona: Some(&persona),
            template: Some(&template),
            base_system_prompt: Some("You are {{char}} chatting with {{user}}."),
            ..inputs(&session, &cards, "alice")
        })
        .unwrap();
        assert!(
            p.system.contains("You are Alice chatting with Trav."),
            "system prompt with macros missing: {}",
            p.system,
        );
        assert!(p.system.contains("A traveler from the north."));
        assert!(p.system.contains("Bard."));
        assert!(p.system.contains("Cheerful."));
        assert!(p.system.contains("A tavern."));
    }

    #[test]
    fn build_turn_prompt_substitutes_macros_in_card_fields() {
        let cards = cards_map(&[
            (
                "alice",
                card("Alice", "{{char}} runs the tavern.", "Friendly to {{user}}.", "", ""),
            ),
            ("bob", card("Bob", "", "", "", "")),
        ]);
        let session = fixture_session(&["alice", "bob"], CardAssembly::SwapCards);
        let persona = crate::persona::PersonaFile {
            name: "Trav".to_owned(),
            persona: String::new(),
        };
        let p = build_turn_prompt(TurnPromptInputs {
            persona: Some(&persona),
            ..inputs(&session, &cards, "alice")
        })
        .unwrap();
        assert!(p.system.contains("Alice runs the tavern."), "got: {}", p.system);
        assert!(p.system.contains("Friendly to Trav."), "got: {}", p.system);
    }
}
