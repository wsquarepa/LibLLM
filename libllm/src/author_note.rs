use crate::session::{Message, Role};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorNote {
    pub text: String,
    pub depth: u32,
    pub at_top: bool,
}

impl AuthorNote {
    pub fn from_row_parts(text: Option<String>, depth: u32, at_top: bool) -> Option<Self> {
        text.filter(|t| !t.trim().is_empty())
            .map(|text| Self { text, depth, at_top })
    }

    pub fn position(&self, message_count: usize) -> usize {
        if self.at_top {
            return 0;
        }
        let depth = self.depth as usize;
        if depth == 0 || depth >= message_count {
            0
        } else {
            message_count - depth
        }
    }
}

pub fn inject_author_notes(
    messages: &mut Vec<Message>,
    card_note: Option<&AuthorNote>,
    session_note: Option<&AuthorNote>,
) {
    let original_len = messages.len();

    if let Some(session) = session_note {
        let pos = session.position(original_len);
        messages.insert(pos, Message::new(Role::System, session.text.clone()));
    }

    if let Some(card) = card_note {
        let pos = card.position(original_len);
        messages.insert(pos, Message::new(Role::System, card.text.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Message, Role};

    fn user(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_owned(),
            timestamp: String::new(),
            thought_seconds: None,
        }
    }

    fn note(text: &str, depth: u32, at_top: bool) -> AuthorNote {
        AuthorNote { text: text.to_owned(), depth, at_top }
    }

    #[test]
    fn inject_no_notes_is_noop() {
        let mut messages = vec![user("a"), user("b"), user("c")];
        inject_author_notes(&mut messages, None, None);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn inject_session_only_at_depth_2() {
        let mut messages = vec![user("a"), user("b"), user("c"), user("d")];
        let session = note("S", 2, false);
        inject_author_notes(&mut messages, None, Some(&session));
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2].content, "S");
        assert_eq!(messages[2].role, Role::System);
    }

    #[test]
    fn inject_card_only_at_depth_2() {
        let mut messages = vec![user("a"), user("b"), user("c"), user("d")];
        let card = note("C", 2, false);
        inject_author_notes(&mut messages, Some(&card), None);
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2].content, "C");
        assert_eq!(messages[2].role, Role::System);
    }

    #[test]
    fn inject_both_same_depth_session_after_card() {
        let mut messages = vec![user("a"), user("b"), user("c"), user("d")];
        let card = note("C", 2, false);
        let session = note("S", 2, false);
        inject_author_notes(&mut messages, Some(&card), Some(&session));
        assert_eq!(messages.len(), 6);
        let card_idx = messages.iter().position(|m| m.content == "C").unwrap();
        let session_idx = messages.iter().position(|m| m.content == "S").unwrap();
        assert!(
            session_idx > card_idx,
            "session must end up at a higher index than card; got card={card_idx} session={session_idx}"
        );
    }

    #[test]
    fn inject_both_different_depths_each_at_own_position() {
        let mut messages = vec![user("a"), user("b"), user("c"), user("d"), user("e")];
        let card = note("C", 4, false);
        let session = note("S", 1, false);
        inject_author_notes(&mut messages, Some(&card), Some(&session));
        let card_idx = messages.iter().position(|m| m.content == "C").unwrap();
        let session_idx = messages.iter().position(|m| m.content == "S").unwrap();
        assert_eq!(card_idx, 1, "card depth=4 against len=5 → position 1");
        assert!(
            session_idx > card_idx,
            "session at depth=1 should land further to the end than card"
        );
    }

    #[test]
    fn inject_at_top_lands_at_zero() {
        let mut messages = vec![user("a"), user("b"), user("c")];
        let session = note("S", 99, true);
        inject_author_notes(&mut messages, None, Some(&session));
        assert_eq!(messages[0].content, "S");
    }

    #[test]
    fn inject_into_empty_messages_does_not_panic() {
        let mut messages: Vec<Message> = Vec::new();
        let session = note("S", 4, false);
        inject_author_notes(&mut messages, None, Some(&session));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "S");
    }

    #[test]
    fn from_row_parts_none_when_text_is_null() {
        assert_eq!(AuthorNote::from_row_parts(None, 4, false), None);
    }

    #[test]
    fn from_row_parts_none_when_text_is_empty() {
        assert_eq!(AuthorNote::from_row_parts(Some(String::new()), 4, false), None);
    }

    #[test]
    fn from_row_parts_none_when_text_is_whitespace() {
        assert_eq!(
            AuthorNote::from_row_parts(Some("   \t\n".to_owned()), 4, false),
            None
        );
    }

    #[test]
    fn from_row_parts_some_when_text_is_present() {
        let note = AuthorNote::from_row_parts(Some("steer".to_owned()), 6, true);
        assert_eq!(
            note,
            Some(AuthorNote {
                text: "steer".to_owned(),
                depth: 6,
                at_top: true,
            })
        );
    }

    #[test]
    fn position_at_top_is_zero() {
        let note = AuthorNote { text: "x".to_owned(), depth: 99, at_top: true };
        assert_eq!(note.position(10), 0);
    }

    #[test]
    fn position_zero_depth_clamps_to_zero() {
        let note = AuthorNote { text: "x".to_owned(), depth: 0, at_top: false };
        assert_eq!(note.position(10), 0);
    }

    #[test]
    fn position_depth_equals_len_clamps_to_zero() {
        let note = AuthorNote { text: "x".to_owned(), depth: 10, at_top: false };
        assert_eq!(note.position(10), 0);
    }

    #[test]
    fn position_depth_exceeds_len_clamps_to_zero() {
        let note = AuthorNote { text: "x".to_owned(), depth: 99, at_top: false };
        assert_eq!(note.position(10), 0);
    }

    #[test]
    fn position_depth_one_is_len_minus_one() {
        let note = AuthorNote { text: "x".to_owned(), depth: 1, at_top: false };
        assert_eq!(note.position(10), 9);
    }

    #[test]
    fn position_depth_four_is_len_minus_four() {
        let note = AuthorNote { text: "x".to_owned(), depth: 4, at_top: false };
        assert_eq!(note.position(10), 6);
    }

    #[test]
    fn position_empty_messages_clamps_to_zero() {
        let note = AuthorNote { text: "x".to_owned(), depth: 4, at_top: false };
        assert_eq!(note.position(0), 0);
    }

    #[test]
    fn serde_round_trip() {
        let note = AuthorNote { text: "hi".to_owned(), depth: 3, at_top: true };
        let json = serde_json::to_string(&note).unwrap();
        let back: AuthorNote = serde_json::from_str(&json).unwrap();
        assert_eq!(note, back);
    }
}
