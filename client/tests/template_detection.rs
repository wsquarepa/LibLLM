//! Integration tests for the auto-template-detect background event flow.
//! These exercise the libllm-level matching logic against the actual built-in presets;
//! the TUI wiring is tested via subprocess in a separate file.

#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use libllm::preset::matching::{pick_best_match, template_hash, MatchOutcome, BEST_GUESS_THRESHOLD};

const REAL_LLAMA3_TEMPLATE: &str = include_str!("../../libllm/src/preset/matching_fixtures/llama3.jinja");

fn all_builtins() -> Vec<libllm::preset::InstructPreset> {
    libllm::preset::list_instruct_preset_names()
        .into_iter()
        .map(|n| libllm::preset::resolve_instruct_preset(&n))
        .collect()
}

#[test]
fn real_llama3_template_resolves_to_llama3_preset() {
    let outcome = pick_best_match(REAL_LLAMA3_TEMPLATE, &all_builtins());
    match outcome {
        MatchOutcome::Confident { preset, .. } | MatchOutcome::BestGuess { preset, .. } => {
            assert_eq!(preset, "Llama 3 Instruct");
        }
        other => panic!("expected match, got {other:?}"),
    }
}

#[test]
fn template_hash_matches_for_kobold_and_llama_cpp_emissions() {
    // Kobold sometimes appends a NUL byte; llama.cpp may have whitespace drift.
    let raw = REAL_LLAMA3_TEMPLATE;
    let with_nul = format!("{raw}\0");
    let with_extra_space = format!("  {raw}  ");
    assert_eq!(template_hash(raw), template_hash(&with_nul));
    assert_eq!(template_hash(raw), template_hash(&with_extra_space));
}

#[test]
fn unknown_template_falls_below_best_guess_threshold() {
    let outcome = pick_best_match(
        "This is just plain text with no Jinja tags or chat structure.",
        &all_builtins(),
    );
    match outcome {
        MatchOutcome::NoMatch { best_score } => assert!(best_score < BEST_GUESS_THRESHOLD),
        other => panic!("expected NoMatch, got {other:?}"),
    }
}
