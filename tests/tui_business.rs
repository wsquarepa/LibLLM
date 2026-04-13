// Each test binary only uses a subset of shared helpers; allow unused ones.
#[allow(dead_code)]
mod common;

use libllm::cli::CliOverrides;
use libllm::commands::{self, COMMANDS};
use libllm::config::Config;
use libllm::sampling::SamplingOverrides;
use libllm::tui::business;
use libllm::export;
use libllm::session::{Message, Role};
use libllm::tui::commands::expand_macro;
use libllm::tui::theme;

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
fn matching_commands_shorter_match_first() {
    let matches = commands::matching_commands("/m", &[]);
    assert!(matches.len() >= 2, "expected at least /macro and /persona(via /me)");
    assert_eq!(
        matches[0].name, "/macro",
        "/macro (via /m alias) should rank before /persona (via /me)"
    );
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

// ---------------------------------------------------------------------------
// Macro Expansion
// ---------------------------------------------------------------------------

#[test]
fn macro_expand_all_args() {
    let result = expand_macro("Refactor: {{}}", "fn foo() {}").unwrap();
    assert_eq!(result, "Refactor: fn foo() {}");
}

#[test]
fn macro_expand_single_positional() {
    let result = expand_macro("Hello {{1}}", "world").unwrap();
    assert_eq!(result, "Hello world");
}

#[test]
fn macro_expand_multiple_positional() {
    let result = expand_macro("Compare {{1}} with {{2}}", "apples oranges").unwrap();
    assert_eq!(result, "Compare apples with oranges");
}

#[test]
fn macro_expand_positional_out_of_bounds() {
    let result = expand_macro("A={{1}} B={{2}} C={{3}}", "only two").unwrap();
    assert_eq!(result, "A=only B=two C=");
}

#[test]
fn macro_expand_range_two_dots() {
    let result = expand_macro("Items: {{1..3}}", "a b c d").unwrap();
    assert_eq!(result, "Items: a b c");
}

#[test]
fn macro_expand_range_three_dots() {
    let result = expand_macro("Items: {{1...3}}", "a b c d").unwrap();
    assert_eq!(result, "Items: a b c");
}

#[test]
fn macro_expand_range_out_of_bounds() {
    let result = expand_macro("Items: {{1..5}}", "a b").unwrap();
    assert_eq!(result, "Items: a b");
}

#[test]
fn macro_expand_greedy_two_dots() {
    let result = expand_macro("{{1}} {{2}} rest: {{3..}}", "a b c d e").unwrap();
    assert_eq!(result, "a b rest: c d e");
}

#[test]
fn macro_expand_greedy_three_dots() {
    let result = expand_macro("{{1}} {{2}} rest: {{3...}}", "a b c d e").unwrap();
    assert_eq!(result, "a b rest: c d e");
}

#[test]
fn macro_expand_greedy_out_of_bounds() {
    let result = expand_macro("{{1}} {{2}} rest: {{3..}}", "a b").unwrap();
    assert_eq!(result, "a b rest: ");
}

#[test]
fn macro_expand_mixed_positional_and_greedy() {
    let result = expand_macro("From {{1}} to {{2}}: {{3..}}", "en fr hello world").unwrap();
    assert_eq!(result, "From en to fr: hello world");
}

#[test]
fn macro_expand_no_placeholders() {
    let result = expand_macro("Just a template", "ignored args").unwrap();
    assert_eq!(result, "Just a template");
}

#[test]
fn macro_expand_empty_args() {
    let result = expand_macro("Hello {{}}", "").unwrap();
    assert_eq!(result, "Hello ");
}

#[test]
fn macro_expand_empty_args_positional() {
    let result = expand_macro("A={{1}}", "").unwrap();
    assert_eq!(result, "A=");
}

#[test]
fn macro_overlap_range_and_single_errors() {
    let result = expand_macro("{{1..3}} {{2}}", "a b c");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("overlaps"));
}

