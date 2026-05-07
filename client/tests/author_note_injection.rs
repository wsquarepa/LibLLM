//! Locks down the contract that streaming-prompt assembly will rely on:
//! `inject_author_notes` lands the synthetic system message at the expected
//! position in the assembled message list.

use libllm::author_note::{AuthorNote, inject_author_notes};
use libllm::session::{Message, Role};

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
    let session_idx = messages.iter().position(|m| m.content == "SESSION").unwrap();
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
