// Each test binary only uses a subset of shared helpers; allow unused ones.
#[allow(dead_code)]
mod common;

use client::cli::CliOverrides;
use client::tui::business;
use libllm::config::Config;
use libllm::sampling::SamplingOverrides;
use libllm::session::{Message, MessageTree, Role, Session};

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
// Config Locked Fields (moved from tui_business.rs)
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
// Config Field Loading (moved from tui_business.rs)
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
    let messages = vec![
        Message::new(Role::User, "Hello world".to_owned()),
        Message::new(Role::Assistant, "Hi there".to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();

    let result = business::inject_loaded_worldbook_entries(&session, &refs, "User", &[]);
    assert_eq!(result.len(), messages.len());
    for (original, returned) in messages.iter().zip(result.iter()) {
        assert_eq!(original.content, returned.content);
        assert_eq!(
            format!("{}", original.role),
            format!("{}", returned.role)
        );
    }
}

#[test]
fn inject_worldbook_entries_empty_worldbooks_unchanged() {
    let session = Session {
        character: Some("TestChar".to_owned()),
        ..empty_session()
    };
    let messages = vec![
        Message::new(Role::User, "trigger_keyword".to_owned()),
        Message::new(Role::Assistant, "response".to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();

    // No worldbooks, so messages pass through unchanged regardless of character.
    let result = business::inject_loaded_worldbook_entries(&session, &refs, "User", &[]);
    assert_eq!(result.len(), messages.len());
}

// ---------------------------------------------------------------------------
// save_config_from_fields
// ---------------------------------------------------------------------------

#[test]
fn save_config_from_fields_clamps_temperature() {
    let dir = common::temp_dir();
    libllm::config::set_data_dir(dir.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();

    let defaults = libllm::sampling::SamplingParams::default();
    let mut fields = business::load_config_fields(&Config::default(), &no_overrides());
    // Set temperature (index 6) to a value above the max of 2.0.
    fields[6] = "5.0".to_owned();

    business::save_config_from_fields(&fields, &[]).unwrap();
    let saved = libllm::config::load();

    let temperature = saved
        .sampling
        .temperature
        .unwrap_or(defaults.temperature);
    assert!(
        temperature <= 2.0,
        "temperature should be clamped to max 2.0, got {temperature}"
    );
}

#[test]
fn save_config_from_fields_skips_locked_fields() {
    let dir = common::temp_dir();
    libllm::config::set_data_dir(dir.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();

    // Save an initial config with a known api_url.
    let initial = Config {
        api_url: Some("http://original:1234/v1".to_owned()),
        ..Config::default()
    };
    libllm::config::save(&initial).unwrap();

    // Build fields with a different api_url at index 0, but lock index 0.
    let mut fields = business::load_config_fields(&initial, &no_overrides());
    fields[0] = "http://should-not-be-saved/v1".to_owned();

    // Index 0 is locked — the original api_url must survive.
    business::save_config_from_fields(&fields, &[0]).unwrap();
    let saved = libllm::config::load();

    assert_eq!(
        saved.api_url.as_deref(),
        Some("http://original:1234/v1"),
        "locked api_url should not be overwritten"
    );
}