#[test]
fn macro_overlap_greedy_and_single_errors() {
    let result = expand_macro("{{3..}} {{5}}", "a b c d e");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("overlaps"));
}

#[test]
fn macro_single_outside_range_ok() {
    let result = expand_macro("{{1..2}} and {{3}}", "a b c").unwrap();
    assert_eq!(result, "a b and c");
}

#[test]
fn macro_zero_index_errors() {
    let result = expand_macro("{{0}}", "a");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("start at 1"));
}

#[test]
fn macro_invalid_placeholder_errors() {
    let result = expand_macro("{{abc}}", "a");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid placeholder"));
}

#[test]
fn macro_range_start_greater_than_end_errors() {
    let result = expand_macro("{{3..1}}", "a b c");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("start > end"));
}

#[test]
fn macro_all_placeholder_preserves_whitespace() {
    let result = expand_macro("Say: {{}}", "  hello   world  ").unwrap();
    assert_eq!(result, "Say:   hello   world  ");
}

#[test]
fn macro_multiple_all_placeholders() {
    let result = expand_macro("{{}} and {{}}", "test").unwrap();
    assert_eq!(result, "test and test");
}

#[test]
fn macro_gap_in_indices_errors() {
    let result = expand_macro("{{1}} {{3}}", "a b c");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Gap at index 2"));
}

#[test]
fn macro_greedy_without_preceding_errors() {
    let result = expand_macro("{{5..}}", "a b c d e");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Gap at index 1"));
}

#[test]
fn macro_greedy_with_all_preceding_ok() {
    let result = expand_macro("{{1}} {{2}} {{3..}}", "a b c d e").unwrap();
    assert_eq!(result, "a b c d e");
}

#[test]
fn macro_escape_backslash_opening_braces() {
    let result = expand_macro("literal \\{{}} here", "args").unwrap();
    assert_eq!(result, "literal {{}} here");
}

#[test]
fn macro_escape_mixed_with_real_placeholder() {
    let result = expand_macro("real={{}} escaped=\\{{}}", "hello").unwrap();
    assert_eq!(result, "real=hello escaped={{}}");
}

#[test]
fn macro_range_covers_all_indices() {
    let result = expand_macro("{{1..3}} then {{4}}", "a b c d").unwrap();
    assert_eq!(result, "a b c then d");
}

#[test]
fn macro_greedy_covers_rest() {
    let result = expand_macro("first={{1}} rest={{2..}}", "a b c d").unwrap();
    assert_eq!(result, "first=a rest=b c d");
}

// ---------------------------------------------------------------------------
// Export Rendering
// ---------------------------------------------------------------------------

fn test_messages() -> Vec<Message> {
    vec![
        Message {
            role: Role::User,
            content: "Hello {{char}}".to_owned(),
            timestamp: "2026-01-15T10:00:00Z".to_owned(),
        },
        Message {
            role: Role::Assistant,
            content: "Hi {{user}}!".to_owned(),
            timestamp: "2026-01-15T10:00:05Z".to_owned(),
        },
    ]
}

#[test]
fn export_markdown_basic() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_markdown(&refs, "Alice", "Bob");
    assert!(result.contains("## Bob\n\nHello Alice"));
    assert!(result.contains("## Alice\n\nHi Bob!"));
}

