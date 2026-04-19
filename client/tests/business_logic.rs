#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use client::cli::CliOverrides;
use client::tui::business;
use libllm::config::Config;
use libllm::session::{Message, MessageTree, Role, Session};

// ---------------------------------------------------------------------------
// Summarizer and summary-aware context integration tests
// ---------------------------------------------------------------------------

#[test]
fn summarizer_excludes_summary_messages() {
    use libllm::summarize::Summarizer;

    let msgs = [
        Message::new(Role::Summary, "Should not appear".to_owned()),
        Message::new(Role::User, "Should appear".to_owned()),
    ];
    let refs: Vec<&_> = msgs.iter().collect();
    let prompt = Summarizer::format_prompt("Instruction", &refs);
    assert!(!prompt.contains("Should not appear"));
    assert!(prompt.contains("Should appear"));
}

#[test]
fn summary_aware_path_with_context_manager() {
    use libllm::context::ContextManager;

    let ctx = ContextManager::new(8192);
    let msgs = [
        Message::new(Role::User, "old msg 1".to_owned()),
        Message::new(Role::Assistant, "old reply 1".to_owned()),
        Message::new(Role::Summary, "Summary of 2 earlier messages".to_owned()),
        Message::new(Role::User, "new msg".to_owned()),
        Message::new(Role::Assistant, "new reply".to_owned()),
    ];
    let refs: Vec<&_> = msgs.iter().collect();
    let aware = ctx.summary_aware_path(&refs);

    assert_eq!(aware.len(), 3);
    assert_eq!(aware[0].role, Role::Summary);
    assert_eq!(aware[1].content, "new msg");
    assert_eq!(aware[2].content, "new reply");
}

fn no_overrides() -> CliOverrides {
    CliOverrides {
        api_url: None,
        template: None,
        tls_skip_verify: false,
        sampling: common::empty_overrides(),
        system_prompt: None,
        persona: None,
        no_summarize: false,
        auth_type: None,
        auth_basic_username: None,
        auth_basic_password: None,
        auth_bearer_token: None,
        auth_header_name: None,
        auth_header_value: None,
        auth_query_name: None,
        auth_query_value: None,
    }
}

fn empty_session() -> Session {
    Session {
        tree: MessageTree::new(),
        model: None,
        template: None,
        system_prompt: None,
        character: None,
        worldbooks: vec![],
        persona: None,
    }
}

// ---------------------------------------------------------------------------
// Tabbed Config Sections
// ---------------------------------------------------------------------------

#[test]
fn load_tabbed_config_sections_defaults() {
    let cfg = Config::default();
    let overrides = no_overrides();
    let sections = business::load_tabbed_config_sections(&cfg, &overrides);
    assert_eq!(sections.len(), 4, "expected 4 tabs");
    assert_eq!(sections[0].len(), 6, "General tab");
    assert_eq!(sections[1].len(), 7, "Sampling tab");
    assert_eq!(sections[2].len(), 6, "Backup tab");
    assert_eq!(sections[3].len(), 5, "Summarization tab");
    assert_eq!(sections[0][0], "http://localhost:5001/v1");
    assert_eq!(sections[1][0], "0.8"); // default temperature
    assert_eq!(sections[2][0], "true"); // backup enabled
    assert_eq!(sections[3][0], "true"); // summarization enabled
}

#[test]
fn load_tabbed_config_sections_override_precedence() {
    let cfg = Config {
        api_url: Some("http://stored.example/v1".to_owned()),
        ..Config::default()
    };
    let overrides = CliOverrides {
        api_url: Some("http://cli.example/v1".to_owned()),
        ..no_overrides()
    };
    let sections = business::load_tabbed_config_sections(&cfg, &overrides);
    assert_eq!(sections[0][0], "http://cli.example/v1");
}

#[test]
fn config_locked_by_section_tracks_sampling_overrides() {
    let overrides = CliOverrides {
        sampling: libllm::sampling::SamplingOverrides {
            temperature: Some(0.5),
            ..Default::default()
        },
        ..no_overrides()
    };
    let locked = business::config_locked_fields_by_section(&overrides);
    assert_eq!(locked.len(), 4);
    assert!(locked[1].contains(&0));
    assert!(locked[0].is_empty());
}

