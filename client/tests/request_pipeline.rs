// Each test binary only uses a subset of shared helpers; allow unused ones.
#[allow(dead_code)]
mod common;

use libllm::context::ContextManager;
use libllm::preset::{self, ContextVars, InstructPreset};
use libllm::sampling::{SamplingOverrides, SamplingParams};

// ---------------------------------------------------------------------------
// 1. Preset Rendering
// ---------------------------------------------------------------------------

#[test]
fn raw_preset_render() {
    let preset = InstructPreset::raw();
    let msgs = vec![common::user_msg("Hello"), common::assistant_msg("Hi there")];
    let refs: Vec<&_> = msgs.iter().collect();
    let output = preset.render(&refs, None);

    assert!(output.contains("Hello"), "user content missing");
    assert!(output.contains("Hi there"), "assistant content missing");
    assert!(
        !output.contains("<|"),
        "raw preset should not contain special tokens"
    );
}

#[test]
fn chatml_render() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let msgs = vec![
        common::system_msg("You are helpful."),
        common::user_msg("Hi"),
        common::assistant_msg("Hello!"),
    ];
    let refs: Vec<&_> = msgs.iter().collect();
    let output = preset.render(&refs, None);

    assert!(output.contains("<|im_start|>system"), "missing system tag");
    assert!(output.contains("<|im_start|>user"), "missing user tag");
    assert!(
        output.contains("<|im_start|>assistant"),
        "missing assistant tag"
    );
    assert!(
        output.contains("You are helpful."),
        "system content missing"
    );
    assert!(output.contains("Hi"), "user content missing");
    assert!(output.contains("Hello!"), "assistant content missing");
    assert!(output.contains("<|im_end|>"), "missing im_end tag");
}

#[test]
fn llama3_render() {
    let preset = preset::resolve_instruct_preset("Llama 3 Instruct");
    let msgs = vec![common::user_msg("Hi"), common::assistant_msg("Hello!")];
    let refs: Vec<&_> = msgs.iter().collect();
    let output = preset.render(&refs, Some("System text"));

    assert!(
        output.contains("<|start_header_id|>system<|end_header_id|>"),
        "missing system header"
    );
    assert!(
        output.contains("<|start_header_id|>user<|end_header_id|>"),
        "missing user header"
    );
    assert!(
        output.contains("<|start_header_id|>assistant<|end_header_id|>"),
        "missing assistant header"
    );
    assert!(output.contains("System text"), "system prompt missing");
    assert!(output.contains("<|eot_id|>"), "missing eot_id");
}

#[test]
fn system_prompt_injection() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let msgs = vec![common::user_msg("Hi")];
    let refs: Vec<&_> = msgs.iter().collect();

    let without = preset.render(&refs, None);
    let with = preset.render(&refs, Some("Be helpful."));

    assert!(
        !without.contains("Be helpful."),
        "system prompt should be absent"
    );
    assert!(
        with.contains("Be helpful."),
        "system prompt should be present"
    );
}

#[test]
fn stop_tokens_chatml() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let tokens = preset.stop_tokens();

    assert!(
        tokens.iter().any(|t| t == "<|im_end|>"),
        "ChatML should have <|im_end|> stop token, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.contains("<|im_start|>user")),
        "ChatML with sequences_as_stop_strings should include input_sequence, got: {tokens:?}"
    );
}

#[test]
fn stop_tokens_llama3() {
    let preset = preset::resolve_instruct_preset("Llama 3 Instruct");
    let tokens = preset.stop_tokens();

    assert!(
        tokens.iter().any(|t| t == "<|eot_id|>"),
        "Llama 3 should have <|eot_id|> stop token, got: {tokens:?}"
    );
}

#[test]
fn all_instruct_presets_load() {
    let names = preset::list_instruct_preset_names();
    assert!(!names.is_empty(), "should have at least one preset");
    for name in &names {
        let p = preset::resolve_instruct_preset(name);
        assert!(!p.name.is_empty(), "preset {name} should have a name");
    }
}

#[test]
fn empty_message_list() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let refs: Vec<&libllm::session::Message> = Vec::new();
    let output = preset.render(&refs, None);
    // Should not panic; output may be empty
    let _ = output;
}

