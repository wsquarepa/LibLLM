//! Group-chat runtime: per-session character attachments, Honkai-Star-Rail-style
//! action-value turn-order engine (lower AV = sooner), and per-turn prompt assembly.
//! Pure logic, no I/O.
//!
//! # Turn order
//!
//! Each character has a `talkativeness` value (treated as SPD) and an `action_points`
//! value (the field name is historical; semantically it is an *action value* — time
//! until the character's next turn). At each iteration of the cascade:
//!
//! 1. The character with the lowest `action_points` among those that haven't yet
//!    spoken this round is selected.
//! 2. Every character's `action_points` decreases by that minimum (the active
//!    character thus reaches zero).
//! 3. The active character speaks, then their `action_points` is reset to their
//!    base action value, `BASE_ACTION_VALUE_NUMERATOR / talkativeness`.
//!
//! Characters who haven't spoken this round carry their reduced AVs into the next
//! iteration; characters who already spoke have `spoke_this_round = true` and are
//! filtered out until the user sends another message (which clears the flags and
//! renormalizes AVs so the minimum is zero — this prevents long-term drift in
//! magnitude).

use serde::{Deserialize, Serialize};

pub const MAX_GROUP_SIZE: usize = 8;
pub const DEFAULT_TALKATIVENESS: f32 = 0.5;
/// SPD-to-period scaling constant. Mirrors HSR's `Base AV = 10000 / SPD`, scaled to
/// our (talkativeness ∈ [0, 1]) range: with `numerator = 1`, a character at
/// talkativeness 1.0 (the cap) has base AV 1.0; at 0.5, base AV 2.0; at 1/6, base AV 6.
pub const BASE_ACTION_VALUE_NUMERATOR: f32 = 1.0;
/// Talkativeness slider granularity. Six notches, mapping to talkativeness values
/// 1/6, 2/6, ..., 6/6 (notch 0 mutes the character entirely).
pub const TALKATIVENESS_NOTCHES: u8 = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAttachment {
    pub slug: String,
    pub talkativeness: f32,
    /// Action value (HSR-style): time until this character's next turn. Lower = sooner.
    /// Stored under the legacy column name `action_points` for back-compat; semantics
    /// is action value, not action points. Migration v8 resets this column to zero on
    /// upgrade, so any old AP-threshold values do not contaminate the new ordering.
    pub action_points: f32,
    /// Transient: set when this character has already spoken since the most recent user
    /// message. Cleared in `push_user_segments`. Not persisted across session reload —
    /// reopening a chat starts a fresh round.
    #[serde(default, skip)]
    pub spoke_this_round: bool,
}

impl CharacterAttachment {
    pub fn new(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            talkativeness: DEFAULT_TALKATIVENESS,
            action_points: 0.0,
            spoke_this_round: false,
        }
    }
}

/// Computes a character's base action value (time until their next turn after they
/// just acted). Mirrors HSR: faster characters (higher talkativeness) have shorter
/// periods. A talkativeness of 0 returns `f32::INFINITY`, meaning the character is
/// muted and will never be chosen by the turn-order engine.
pub fn base_action_value(talkativeness: f32) -> f32 {
    if talkativeness <= 0.0 {
        f32::INFINITY
    } else {
        BASE_ACTION_VALUE_NUMERATOR / talkativeness
    }
}

/// Normalizes raw talkativeness values so they sum to 1.0. Negatives are clamped to 0.
/// If all weights are zero, returns a uniform distribution so callers always have a
/// well-defined relative ratio. Used for the percentage display in the settings
/// dialog and as the tiebreak weight when multiple eligible speakers share the same
/// AV under `WeightedRandom` policy.
pub fn normalized_talkativeness(characters: &[CharacterAttachment]) -> Vec<f32> {
    if characters.is_empty() {
        return Vec::new();
    }
    let raw: Vec<f32> = characters.iter().map(|c| c.talkativeness.max(0.0)).collect();
    let sum: f32 = raw.iter().sum();
    if sum <= 0.0 {
        let n = characters.len() as f32;
        return vec![1.0 / n; characters.len()];
    }
    raw.into_iter().map(|w| w / sum).collect()
}