#[test]
fn apply_tabbed_config_fields_preserves_locked() {
    let dir = tempfile::tempdir().unwrap();
    libllm::config::set_data_dir(dir.path().to_path_buf()).ok();
    let existing = Config {
        api_url: Some("http://preserve.me/v1".to_owned()),
        ..Config::default()
    };
    let overrides = CliOverrides {
        api_url: Some("http://cli.example/v1".to_owned()),
        ..no_overrides()
    };
    let sections = vec![
        vec![
            "http://should-not-save.example/v1".to_owned(),
            "None".to_owned(),
            "Default".to_owned(),
            "Mistral V3-Tekken".to_owned(),
            "OFF".to_owned(),
            "false".to_owned(),
        ],
        vec![
            "1".to_owned(),
            "40".to_owned(),
            "0.9".to_owned(),
            "0.1".to_owned(),
            "64".to_owned(),
            "1.1".to_owned(),
            "512".to_owned(),
        ],
        vec![
            "true".to_owned(),
            "7".to_owned(),
            "30".to_owned(),
            "90".to_owned(),
            "50".to_owned(),
            "10".to_owned(),
        ],
        vec![
            "true".to_owned(),
            "".to_owned(),
            "8192".to_owned(),
            "5".to_owned(),
            "Summarize.".to_owned(),
        ],
    ];
    business::apply_tabbed_config_fields(&sections, existing, &overrides).unwrap();
    let saved = libllm::config::load();
    assert_eq!(saved.api_url.as_deref(), Some("http://preserve.me/v1"));
}

// ---------------------------------------------------------------------------
// Sidebar Helpers (moved from tui_business.rs)
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
// build_effective_system_prompt
// ---------------------------------------------------------------------------

#[test]
fn build_effective_system_prompt_no_character() {
    let session = empty_session();
    // Without a db, there is no builtin prompt to load, so the result is None
    // when there is no session-level system_prompt and no persona.
    let result = business::build_effective_system_prompt(&session, None);
    assert!(result.is_none());
}

#[test]
fn build_effective_system_prompt_with_custom_prompt() {
    let session = Session {
        system_prompt: Some("Custom prompt text".to_owned()),
        ..empty_session()
    };
    let result = business::build_effective_system_prompt(&session, None);
    assert!(result.is_some());
    let text = result.unwrap();
    assert!(text.contains("Custom prompt text"));
}

#[test]
fn build_effective_system_prompt_custom_prompt_preserved_without_character() {
    let session = Session {
        system_prompt: Some("You are a helpful assistant.".to_owned()),
        character: None,
        ..empty_session()
    };
    let result = business::build_effective_system_prompt(&session, None);
    assert_eq!(result.as_deref(), Some("You are a helpful assistant."));
}

// ---------------------------------------------------------------------------
// inject_loaded_worldbook_entries
// ---------------------------------------------------------------------------

#[test]
fn inject_worldbook_entries_no_character_unchanged() {
    let session = empty_session();
    let messages = [
        Message::new(Role::User, "Hello world".to_owned()),
        Message::new(Role::Assistant, "Hi there".to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();

    let result = business::inject_loaded_worldbook_entries(&session, &refs, "User", &[]);
    assert_eq!(result.len(), messages.len());
    for (original, returned) in messages.iter().zip(result.iter()) {
        assert_eq!(original.content, returned.content);
        assert_eq!(format!("{}", original.role), format!("{}", returned.role));
    }
}

#[test]
fn inject_worldbook_entries_empty_worldbooks_unchanged() {
    let session = Session {
        character: Some("TestChar".to_owned()),
        ..empty_session()
    };
    let messages = [
        Message::new(Role::User, "trigger_keyword".to_owned()),
        Message::new(Role::Assistant, "response".to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();

    // No worldbooks, so messages pass through unchanged regardless of character.
    let result = business::inject_loaded_worldbook_entries(&session, &refs, "User", &[]);
    assert_eq!(result.len(), messages.len());
}