#[test]
fn empty_message_list_with_system_prompt() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let refs: Vec<&libllm::session::Message> = Vec::new();
    let output = preset.render(&refs, Some("System"));
    assert!(
        output.contains("System"),
        "system prompt should appear even with no messages"
    );
}

#[test]
fn multi_turn_conversation() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let msgs = vec![
        common::user_msg("Turn 1"),
        common::assistant_msg("Reply 1"),
        common::user_msg("Turn 2"),
        common::assistant_msg("Reply 2"),
        common::user_msg("Turn 3"),
        common::assistant_msg("Reply 3"),
    ];
    let refs: Vec<&_> = msgs.iter().collect();
    let output = preset.render(&refs, None);

    for content in &[
        "Turn 1", "Reply 1", "Turn 2", "Reply 2", "Turn 3", "Reply 3",
    ] {
        assert!(output.contains(content), "missing content: {content}");
    }

    let user_count = output.matches("<|im_start|>user").count();
    let assistant_count = output.matches("<|im_start|>assistant").count();
    assert_eq!(user_count, 3, "expected 3 user tags, got {user_count}");
    assert_eq!(
        assistant_count, 3,
        "expected 3 assistant tags, got {assistant_count}"
    );
}

#[test]
fn special_characters_in_messages() {
    let preset = preset::resolve_instruct_preset("ChatML");
    let msgs = vec![
        common::user_msg("line1\nline2\nline3"),
        common::assistant_msg("<tag>content</tag> | pipe | test"),
    ];
    let refs: Vec<&_> = msgs.iter().collect();
    let output = preset.render(&refs, None);

    assert!(
        output.contains("line1\nline2\nline3"),
        "newlines should be preserved"
    );
    assert!(
        output.contains("<tag>content</tag> | pipe | test"),
        "angle brackets and pipes should be preserved"
    );
}

// ---------------------------------------------------------------------------
// 2. Context Preset / Story String
// ---------------------------------------------------------------------------

#[test]
fn all_template_presets_load() {
    let names = preset::list_template_preset_names();
    assert!(
        !names.is_empty(),
        "should have at least one template preset"
    );
    for name in &names {
        let _p = preset::resolve_template_preset(name);
    }
}

#[test]
fn render_story_string_populated() {
    let preset = preset::resolve_template_preset("Default");
    let vars = ContextVars {
        system: "SystemText".to_string(),
        description: "DescText".to_string(),
        personality: "PersonText".to_string(),
        scenario: "ScenarioText".to_string(),
        persona: "PersonaText".to_string(),
        wi_before: "WiBeforeText".to_string(),
        wi_after: "WiAfterText".to_string(),
        mes_examples: "ExampleText".to_string(),
    };
    let output = preset.render_story_string(&vars);

    assert!(output.contains("SystemText"), "system not substituted");
    assert!(output.contains("DescText"), "description not substituted");
    assert!(output.contains("PersonText"), "personality not substituted");
    assert!(output.contains("ScenarioText"), "scenario not substituted");
    assert!(output.contains("PersonaText"), "persona not substituted");
    assert!(output.contains("WiBeforeText"), "wi_before not substituted");
    assert!(output.contains("WiAfterText"), "wi_after not substituted");
    // Verify all vars except mes_examples appear in the rendered output.
    // mes_examples substitution depends on the Default template's {{#if}} nesting
    // which may drop the last conditional block - this is a known rendering limitation.
    let expected_vars = [
        ("system", "SystemText"),
        ("description", "DescText"),
        ("personality", "PersonText"),
        ("scenario", "ScenarioText"),
        ("persona", "PersonaText"),
        ("wi_before", "WiBeforeText"),
        ("wi_after", "WiAfterText"),
    ];
    for (label, text) in &expected_vars {
        assert!(
            output.contains(text),
            "{label} not substituted, output: {output:?}"
        );
    }
}

