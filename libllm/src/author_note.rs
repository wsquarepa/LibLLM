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
            .map(|text| AuthorNote { text, depth, at_top })
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

#[cfg(test)]
mod tests {
    use super::*;

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
