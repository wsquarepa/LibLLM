//! Lexer-driven helpers for splitting and validating SQL input.
//!
//! `is_statement_complete` returns true once the first top-level `;` is
//! observed outside of any string literal, line comment, block comment, or
//! parenthesised expression. The shell uses it to decide when to stop
//! accumulating continuation lines.
//!
//! `is_single_statement` returns true when the buffer contains exactly one
//! top-level `;` and everything after it is whitespace or comments. The one-shot
//! `db sql` runner uses this to reject multi-statement input.

#[derive(Clone, Copy, PartialEq, Eq)]
enum LexState {
    Code,
    SingleQuote,
    DoubleQuote,
    LineComment,
    BlockComment,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    idx: usize,
    state: LexState,
    paren_depth: u32,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a str) -> Self {
        Self {
            bytes: buf.as_bytes(),
            idx: 0,
            state: LexState::Code,
            paren_depth: 0,
        }
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.idx + offset).copied()
    }

    /// Advance one position, mutating state. Returns true if a top-level `;`
    /// was just consumed.
    fn step(&mut self) -> bool {
        let Some(byte) = self.peek(0) else {
            return false;
        };
        match self.state {
            LexState::Code => match byte {
                b'\'' => {
                    self.state = LexState::SingleQuote;
                    self.idx += 1;
                }
                b'"' => {
                    self.state = LexState::DoubleQuote;
                    self.idx += 1;
                }
                b'-' if self.peek(1) == Some(b'-') => {
                    self.state = LexState::LineComment;
                    self.idx += 2;
                }
                b'/' if self.peek(1) == Some(b'*') => {
                    self.state = LexState::BlockComment;
                    self.idx += 2;
                }
                b'(' => {
                    self.paren_depth += 1;
                    self.idx += 1;
                }
                b')' => {
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.idx += 1;
                }
                b';' if self.paren_depth == 0 => {
                    self.idx += 1;
                    return true;
                }
                _ => self.idx += 1,
            },
            LexState::SingleQuote => {
                if byte == b'\'' && self.peek(1) == Some(b'\'') {
                    self.idx += 2;
                } else if byte == b'\'' {
                    self.state = LexState::Code;
                    self.idx += 1;
                } else {
                    self.idx += 1;
                }
            }
            LexState::DoubleQuote => {
                if byte == b'"' && self.peek(1) == Some(b'"') {
                    self.idx += 2;
                } else if byte == b'"' {
                    self.state = LexState::Code;
                    self.idx += 1;
                } else {
                    self.idx += 1;
                }
            }
            LexState::LineComment => {
                if byte == b'\n' {
                    self.state = LexState::Code;
                }
                self.idx += 1;
            }
            LexState::BlockComment => {
                if byte == b'*' && self.peek(1) == Some(b'/') {
                    self.state = LexState::Code;
                    self.idx += 2;
                } else {
                    self.idx += 1;
                }
            }
        }
        false
    }
}

/// True iff `buf` contains at least one top-level `;` outside of any
/// quoted string, comment, or paren group.
pub fn is_statement_complete(buf: &str) -> bool {
    let mut cursor = Cursor::new(buf);
    while cursor.idx < cursor.bytes.len() {
        if cursor.step() {
            return true;
        }
    }
    false
}

/// True iff `buf` contains exactly one top-level `;` and the trailing tail
/// (after that semicolon) consists only of whitespace and comments.
pub fn is_single_statement(buf: &str) -> bool {
    let mut cursor = Cursor::new(buf);
    let mut found_terminator = false;
    while cursor.idx < cursor.bytes.len() {
        if cursor.step() {
            if found_terminator {
                return false;
            }
            found_terminator = true;
        }
    }
    found_terminator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_simple() {
        assert!(is_statement_complete("SELECT 1;"));
    }

    #[test]
    fn complete_multiline() {
        assert!(is_statement_complete("SELECT\n  1\n;"));
    }

    #[test]
    fn incomplete_no_semicolon() {
        assert!(!is_statement_complete("SELECT 1"));
    }

    #[test]
    fn incomplete_inside_string() {
        assert!(!is_statement_complete("SELECT 'abc;"));
    }

    #[test]
    fn complete_after_escaped_quote() {
        assert!(is_statement_complete("SELECT 'it''s';"));
    }

    #[test]
    fn incomplete_inside_block_comment() {
        assert!(!is_statement_complete("SELECT /* hi; */ 1"));
    }

    #[test]
    fn complete_after_block_comment() {
        assert!(is_statement_complete("SELECT /* hi */ 1;"));
    }

    #[test]
    fn incomplete_inside_line_comment() {
        assert!(!is_statement_complete("SELECT 1 -- hi;"));
    }

    #[test]
    fn complete_after_line_comment_with_newline() {
        assert!(is_statement_complete("SELECT 1 -- hi\n;"));
    }

    #[test]
    fn incomplete_inside_parens() {
        assert!(!is_statement_complete("SELECT (1; 2)"));
    }

    #[test]
    fn single_one_statement() {
        assert!(is_single_statement("SELECT 1;"));
    }

    #[test]
    fn single_with_trailing_whitespace() {
        assert!(is_single_statement("SELECT 1;   \n  "));
    }

    #[test]
    fn single_with_trailing_line_comment() {
        assert!(is_single_statement("SELECT 1; -- bye\n"));
    }

    #[test]
    fn single_with_trailing_block_comment() {
        assert!(is_single_statement("SELECT 1; /* bye */"));
    }

    #[test]
    fn single_rejects_two_statements() {
        assert!(!is_single_statement("SELECT 1; SELECT 2;"));
    }

    #[test]
    fn single_accepts_inner_semicolon_in_string() {
        assert!(is_single_statement("SELECT 'a;b';"));
    }

    #[test]
    fn single_rejects_no_terminator() {
        assert!(!is_single_statement("SELECT 1"));
    }
}