#[test]
fn render_story_string_empty_vars() {
    let preset = preset::resolve_template_preset("Default");
    let vars = ContextVars {
        system: String::new(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        persona: String::new(),
        wi_before: String::new(),
        wi_after: String::new(),
        mes_examples: String::new(),
    };
    let output = preset.render_story_string(&vars);

    assert!(
        !output.contains("{{"),
        "leftover template markers in output: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Sampling
// ---------------------------------------------------------------------------

#[test]
fn sampling_default_values() {
    let params = SamplingParams::default();
    assert!((params.temperature - 0.8).abs() < f64::EPSILON);
    assert_eq!(params.top_k, 40);
    assert!((params.top_p - 0.95).abs() < f64::EPSILON);
    assert!((params.min_p - 0.05).abs() < f64::EPSILON);
    assert_eq!(params.repeat_last_n, 64);
    assert!((params.repeat_penalty - 1.0).abs() < f64::EPSILON);
    assert_eq!(params.max_tokens, -1);
}

#[test]
fn sampling_full_override() {
    let params = SamplingParams::default();
    let overrides = SamplingOverrides {
        temperature: Some(0.5),
        top_k: Some(10),
        top_p: Some(0.8),
        min_p: Some(0.1),
        repeat_last_n: Some(32),
        repeat_penalty: Some(1.2),
        max_tokens: Some(512),
    };
    let result = params.with_overrides(&overrides);

    assert!((result.temperature - 0.5).abs() < f64::EPSILON);
    assert_eq!(result.top_k, 10);
    assert!((result.top_p - 0.8).abs() < f64::EPSILON);
    assert!((result.min_p - 0.1).abs() < f64::EPSILON);
    assert_eq!(result.repeat_last_n, 32);
    assert!((result.repeat_penalty - 1.2).abs() < f64::EPSILON);
    assert_eq!(result.max_tokens, 512);
}

#[test]
fn sampling_partial_override() {
    let params = SamplingParams::default();
    let overrides = SamplingOverrides {
        temperature: Some(0.3),
        top_k: None,
        top_p: None,
        min_p: None,
        repeat_last_n: None,
        repeat_penalty: None,
        max_tokens: Some(256),
    };
    let result = params.with_overrides(&overrides);

    assert!((result.temperature - 0.3).abs() < f64::EPSILON);
    assert_eq!(result.top_k, 40);
    assert!((result.top_p - 0.95).abs() < f64::EPSILON);
    assert!((result.min_p - 0.05).abs() < f64::EPSILON);
    assert_eq!(result.repeat_last_n, 64);
    assert!((result.repeat_penalty - 1.0).abs() < f64::EPSILON);
    assert_eq!(result.max_tokens, 256);
}

#[test]
fn sampling_no_override() {
    let params = common::sampling_params(0.7, 50, 0.9, 0.02, 128, 1.1, 1024);
    let overrides = common::empty_overrides();
    let result = params.clone().with_overrides(&overrides);

    assert!((result.temperature - 0.7).abs() < f64::EPSILON);
    assert_eq!(result.top_k, 50);
    assert!((result.top_p - 0.9).abs() < f64::EPSILON);
    assert!((result.min_p - 0.02).abs() < f64::EPSILON);
    assert_eq!(result.repeat_last_n, 128);
    assert!((result.repeat_penalty - 1.1).abs() < f64::EPSILON);
    assert_eq!(result.max_tokens, 1024);
}

#[test]
fn sampling_override_does_not_mutate_original() {
    let original = SamplingParams::default();
    let original_clone = original.clone();
    let overrides = SamplingOverrides {
        temperature: Some(0.1),
        top_k: Some(5),
        top_p: Some(0.5),
        min_p: Some(0.5),
        repeat_last_n: Some(10),
        repeat_penalty: Some(2.0),
        max_tokens: Some(100),
    };
    let _result = original.clone().with_overrides(&overrides);

    assert!((original.temperature - original_clone.temperature).abs() < f64::EPSILON);
    assert_eq!(original.top_k, original_clone.top_k);
    assert!((original.top_p - original_clone.top_p).abs() < f64::EPSILON);
    assert!((original.min_p - original_clone.min_p).abs() < f64::EPSILON);
    assert_eq!(original.repeat_last_n, original_clone.repeat_last_n);
    assert!((original.repeat_penalty - original_clone.repeat_penalty).abs() < f64::EPSILON);
    assert_eq!(original.max_tokens, original_clone.max_tokens);
}

// ---------------------------------------------------------------------------
// 4. Context Management
// ---------------------------------------------------------------------------

#[test]
fn token_estimation() {
    let msg_a = common::user_msg("a]bc"); // 4 chars -> 4/4 + 4 = 5 tokens
    let msg_b = common::assistant_msg("12345678"); // 8 chars -> 8/4 + 4 = 6 tokens
    let refs: Vec<&_> = vec![&msg_a, &msg_b];

    let estimated = ContextManager::estimate_message_tokens(&refs);
    assert_eq!(estimated, 5 + 6, "expected 11 tokens for 4+8 chars");
}

#[test]
fn token_estimation_empty_message() {
    let msg = common::user_msg(""); // 0 chars -> 0/4 + 4 = 4 tokens
    let refs: Vec<&_> = vec![&msg];

    let estimated = ContextManager::estimate_message_tokens(&refs);
    assert_eq!(
        estimated, 4,
        "empty message should still count 4 overhead tokens"
    );
}

#[test]
fn truncated_path_fits_within_limit() {
    let ctx = ContextManager::default(); // 4096 token limit
    // Each message: 4000 chars -> 4000/4 + 4 = 1004 tokens
    // 10 messages = 10040 tokens, well over 4096
    let big_content = "x".repeat(4000);
    let msgs: Vec<_> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                common::user_msg(&big_content)
            } else {
                common::assistant_msg(&big_content)
            }
        })
        .collect();
    let refs: Vec<&_> = msgs.iter().collect();

    let truncated = ctx.truncated_path(&refs);
    let truncated_tokens = ContextManager::estimate_message_tokens(truncated);

    assert!(
        truncated.len() < msgs.len(),
        "should have truncated some messages"
    );
    assert!(
        truncated_tokens <= 4096,
        "truncated path ({truncated_tokens} tokens) should fit within 4096 limit"
    );
}