#[test]
fn export_markdown_system_message() {
    let msgs = vec![Message {
        role: Role::System,
        content: "You are helpful.".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_markdown(&refs, "Char", "User");
    assert!(result.contains("## System\n\nYou are helpful."));
}

#[test]
fn export_html_escapes_content() {
    let msgs = vec![Message {
        role: Role::User,
        content: "<script>alert('xss')</script>".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Char", "User");
    assert!(result.contains("&lt;script&gt;"));
    assert!(!result.contains("<script>alert"));
}

#[test]
fn export_html_has_structure() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Alice", "Bob");
    assert!(result.starts_with("<!DOCTYPE html>"));
    assert!(result.contains("class=\"message user\""));
    assert!(result.contains("class=\"message assistant\""));
}

#[test]
fn export_html_applies_template_vars() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Alice", "Bob");
    assert!(result.contains("Hello Alice"));
    assert!(result.contains("Hi Bob!"));
}

#[test]
fn export_html_formats_bold() {
    let msgs = vec![Message {
        role: Role::User,
        content: "This is **bold** text".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Char", "User");
    assert!(result.contains("<strong>bold</strong>"));
    assert!(!result.contains("**bold**"));
}

#[test]
fn export_html_formats_italic() {
    let msgs = vec![Message {
        role: Role::User,
        content: "This is *italic* text".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Char", "User");
    assert!(result.contains("<em>italic</em>"));
    assert!(!result.contains("*italic*"));
}

#[test]
fn export_html_formats_dialogue() {
    let msgs = vec![Message {
        role: Role::Assistant,
        content: "She said \"hello there\" softly".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Char", "User");
    assert!(result.contains("<q>hello there</q>"));
}

#[test]
fn export_html_formats_mixed_markdown() {
    let msgs = vec![Message {
        role: Role::User,
        content: "**bold** and *italic* and \"dialogue\"".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_html(&refs, "Char", "User");
    assert!(result.contains("<strong>bold</strong>"));
    assert!(result.contains("<em>italic</em>"));
    assert!(result.contains("<q>dialogue</q>"));
}

#[test]
fn export_jsonl_has_header() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_jsonl(&refs, "Alice", "Bob");
    let first_line = result.lines().next().unwrap();
    let header: serde_json::Value = serde_json::from_str(first_line).unwrap();
    assert_eq!(header["user_name"], "Bob");
    assert_eq!(header["character_name"], "Alice");
    assert!(header["create_date"].is_string());
}

#[test]
fn export_jsonl_message_fields() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_jsonl(&refs, "Alice", "Bob");
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 3);

    let user_msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(user_msg["name"], "Bob");
    assert_eq!(user_msg["is_user"], true);
    assert_eq!(user_msg["mes"], "Hello Alice");
    assert_eq!(user_msg["send_date"], "2026-01-15T10:00:00Z");

    let asst_msg: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(asst_msg["name"], "Alice");
    assert_eq!(asst_msg["is_user"], false);
    assert_eq!(asst_msg["mes"], "Hi Bob!");
}

#[test]
fn export_jsonl_system_message() {
    let msgs = vec![Message {
        role: Role::System,
        content: "System prompt".to_owned(),
        timestamp: "2026-01-15T10:00:00Z".to_owned(),
    }];
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_jsonl(&refs, "Char", "User");
    let lines: Vec<&str> = result.lines().collect();
    let sys_msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(sys_msg["name"], "System");
    assert_eq!(sys_msg["is_user"], false);
    assert_eq!(sys_msg["is_system"], true);
}

#[test]
fn export_markdown_empty() {
    let refs: Vec<&Message> = vec![];
    let result = export::render_markdown(&refs, "Char", "User");
    assert!(result.is_empty());
}

#[test]
fn export_jsonl_applies_template_vars() {
    let msgs = test_messages();
    let refs: Vec<&Message> = msgs.iter().collect();
    let result = export::render_jsonl(&refs, "Alice", "Bob");
    let lines: Vec<&str> = result.lines().collect();
    let user_msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(user_msg["mes"], "Hello Alice");
}

// ---------------------------------------------------------------------------
// Theme Engine
// ---------------------------------------------------------------------------

#[test]
fn theme_parse_color_named() {
    use ratatui::style::Color;
    assert_eq!(theme::parse_color("red"), Some(Color::Red));
    assert_eq!(theme::parse_color("green"), Some(Color::Green));
    assert_eq!(theme::parse_color("dark_gray"), Some(Color::DarkGray));
    assert_eq!(theme::parse_color("darkgray"), Some(Color::DarkGray));
    assert_eq!(theme::parse_color("light_blue"), Some(Color::LightBlue));
    assert_eq!(theme::parse_color("lightblue"), Some(Color::LightBlue));
    assert_eq!(theme::parse_color("white"), Some(Color::White));
    assert_eq!(theme::parse_color("black"), Some(Color::Black));
}

#[test]
fn theme_parse_color_hex() {
    use ratatui::style::Color;
    assert_eq!(theme::parse_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
    assert_eq!(theme::parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
    assert_eq!(theme::parse_color("#1a2b3c"), Some(Color::Rgb(26, 43, 60)));
}

#[test]
fn theme_parse_color_indexed() {
    use ratatui::style::Color;
    assert_eq!(theme::parse_color("indexed(236)"), Some(Color::Indexed(236)));
    assert_eq!(theme::parse_color("indexed(0)"), Some(Color::Indexed(0)));
}

#[test]
fn theme_parse_color_invalid() {
    assert_eq!(theme::parse_color("notacolor"), None);
    assert_eq!(theme::parse_color("#xyz"), None);
    assert_eq!(theme::parse_color("#12345"), None);
    assert_eq!(theme::parse_color("indexed(abc)"), None);
    assert_eq!(theme::parse_color(""), None);
}

#[test]
fn theme_parse_color_case_insensitive() {
    use ratatui::style::Color;
    assert_eq!(theme::parse_color("RED"), Some(Color::Red));
    assert_eq!(theme::parse_color("Dark_Gray"), Some(Color::DarkGray));
    assert_eq!(theme::parse_color("LightBlue"), Some(Color::LightBlue));
}

#[test]
fn theme_parse_color_with_whitespace() {
    use ratatui::style::Color;
    assert_eq!(theme::parse_color("  red  "), Some(Color::Red));
    assert_eq!(theme::parse_color(" #ff0000 "), Some(Color::Rgb(255, 0, 0)));
}

#[test]
fn theme_from_name_dark() {
    assert!(theme::Theme::from_name("dark").is_some());
}

#[test]
fn theme_from_name_light() {
    assert!(theme::Theme::from_name("light").is_some());
}

#[test]
fn theme_from_name_unknown() {
    assert!(theme::Theme::from_name("solarized").is_none());
    assert!(theme::Theme::from_name("").is_none());
}

#[test]
fn theme_resolve_default() {
    let config = Config::default();
    let t = theme::resolve_theme(&config);
    assert_eq!(t.user_message, ratatui::style::Color::Green);
}

#[test]
fn theme_resolve_light() {
    let mut config = Config::default();
    config.theme = Some("light".to_owned());
    let t = theme::resolve_theme(&config);
    assert_eq!(t.user_message, ratatui::style::Color::Blue);
}

#[test]
fn theme_resolve_with_overrides() {
    use libllm::config::ThemeColorOverrides;
    let mut config = Config::default();
    config.theme_colors = Some(ThemeColorOverrides {
        user_message: Some("red".to_owned()),
        ..Default::default()
    });
    let t = theme::resolve_theme(&config);
    assert_eq!(t.user_message, ratatui::style::Color::Red);
}

#[test]
fn theme_resolve_invalid_override_ignored() {
    use libllm::config::ThemeColorOverrides;
    let mut config = Config::default();
    config.theme_colors = Some(ThemeColorOverrides {
        user_message: Some("notacolor".to_owned()),
        ..Default::default()
    });
    let t = theme::resolve_theme(&config);
    assert_eq!(t.user_message, ratatui::style::Color::Green);
}

#[test]
fn theme_available_themes_not_empty() {
    let themes = theme::Theme::available_themes();
    assert!(themes.contains(&"dark"));
    assert!(themes.contains(&"light"));
}
