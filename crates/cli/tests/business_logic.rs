#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use libllm_cli::cli::CliOverrides;
use libllm_core::config::Config;
use libllm_core::group_chat::{CharacterAttachment, ChatMode};
use libllm_core::session::{Message, MessageTree, Role, Session};
use libllm_core::worldinfo::RuntimeWorldBook;
use libllm_tui::business;
use libllm_tui::dialogs::chat_settings::roll_back_provisional_group;

fn no_overrides() -> CliOverrides {
    CliOverrides {
        api_url: None,
        template: None,
        tls_skip_verify: false,
        sampling: common::empty_overrides(),
        system_prompt: None,
        persona: None,
        characters: vec![],
        chat_mode: None,
        scenario: None,
        talkativeness: std::collections::HashMap::new(),
        author_note: None,
        author_note_depth: None,
        author_note_at_top: None,
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
        scenario: None,
        characters: Vec::new(),
        chat_mode: libllm_core::group_chat::ChatMode::default(),
        author_note: None,
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
    assert_eq!(sections.len(), 6, "expected 6 tabs");
    assert_eq!(sections[0].len(), 6, "General tab");
    assert_eq!(sections[1].len(), 7, "Sampling tab");
    assert_eq!(sections[2].len(), 6, "Backup tab");
    assert_eq!(sections[3].len(), 6, "Summarization tab");
    assert_eq!(sections[4].len(), 5, "Files tab");
    assert_eq!(sections[0][0], "http://localhost:5001/v1");
    assert_eq!(sections[1][0], "0.8"); // default temperature
    assert_eq!(sections[2][0], "true"); // backup enabled
    assert_eq!(sections[3][0], "true"); // summarization enabled
    assert_eq!(sections[4][3], "eager"); // default summarize_mode
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
        sampling: libllm_core::sampling::SamplingOverrides {
            temperature: Some(0.5),
            ..Default::default()
        },
        ..no_overrides()
    };
    let locked = business::config_locked_fields_by_section(&overrides);
    assert_eq!(locked.len(), 6);
    assert!(locked[1].contains(&0));
    assert!(locked[0].is_empty());
}

#[test]
fn apply_tabbed_config_fields_preserves_locked() {
    let dir = tempfile::tempdir().unwrap();
    libllm_config::set_data_dir(dir.path().to_path_buf()).ok();
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
            "4".to_owned(),
            "Summarize.".to_owned(),
        ],
        vec![
            "true".to_owned(),
            "524288".to_owned(),
            "4194304".to_owned(),
            "eager".to_owned(),
            "Summarize this file.".to_owned(),
        ],
    ];
    business::apply_tabbed_config_fields(&sections, existing, &overrides).unwrap();
    let saved = libllm_config::load();
    assert_eq!(saved.api_url.as_deref(), Some("http://preserve.me/v1"));
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
fn inject_worldbook_entries_requires_an_attached_character() {
    let worldbooks = [RuntimeWorldBook::from_worldbook(&common::worldbook(
        "lore",
        vec![common::worldbook_entry(
            vec!["dragon"],
            "Dragons hoard gold.",
        )],
    ))];
    let messages = [
        Message::new(Role::User, "Tell me about the dragon".to_owned()),
        Message::new(Role::Assistant, "Which one?".to_owned()),
    ];
    let refs: Vec<&Message> = messages.iter().collect();

    let with_character = Session {
        character: Some("TestChar".to_owned()),
        ..empty_session()
    };
    let injected =
        business::inject_loaded_worldbook_entries(&with_character, &refs, "User", &worldbooks);
    assert!(
        injected.iter().any(|m| m.content == "Dragons hoard gold."),
        "a keyword hit with a character attached must inject the entry"
    );

    let unchanged =
        business::inject_loaded_worldbook_entries(&empty_session(), &refs, "User", &worldbooks);
    let contents: Vec<&str> = unchanged.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["Tell me about the dragon", "Which one?"]);
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

// ---------------------------------------------------------------------------
// Group chat creation rollback
// ---------------------------------------------------------------------------

#[test]
fn rollback_provisional_group_clears_session_state() {
    let mut session = Session {
        characters: vec![
            CharacterAttachment {
                slug: "alice".into(),
                talkativeness: 0.5,
                action_points: 0.0,
                spoke_this_round: false,
            },
            CharacterAttachment {
                slug: "bob".into(),
                talkativeness: 0.5,
                action_points: 0.0,
                spoke_this_round: false,
            },
        ],
        chat_mode: ChatMode::WeightedRandom,
        scenario: Some("partial".into()),
        ..Session::default()
    };
    session
        .tree
        .push(None, Message::new(Role::User, "hello".to_owned()));
    assert!(
        session.tree.head().is_some(),
        "precondition: tree must be non-empty before rollback"
    );
    roll_back_provisional_group(&mut session);
    assert!(
        session.characters.is_empty(),
        "rollback must clear character attachments"
    );
    assert_eq!(
        session.chat_mode,
        ChatMode::default(),
        "rollback must reset chat mode to default"
    );
    assert_eq!(session.scenario, None, "rollback must clear the scenario");
    assert!(
        session.tree.head().is_none(),
        "rollback must clear the message tree"
    );
}