/// Snaps a talkativeness value to the nearest notch in `[0, TALKATIVENESS_NOTCHES]`,
/// returning that notch index and the canonical f32 value for it.
pub fn talkativeness_to_notch(talkativeness: f32) -> u8 {
    let scaled = (talkativeness.clamp(0.0, 1.0) * TALKATIVENESS_NOTCHES as f32).round();
    (scaled as u8).min(TALKATIVENESS_NOTCHES)
}

/// Converts a notch index back into the canonical talkativeness f32 (notch/N).
pub fn notch_to_talkativeness(notch: u8) -> f32 {
    let n = notch.min(TALKATIVENESS_NOTCHES);
    n as f32 / TALKATIVENESS_NOTCHES as f32
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

/// Default amount of conversation time that elapses for one user message. The fastest
/// character (talkativeness 1.0, base AV 1.0) gets a turn each round; slower characters
/// accumulate progress over multiple rounds and eventually speak.
pub const DEFAULT_TURN_TIME_BUDGET: f32 = 1.0;

#[derive(Debug)]
pub struct TurnDecision {
    pub speaker_slug: String,
    /// New action-value for every attached character after this turn fires. The chosen
    /// speaker is reset to their base AV; everyone else has the minimum AV subtracted
    /// from theirs (HSR-style time advance).
    pub updated_action_points: Vec<(String, f32)>,
    /// Pre-turn snapshot of every character's action_value, recorded for the per-message
    /// diff dialog. Mirrors the field name on the stored snapshot (`pre_turn_action_points`).
    pub snapshot_before: HashMap<String, f32>,
    /// How much "conversation time" elapsed for this turn (i.e. the AV of the chosen
    /// speaker before they acted). Callers subtract this from the per-cascade
    /// remaining-time budget; when the next speaker's AV exceeds what's left, the
    /// cascade yields to the user.
    pub time_advanced: f32,
}

/// Picks the next speaker using HSR-style turn order. The caller passes the remaining
/// per-cascade time budget; if the next eligible character's AV exceeds that budget,
/// returns `None` (the cascade yields to the user). Passing `None` for `time_budget`
/// disables the check, used for the **forced first turn** of each cascade so at least
/// one character always speaks per user message.
///
/// Returns `None` when every attached character has already spoken this round
/// (cascade complete). Characters with `talkativeness == 0` have `f32::INFINITY` base
/// AV and effectively never speak unless they're the only non-spoken candidate and
/// the budget is unbounded (the forced first turn).
pub fn decide_next_speaker(
    characters: &[CharacterAttachment],
    policy: ChatPolicy,
    rng: &mut impl Rng,
    time_budget: Option<f32>,
) -> Option<TurnDecision> {
    if characters.is_empty() {
        return None;
    }

    let eligible: Vec<usize> = characters
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.spoke_this_round)
        .map(|(i, _)| i)
        .collect();
    if eligible.is_empty() {
        return None;
    }

    let snapshot_before: HashMap<String, f32> = characters
        .iter()
        .map(|c| (c.slug.clone(), c.action_points))
        .collect();

    let min_av = eligible
        .iter()
        .map(|&i| characters[i].action_points)
        .fold(f32::INFINITY, f32::min);

    if let Some(budget) = time_budget
        && min_av > budget
    {
        return None;
    }

    let candidates: Vec<usize> = eligible
        .iter()
        .copied()
        .filter(|&i| (characters[i].action_points - min_av).abs() < 1e-5)
        .collect();

    let chosen_idx = if candidates.len() == 1 {
        candidates[0]
    } else {
        match policy {
            ChatPolicy::RoundRobin => candidates[0],
            ChatPolicy::WeightedRandom => {
                let weights = normalized_talkativeness(characters);
                weighted_pick(&candidates, &weights, rng)
            }
        }
    };

    let updated: Vec<(String, f32)> = characters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let new_av = if i == chosen_idx {
                base_action_value(c.talkativeness)
            } else {
                c.action_points - min_av
            };
            (c.slug.clone(), new_av)
        })
        .collect();

    Some(TurnDecision {
        speaker_slug: characters[chosen_idx].slug.clone(),
        updated_action_points: updated,
        snapshot_before,
        time_advanced: min_av,
    })
}

