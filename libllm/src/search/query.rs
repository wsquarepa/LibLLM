use time::OffsetDateTime;

use crate::session::Role;

#[derive(Debug, Clone)]
pub struct CompiledQuery {
    pub match_expr: String,
    pub session_ids: Option<Vec<String>>,
    pub role: Option<Role>,
    pub before: Option<OffsetDateTime>,
    pub after: Option<OffsetDateTime>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    Empty,
    BadFilter(String),
    UnknownSession(String),
    ParseDate(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("query is empty"),
            Self::BadFilter(s) => write!(f, "malformed scope filter: {s}"),
            Self::UnknownSession(s) => write!(f, "no session matched: {s}"),
            Self::ParseDate(s) => write!(f, "malformed date: {s}"),
        }
    }
}

impl std::error::Error for QueryError {}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ParsedQuery {
    pub match_expr: String,
    #[expect(dead_code, reason = "populated by later tasks that parse the session: filter")]
    pub session_substring: Option<String>,
    pub role: Option<Role>,
    pub before: Option<OffsetDateTime>,
    pub after: Option<OffsetDateTime>,
}

#[cfg(test)]
pub(crate) fn parse(raw: &str) -> Result<ParsedQuery, QueryError> {
    let tokens = tokenize(raw);
    if tokens.is_empty() {
        return Err(QueryError::Empty);
    }

    let mut match_parts: Vec<String> = Vec::new();
    for token in tokens {
        match token {
            Token::Phrase(phrase) => {
                match_parts.push(format!("\"{}\"", phrase));
            }
            Token::Term(term) => {
                let cleaned = sanitize_term(&term);
                if !cleaned.is_empty() {
                    match_parts.push(format!("{cleaned}*"));
                }
            }
        }
    }

    if match_parts.is_empty() {
        return Err(QueryError::Empty);
    }

    Ok(ParsedQuery {
        match_expr: match_parts.join(" AND "),
        session_substring: None,
        role: None,
        before: None,
        after: None,
    })
}

#[cfg(test)]
#[derive(Debug)]
enum Token {
    Term(String),
    Phrase(String),
}

#[cfg(test)]
fn tokenize(raw: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' {
            chars.next();
            let mut phrase = String::new();
            for ch in chars.by_ref() {
                if ch == '"' {
                    break;
                }
                phrase.push(ch);
            }
            if !phrase.is_empty() {
                tokens.push(Token::Phrase(phrase));
            }
            continue;
        }
        let mut term = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() {
                break;
            }
            term.push(ch);
            chars.next();
        }
        if !term.is_empty() {
            tokens.push(Token::Term(term));
        }
    }
    tokens
}

#[cfg(test)]
fn sanitize_term(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(*c, '"' | '*' | '(' | ')' | ':' | '^'))
        .collect()
}

#[cfg(test)]
pub(crate) fn compile_match_only(raw: &str) -> Result<CompiledQuery, QueryError> {
    let parsed = parse(raw)?;
    Ok(CompiledQuery {
        match_expr: parsed.match_expr,
        session_ids: None,
        role: parsed.role,
        before: parsed.before,
        after: parsed.after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn match_expr_only(raw: &str) -> Result<String, QueryError> {
        compile_match_only(raw).map(|q| q.match_expr)
    }

    #[test]
    fn single_term_gets_prefix_star() {
        assert_eq!(match_expr_only("redact").unwrap(), "redact*");
    }

    #[test]
    fn multiple_terms_are_anded() {
        assert_eq!(match_expr_only("redact pii").unwrap(), "redact* AND pii*");
    }

    #[test]
    fn quoted_phrase_is_preserved() {
        assert_eq!(
            match_expr_only("\"redact pii\"").unwrap(),
            "\"redact pii\""
        );
    }

    #[test]
    fn quoted_and_term_combined() {
        assert_eq!(
            match_expr_only("\"redact pii\" log").unwrap(),
            "\"redact pii\" AND log*"
        );
    }

    #[test]
    fn special_chars_are_stripped_from_terms() {
        assert_eq!(match_expr_only("re*dact").unwrap(), "redact*");
        assert_eq!(match_expr_only("re(dact)").unwrap(), "redact*");
    }

    #[test]
    fn empty_after_stripping_is_empty_error() {
        assert_eq!(match_expr_only("***"), Err(QueryError::Empty));
        assert_eq!(match_expr_only("   "), Err(QueryError::Empty));
    }
}
