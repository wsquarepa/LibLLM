mod common;

use libllm::cli::CliOverrides;
use libllm::commands::{self, COMMANDS};
use libllm::config::Config;
use libllm::sampling::SamplingOverrides;
use libllm::tui::business;

fn no_overrides() -> CliOverrides {
    CliOverrides {
        api_url: None,
        template: None,
        tls_skip_verify: false,
        sampling: common::empty_overrides(),
        system_prompt: None,
        persona: None,
    }
}

// ---------------------------------------------------------------------------
// 1. Template Variable Substitution
// ---------------------------------------------------------------------------

#[test]
fn apply_template_vars_basic_char() {
    let result = business::apply_template_vars("Hello {{char}}", "Alice", "Bob");
    assert_eq!(result, "Hello Alice");
}

#[test]
fn apply_template_vars_both_variables() {
    let result = business::apply_template_vars("{{user}} talks to {{char}}", "Alice", "Bob");
    assert_eq!(result, "Bob talks to Alice");
}

#[test]
fn apply_template_vars_multiple_occurrences() {
    let result = business::apply_template_vars("{{char}} meets {{char}}", "Alice", "Bob");
    assert_eq!(result, "Alice meets Alice");
}

#[test]
fn apply_template_vars_no_variables() {
    let result = business::apply_template_vars("plain text", "Alice", "Bob");
    assert_eq!(result, "plain text");
}

#[test]
fn apply_template_vars_empty_names() {
    let result = business::apply_template_vars("Hi {{char}} and {{user}}", "", "");
    assert_eq!(result, "Hi  and ");
}

#[test]
fn apply_template_vars_nested_braces() {
    let result = business::apply_template_vars("{{{char}}}", "Alice", "Bob");
    // The parser finds `{{` at position 0, but `{{{char}}}` does not start
    // with `{{char}}` (it starts with `{{{`), so the outer brace prevents
    // the substitution from triggering.
    assert_eq!(result, "{{{char}}}");
}

// ---------------------------------------------------------------------------
// 2. Non-empty Helper
// ---------------------------------------------------------------------------

#[test]
fn non_empty_with_content() {
    assert_eq!(business::non_empty("hello"), Some("hello".to_owned()));
}

#[test]
fn non_empty_empty_string() {
    assert_eq!(business::non_empty(""), None);
}

#[test]
fn non_empty_whitespace_only() {
    assert_eq!(business::non_empty("   "), None);
    assert_eq!(business::non_empty("\t\n"), None);
}

#[test]
fn non_empty_with_surrounding_whitespace() {
    let result = business::non_empty("  hello  ");
    assert_eq!(result, Some("  hello  ".to_owned()));
}

// ---------------------------------------------------------------------------
// 3. Enabled Worldbook Names
// ---------------------------------------------------------------------------

#[test]
fn enabled_worldbook_names_session_only() {
    let mut session = common::linear_session(vec![common::user_msg("hi")]);
    session.worldbooks = vec!["lore_a".to_owned(), "lore_b".to_owned()];
    let cfg = Config::default();

    let names = business::enabled_worldbook_names(&session, &cfg);
    assert_eq!(names, vec!["lore_a", "lore_b"]);
}

#[test]
fn enabled_worldbook_names_config_only() {
    let session = common::linear_session(vec![common::user_msg("hi")]);
    let cfg = Config {
        worldbooks: vec!["cfg_lore".to_owned()],
        ..Config::default()
    };

    let names = business::enabled_worldbook_names(&session, &cfg);
    assert_eq!(names, vec!["cfg_lore"]);
}

#[test]
fn enabled_worldbook_names_merged_dedup() {
    let mut session = common::linear_session(vec![common::user_msg("hi")]);
    session.worldbooks = vec!["shared".to_owned(), "session_only".to_owned()];
    let cfg = Config {
        worldbooks: vec!["shared".to_owned(), "cfg_only".to_owned()],
        ..Config::default()
    };

    let names = business::enabled_worldbook_names(&session, &cfg);
    assert!(names.contains(&"shared".to_owned()));
    assert!(names.contains(&"cfg_only".to_owned()));
    assert!(names.contains(&"session_only".to_owned()));
    assert_eq!(
        names.iter().filter(|n| *n == "shared").count(),
        1,
        "shared should appear exactly once"
    );
}

#[test]
fn enabled_worldbook_names_both_empty() {
    let session = common::linear_session(vec![common::user_msg("hi")]);
    let cfg = Config::default();

    let names = business::enabled_worldbook_names(&session, &cfg);
    assert!(names.is_empty());
}

// ---------------------------------------------------------------------------
// 4. Config Locked Fields
// ---------------------------------------------------------------------------

#[test]
fn config_locked_fields_no_overrides() {
    let overrides = no_overrides();
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.is_empty());
}

#[test]
fn config_locked_fields_api_url() {
    let overrides = CliOverrides {
        api_url: Some("http://example.com".to_owned()),
        ..no_overrides()
    };
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.contains(&0));
}

#[test]
fn config_locked_fields_template() {
    let overrides = CliOverrides {
        template: Some("chatml".to_owned()),
        ..no_overrides()
    };
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.contains(&3));
}

