//! Locks down the contract that streaming-prompt assembly will rely on:
//! `inject_author_notes` lands the synthetic system message at the expected
//! position in the assembled message list.

use libllm_core::author_note::{AuthorNote, inject_author_notes};
use libllm_core::session::{Message, Role};

#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

fn user(content: &str) -> Message {
    Message::new(Role::User, content.to_owned())
}

#[test]
fn session_and_card_layered_with_different_depths() {
    let mut messages = vec![user("a"), user("b"), user("c"), user("d"), user("e")];
    let card = AuthorNote {
        text: "CARD".to_owned(),
        depth: 4,
        at_top: false,
    };
    let session = AuthorNote {
        text: "SESSION".to_owned(),
        depth: 1,
        at_top: false,
    };

    inject_author_notes(&mut messages, Some(&card), Some(&session));

    let card_idx = messages.iter().position(|m| m.content == "CARD").unwrap();
    let session_idx = messages
        .iter()
        .position(|m| m.content == "SESSION")
        .unwrap();
    assert!(
        session_idx > card_idx,
        "session must end up at a higher index; got card={card_idx}, session={session_idx}"
    );
    for m in &messages {
        if m.content == "CARD" || m.content == "SESSION" {
            assert_eq!(m.role, Role::System);
        }
    }
}

#[test]
fn at_top_overrides_a_high_depth() {
    let mut messages = vec![user("a"), user("b"), user("c")];
    let session = AuthorNote {
        text: "PIN".to_owned(),
        depth: 99,
        at_top: true,
    };
    inject_author_notes(&mut messages, None, Some(&session));
    assert_eq!(messages[0].content, "PIN");
}

#[test]
fn card_author_note_loaded_when_session_character_is_display_name() {
    use libllm_core::author_note::AuthorNote;
    use libllm_core::character::CharacterCard;
    use libllm_storage::db::Database;

    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).unwrap();

    let card = CharacterCard {
        name: "Alice Example".to_owned(),
        author_note: Some(AuthorNote {
            text: "SENTINEL".to_owned(),
            depth: 2,
            at_top: false,
        }),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
    };
    let slug = libllm_core::character::slugify(&card.name);
    db.insert_character(&slug, &card).unwrap();

    let slug_of_display = libllm_core::character::slugify("Alice Example");
    let loaded = db.load_character(&slug_of_display).unwrap();

    assert_eq!(
        loaded.author_note.as_ref().map(|n| n.text.as_str()),
        Some("SENTINEL"),
        "card author note must be retrievable when looked up by slugified display name"
    );
}

#[test]
fn slugify_is_idempotent_on_slug() {
    assert_eq!(
        libllm_core::character::slugify("alice-example"),
        "alice-example"
    );
    assert_eq!(
        libllm_core::character::slugify("Alice Example"),
        "alice-example"
    );
}