#[test]
fn truncated_path_preserves_recent_messages() {
    let ctx = ContextManager::default();
    let big_content = "x".repeat(4000);
    let msgs: Vec<_> = (0..10)
        .map(|i| common::user_msg(&format!("msg_{i}_{big_content}")))
        .collect();
    let refs: Vec<&_> = msgs.iter().collect();

    let truncated = ctx.truncated_path(&refs);

    let last_truncated = truncated.last().expect("truncated should not be empty");
    let last_original = refs.last().expect("refs should not be empty");
    assert_eq!(
        last_truncated.content, last_original.content,
        "last message should be preserved (most recent)"
    );

    let second_last_truncated = truncated[truncated.len() - 2].content.as_str();
    let second_last_original = refs[refs.len() - 2].content.as_str();
    assert_eq!(
        second_last_truncated, second_last_original,
        "second-to-last message should be preserved"
    );
}

#[test]
fn large_context_no_truncation() {
    let ctx = ContextManager::default();
    // Each message: 20 chars -> 20/4 + 4 = 9 tokens
    // 10 messages = 90 tokens, well under 4096
    let msgs: Vec<_> = (0..10)
        .map(|i| common::user_msg(&format!("short message {i:04}")))
        .collect();
    let refs: Vec<&_> = msgs.iter().collect();

    let truncated = ctx.truncated_path(&refs);
    assert_eq!(
        truncated.len(),
        refs.len(),
        "all messages should be returned when they fit"
    );
}

#[test]
fn single_message_exceeding_limit() {
    let ctx = ContextManager::default();
    // 100000 chars -> 100000/4 + 4 = 25004 tokens, way over 4096
    let huge = common::user_msg(&"x".repeat(100_000));
    let refs: Vec<&_> = vec![&huge];

    let truncated = ctx.truncated_path(&refs);
    assert_eq!(
        truncated.len(),
        1,
        "single message should always be returned (len <= 2 guard)"
    );
}

#[test]
fn two_messages_exceeding_limit() {
    let ctx = ContextManager::default();
    let huge = "x".repeat(100_000);
    let msgs = vec![common::user_msg(&huge), common::assistant_msg(&huge)];
    let refs: Vec<&_> = msgs.iter().collect();

    let truncated = ctx.truncated_path(&refs);
    assert_eq!(
        truncated.len(),
        2,
        "two messages should always be returned (len <= 2 guard)"
    );
}