fn weighted_pick(candidates: &[usize], weights: &[f32], rng: &mut impl Rng) -> usize {
    let candidate_weights: Vec<f32> = candidates.iter().map(|&i| weights[i].max(0.0)).collect();
    let total: f32 = candidate_weights.iter().sum();
    if total <= 0.0 {
        return candidates[0];
    }
    let mut roll = rng.random::<f32>() * total;
    for (k, w) in candidate_weights.iter().enumerate() {
        if roll < *w {
            return candidates[k];
        }
        roll -= w;
    }
    *candidates.last().expect("non-empty by guard")
}

/// Renormalizes action values so the minimum across all characters is zero. Called when
/// a new user message arrives (the start of a fresh cascade), to keep AV magnitudes
/// bounded across many rounds. Preserves relative ordering exactly.
pub fn renormalize_action_values(characters: &mut [CharacterAttachment]) {
    if characters.is_empty() {
        return;
    }
    let min_av = characters
        .iter()
        .map(|c| c.action_points)
        .filter(|av| av.is_finite())
        .fold(f32::INFINITY, f32::min);
    if !min_av.is_finite() || min_av == 0.0 {
        return;
    }
    for c in characters.iter_mut() {
        if c.action_points.is_finite() {
            c.action_points -= min_av;
        }
    }
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
        CharacterAttachment {
            slug: slug.to_owned(),
            talkativeness: talk,
            action_points: ap,
            spoke_this_round: false,
        }
    }

    fn att_spoken(slug: &str, talk: f32, ap: f32) -> CharacterAttachment {
        CharacterAttachment {
            slug: slug.to_owned(),
            talkativeness: talk,
            action_points: ap,
            spoke_this_round: true,
        }
    }

    #[test]
    fn normalized_talkativeness_sums_to_one() {
        let cs = vec![att("a", 0.4, 0.0), att("b", 0.5, 0.0), att("c", 0.6, 0.0)];
        let weights = normalized_talkativeness(&cs);
        let sum: f32 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "weights sum {} != 1.0", sum);
    }

    #[test]
    fn normalized_talkativeness_zero_returns_uniform() {
        let cs = vec![att("a", 0.0, 0.0), att("b", 0.0, 0.0)];
        let weights = normalized_talkativeness(&cs);
        assert!((weights[0] - 0.5).abs() < 1e-5);
        assert!((weights[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn base_action_value_inverse_of_talkativeness() {
        assert!((base_action_value(1.0) - 1.0).abs() < 1e-5);
        assert!((base_action_value(0.5) - 2.0).abs() < 1e-5);
        let one_sixth = 1.0_f32 / 6.0;
        assert!((base_action_value(one_sixth) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn base_action_value_zero_talkativeness_is_infinite() {
        assert_eq!(base_action_value(0.0), f32::INFINITY);
        assert_eq!(base_action_value(-0.1), f32::INFINITY);
    }

    #[test]
    fn notch_round_trip_is_stable() {
        for n in 0..=TALKATIVENESS_NOTCHES {
            let t = notch_to_talkativeness(n);
            assert_eq!(talkativeness_to_notch(t), n, "notch {n} did not round-trip");
        }
    }

    #[test]
    fn decide_next_returns_none_when_empty() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs: Vec<CharacterAttachment> = vec![];
        assert!(decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).is_none());
    }

    #[test]
    fn decide_next_picks_lowest_av_speaker() {
        let mut rng = StdRng::seed_from_u64(0);
        // a: AV=2 (waiting), b: AV=0.5 (sooner). b wins.
        let cs = vec![att("a", 0.5, 2.0), att("b", 0.5, 0.5)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert_eq!(d.speaker_slug, "b");
        // Time advance = 0.5. a: 2 - 0.5 = 1.5. b: reset to base = 1/0.5 = 2.
        let new_a = d.updated_action_points.iter().find(|(s, _)| s == "a").unwrap().1;
        let new_b = d.updated_action_points.iter().find(|(s, _)| s == "b").unwrap().1;
        assert!((new_a - 1.5).abs() < 1e-4, "new_a={new_a}");
        assert!((new_b - 2.0).abs() < 1e-4, "new_b={new_b}");
    }

    #[test]
    fn decide_next_skips_characters_that_already_spoke() {
        let mut rng = StdRng::seed_from_u64(0);
        // a has lower AV but already spoke; b should be picked despite higher AV.
        let cs = vec![att_spoken("a", 0.5, 0.0), att("b", 0.5, 1.5)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert_eq!(d.speaker_slug, "b", "should pick b because a already spoke");
    }

    #[test]
    fn decide_next_returns_none_when_all_eligible_spoke() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att_spoken("a", 0.5, 0.0), att_spoken("b", 0.5, 1.0)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None);
        assert!(d.is_none(), "no eligible characters left in this round");
    }

    #[test]
    fn decide_next_round_robin_ties_break_by_attach_index() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 0.0), att("b", 0.5, 0.0), att("c", 0.5, 0.0)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert_eq!(d.speaker_slug, "a", "all tied at AV=0; round-robin picks first attach");
    }

    #[test]
    fn decide_next_weighted_random_uses_talkativeness_at_ties() {
        // Both at AV=0 (tied for next), but b has 3x talkativeness. Over many rolls,
        // b should be picked roughly 3x as often.
        let mut counts = (0u32, 0u32);
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..3000 {
            let cs = vec![att("a", 0.25, 0.0), att("b", 0.75, 0.0)];
            let d = decide_next_speaker(&cs, ChatPolicy::WeightedRandom, &mut rng, None).unwrap();
            if d.speaker_slug == "a" { counts.0 += 1; } else { counts.1 += 1; }
        }
        assert!(
            counts.1 > counts.0 * 2,
            "expected b to win at least 2x: a={}, b={}",
            counts.0, counts.1,
        );
    }

    #[test]
    fn decide_next_zero_talkativeness_yields_to_others() {
        // a is muted (talk=0 -> base AV=inf); b should always be picked.
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.0, f32::INFINITY), att("b", 0.5, 1.0)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert_eq!(d.speaker_slug, "b");
    }

    #[test]
    fn decide_next_snapshot_before_captures_pre_advance_state() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 0.5), att("b", 0.5, 0.7)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert!((d.snapshot_before["a"] - 0.5).abs() < 1e-5);
        assert!((d.snapshot_before["b"] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn renormalize_action_values_subtracts_minimum() {
        let mut cs = vec![att("a", 0.5, -4.0), att("b", 0.5, -2.0), att("c", 0.5, 6.0)];
        renormalize_action_values(&mut cs);
        assert!((cs[0].action_points - 0.0).abs() < 1e-5);
        assert!((cs[1].action_points - 2.0).abs() < 1e-5);
        assert!((cs[2].action_points - 10.0).abs() < 1e-5);
    }

    #[test]
    fn renormalize_action_values_handles_infinity() {
        let mut cs = vec![att("a", 0.0, f32::INFINITY), att("b", 0.5, 2.0)];
        renormalize_action_values(&mut cs);
        // Min is 2.0 (infinity ignored). a stays infinite, b drops to 0.
        assert_eq!(cs[0].action_points, f32::INFINITY);
        assert!((cs[1].action_points - 0.0).abs() < 1e-5);
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
        let a = CharacterAttachment {
            slug: "alice".to_owned(),
            talkativeness: 0.7,
            action_points: 0.3,
            spoke_this_round: true,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: CharacterAttachment = serde_json::from_str(&s).unwrap();
        assert_eq!(back.slug, "alice");
        assert!((back.talkativeness - 0.7).abs() < f32::EPSILON);
        assert!((back.action_points - 0.3).abs() < f32::EPSILON);
        assert!(!back.spoke_this_round, "spoke_this_round must not persist across serde");
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

    /// Simulates `user_rounds` user messages. Within each round the cascade runs until
    /// either every character has spoken once (cap=1/character) or `max_per_round` is
    /// reached. Between rounds spoke_this_round is cleared and AVs are renormalized.
    fn simulate(
        characters: Vec<CharacterAttachment>,
        policy: ChatPolicy,
        user_rounds: usize,
        max_per_round: usize,
        seed: u64,
    ) -> Vec<Vec<String>> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = characters;
        let mut rounds: Vec<Vec<String>> = Vec::new();
        for _ in 0..user_rounds {
            for c in state.iter_mut() {
                c.spoke_this_round = false;
            }
            renormalize_action_values(&mut state);
            let mut order = Vec::new();
            for _ in 0..max_per_round {
                let Some(d) = decide_next_speaker(&state, policy, &mut rng, None) else { break };
                order.push(d.speaker_slug.clone());
                for (slug, av) in d.updated_action_points {
                    if let Some(c) = state.iter_mut().find(|c| c.slug == slug) {
                        c.action_points = av;
                    }
                }
                if let Some(c) = state.iter_mut().find(|c| c.slug == d.speaker_slug) {
                    c.spoke_this_round = true;
                }
            }
            rounds.push(order);
        }
        rounds
    }

    #[test]
    fn cascade_caps_at_one_turn_per_character_per_round() {
        let cs = vec![att("a", 0.5, 0.0), att("b", 0.5, 0.0), att("c", 0.5, 0.0)];
        let rounds = simulate(cs, ChatPolicy::RoundRobin, 3, 10, 0);
        for (i, order) in rounds.iter().enumerate() {
            let unique: std::collections::HashSet<&String> = order.iter().collect();
            assert_eq!(unique.len(), order.len(), "round {i}: duplicate speaker");
        }
    }

    #[test]
    fn cascade_orders_by_speed_higher_first() {
        // Loud character has high SPD (base AV 1), quiet has low (base AV 5).
        // Both start at AV=0, so first turn ties on AV; round-robin breaks toward loud.
        // After loud speaks, loud.AV=1, quiet.AV=0, so quiet goes next.
        let cs = vec![att("loud", 1.0, 0.0), att("quiet", 0.2, 0.0)];
        let rounds = simulate(cs, ChatPolicy::RoundRobin, 1, 5, 0);
        assert_eq!(rounds[0], vec!["loud", "quiet"]);
    }

    #[test]
    fn cascade_high_speed_dominates_across_many_rounds() {
        // After the warm-up, loud should overwhelmingly take the first slot each round
        // because its AV resets to 1.0 (vs. quiet's 5.0); time-advance never lets quiet
        // catch up unless it has been waiting.
        let cs = vec![att("loud", 1.0, 0.0), att("quiet", 0.2, 0.0)];
        let rounds = simulate(cs, ChatPolicy::RoundRobin, 20, 2, 0);
        let first_slot_loud = rounds.iter().filter(|r| r.first() == Some(&"loud".to_owned())).count();
        assert!(
            first_slot_loud >= 18,
            "expected loud to take the first slot in nearly every round: {first_slot_loud}/20",
        );
    }

    /// Drives the full per-user-round cascade: forced first turn, then budget-checked
    /// subsequent turns; renormalizes AVs between rounds and clears `spoke_this_round`.
    fn run_cascade(
        state: &mut [CharacterAttachment],
        policy: ChatPolicy,
        rng: &mut StdRng,
    ) -> Vec<String> {
        for c in state.iter_mut() {
            c.spoke_this_round = false;
        }
        renormalize_action_values(state);
        let mut order = Vec::new();
        let mut budget = DEFAULT_TURN_TIME_BUDGET;
        let mut first = true;
        loop {
            let tb = if first { None } else { Some(budget) };
            let Some(d) = decide_next_speaker(state, policy, rng, tb) else { break };
            order.push(d.speaker_slug.clone());
            budget -= d.time_advanced.max(0.0);
            for (slug, av) in d.updated_action_points {
                if let Some(c) = state.iter_mut().find(|c| c.slug == slug) {
                    c.action_points = av;
                }
            }
            if let Some(c) = state.iter_mut().find(|c| c.slug == d.speaker_slug) {
                c.spoke_this_round = true;
            }
            first = false;
        }
        order
    }

    #[test]
    fn cascade_with_budget_keeps_av_magnitudes_bounded() {
        // Run many user-rounds. With the time-budget gate, slow characters skip rounds
        // rather than being forced to speak every round; their AVs stay within
        // base-AV range and don't drift.
        let cs = vec![att("a", 1.0, 0.0), att("b", 0.5, 0.0), att("c", 0.16667, 0.0)];
        let mut state = cs;
        let mut rng = StdRng::seed_from_u64(0);
        for _ in 0..50 {
            run_cascade(&mut state, ChatPolicy::RoundRobin, &mut rng);
        }
        renormalize_action_values(&mut state);
        let finite_min = state
            .iter()
            .map(|c| c.action_points)
            .filter(|v| v.is_finite())
            .fold(f32::INFINITY, f32::min);
        assert!((finite_min - 0.0).abs() < 1e-3, "min AV after renormalize: {finite_min}");
        // Max base AV here is 6.0 (talk=1/6). With renormalize between rounds, max AV
        // stays under (max base AV + 1 time advance).
        let finite_max = state
            .iter()
            .map(|c| c.action_points)
            .filter(|v| v.is_finite())
            .fold(0.0_f32, f32::max);
        assert!(finite_max < 7.5, "unexpected AV magnitude after 50 rounds: {finite_max}");
    }

    #[test]
    fn cascade_long_run_speaks_proportional_to_speed() {
        // Each user-round runs the cascade. Over many rounds the per-character turn
        // count should track the SPD ratio (1.0 : 0.5 : 0.167 ≈ 6 : 3 : 1).
        let cs = vec![att("a", 1.0, 0.0), att("b", 0.5, 0.0), att("c", 0.16667, 0.0)];
        let mut state = cs;
        let mut rng = StdRng::seed_from_u64(0);
        let mut counts = (0u32, 0u32, 0u32);
        for _ in 0..120 {
            let order = run_cascade(&mut state, ChatPolicy::RoundRobin, &mut rng);
            for slug in order {
                match slug.as_str() {
                    "a" => counts.0 += 1,
                    "b" => counts.1 += 1,
                    "c" => counts.2 += 1,
                    other => panic!("unexpected speaker: {other}"),
                }
            }
        }
        // Each user-round forces one speaker, so a (highest SPD) ≈ 120.
        // b should be roughly half of a, c roughly a sixth.
        assert!(counts.0 >= 100, "a underrepresented: {counts:?}");
        assert!(counts.1 > counts.2 * 2, "expected b > 2*c: {counts:?}");
        assert!(counts.1 < counts.0, "b should not exceed a: {counts:?}");
    }

    #[test]
    fn decide_next_returns_none_when_av_exceeds_budget() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 2.0), att("b", 0.5, 3.0)];
        // Lowest AV is 2.0, but budget is only 1.0.
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, Some(1.0));
        assert!(d.is_none(), "AV 2.0 exceeds budget 1.0; should yield");
    }

    #[test]
    fn decide_next_unconditional_when_budget_is_none() {
        let mut rng = StdRng::seed_from_u64(0);
        let cs = vec![att("a", 0.5, 99.0), att("b", 0.5, 100.0)];
        let d = decide_next_speaker(&cs, ChatPolicy::RoundRobin, &mut rng, None).unwrap();
        assert_eq!(d.speaker_slug, "a", "no budget → always picks lowest AV");
        assert!((d.time_advanced - 99.0).abs() < 1e-3);
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