#[test]
fn config_locked_fields_sampling_temperature() {
    let overrides = CliOverrides {
        sampling: SamplingOverrides {
            temperature: Some(0.5),
            ..common::empty_overrides()
        },
        ..no_overrides()
    };
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.contains(&6));
}

#[test]
fn config_locked_fields_multiple_overrides() {
    let overrides = CliOverrides {
        api_url: Some("http://example.com".to_owned()),
        template: Some("chatml".to_owned()),
        tls_skip_verify: true,
        sampling: SamplingOverrides {
            temperature: Some(0.5),
            top_k: Some(50),
            max_tokens: Some(1024),
            ..common::empty_overrides()
        },
        ..no_overrides()
    };
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.contains(&0), "api_url index 0");
    assert!(locked.contains(&3), "template index 3");
    assert!(locked.contains(&6), "temperature index 6");
    assert!(locked.contains(&7), "top_k index 7");
    assert!(locked.contains(&12), "max_tokens index 12");
    assert!(locked.contains(&13), "tls_skip_verify index 13");
}

#[test]
fn config_locked_fields_tls_skip_verify() {
    let overrides = CliOverrides {
        tls_skip_verify: true,
        ..no_overrides()
    };
    let locked = business::config_locked_fields(&overrides);
    assert!(locked.contains(&13));
}

// ---------------------------------------------------------------------------
// 5. Config Field Loading
// ---------------------------------------------------------------------------

#[test]
fn load_config_fields_defaults() {
    let cfg = Config::default();
    let overrides = no_overrides();
    let fields = business::load_config_fields(&cfg, &overrides);

    assert_eq!(fields[0], "http://localhost:5001/v1");
    assert_eq!(fields[13], "false");
    assert_eq!(fields[14], "false");
    assert_eq!(fields.len(), 15);
}

#[test]
fn load_config_fields_custom_config() {
    let cfg = Config {
        api_url: Some("http://custom:8080/v1".to_owned()),
        tls_skip_verify: true,
        debug_log: true,
        ..Config::default()
    };
    let overrides = no_overrides();
    let fields = business::load_config_fields(&cfg, &overrides);

    assert_eq!(fields[0], "http://custom:8080/v1");
    assert_eq!(fields[13], "true");
    assert_eq!(fields[14], "true");
}

#[test]
fn load_config_fields_override_precedence() {
    let cfg = Config {
        api_url: Some("http://config-url/v1".to_owned()),
        ..Config::default()
    };
    let overrides = CliOverrides {
        api_url: Some("http://override-url/v1".to_owned()),
        sampling: SamplingOverrides {
            temperature: Some(0.1),
            ..common::empty_overrides()
        },
        ..no_overrides()
    };
    let fields = business::load_config_fields(&cfg, &overrides);

    assert_eq!(fields[0], "http://override-url/v1");
    assert_eq!(fields[6], "0.1");
}

// ---------------------------------------------------------------------------
// 6. Sidebar Helpers
// ---------------------------------------------------------------------------

#[test]
fn new_chat_entry_is_new_chat() {
    let entry = business::new_chat_entry();
    assert!(entry.is_new_chat);
    assert_eq!(entry.display_name, "+ New Chat");
    assert_eq!(entry.sidebar_label, "+ New Chat");
    assert!(entry.message_count.is_none());
    assert!(entry.last_assistant_preview.is_none());
    assert!(entry.sidebar_preview.is_none());
}

// ---------------------------------------------------------------------------
// 7. Command Registry
// ---------------------------------------------------------------------------

#[test]
fn resolve_alias_new_to_clear() {
    assert_eq!(commands::resolve_alias("/new"), "/clear");
}

#[test]
fn resolve_alias_canonical_name_unchanged() {
    assert_eq!(commands::resolve_alias("/quit"), "/quit");
}

#[test]
fn resolve_alias_unknown_passthrough() {
    assert_eq!(commands::resolve_alias("/nonexistent"), "/nonexistent");
}

#[test]
fn matching_commands_prefix() {
    let matches = commands::matching_commands("/b", &[]);
    assert!(
        matches.iter().any(|c| c.name == "/branch"),
        "expected /branch to match /b prefix"
    );
}

#[test]
fn matching_commands_empty_prefix_returns_all() {
    let all = commands::matching_commands("/", &[]);
    assert_eq!(all.len(), COMMANDS.len());
}

#[test]
fn matching_commands_with_hidden() {
    let matches = commands::matching_commands("/", &["/quit"]);
    assert!(
        !matches.iter().any(|c| c.name == "/quit"),
        "/quit should be hidden"
    );
    assert!(
        matches.len() < COMMANDS.len(),
        "hidden commands reduce the count"
    );
}

#[test]
fn all_commands_have_required_fields() {
    for cmd in COMMANDS {
        assert!(
            cmd.name.starts_with('/'),
            "command name must start with /: {}",
            cmd.name
        );
        assert!(
            !cmd.description.is_empty(),
            "command {} must have a description",
            cmd.name
        );
    }
}
