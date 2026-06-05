use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

use crate::db::Database;
use libllm_core::session::Role;

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

#[derive(Debug, Clone)]
pub(crate) struct ParsedQuery {
    pub match_expr: String,
    pub session_substring: Option<String>,
    pub role: Option<Role>,
    pub before: Option<OffsetDateTime>,
    pub after: Option<OffsetDateTime>,
}

pub(crate) fn parse(raw: &str) -> Result<ParsedQuery, QueryError> {
    let mut session_substring: Option<String> = None;
    let mut role: Option<Role> = None;
    let mut before: Option<OffsetDateTime> = None;
    let mut after: Option<OffsetDateTime> = None;

    let mut leftover = String::new();
    let mut raw_match: Option<String> = None;

    let tokens = tokenize(raw);
    if tokens.is_empty() {
        return Err(QueryError::Empty);
    }

    let mut iter = tokens.into_iter();
    while let Some(token) = iter.next() {
        match token {
            Token::Phrase(phrase) => {
                if !leftover.is_empty() {
                    leftover.push(' ');
                }
                leftover.push('"');
                leftover.push_str(&phrase);
                leftover.push('"');
            }
            Token::Term(term) => {
                if let Some((key, value)) = split_filter(&term) {
                    apply_filter(
                        key,
                        value,
                        &mut session_substring,
                        &mut role,
                        &mut before,
                        &mut after,
                    )?;
                    continue;
                }
                if let Some(rest) = term.strip_prefix("m:") {
                    let mut buf = rest.to_owned();
                    for next in iter.by_ref() {
                        match next {
                            Token::Phrase(p) => {
                                buf.push_str(" \"");
                                buf.push_str(&p);
                                buf.push('"');
                            }
                            Token::Term(t) => {
                                buf.push(' ');
                                buf.push_str(&t);
                            }
                        }
                    }
                    raw_match = Some(buf.trim().to_owned());
                    break;
                }
                if !leftover.is_empty() {
                    leftover.push(' ');
                }
                leftover.push_str(&term);
            }
        }
    }

    let match_expr = if let Some(raw) = raw_match {
        if raw.is_empty() {
            return Err(QueryError::Empty);
        }
        raw
    } else {
        compile_match_expr(&leftover)?
    };

    Ok(ParsedQuery {
        match_expr,
        session_substring,
        role,
        before,
        after,
    })
}

fn compile_match_expr(input: &str) -> Result<String, QueryError> {
    let tokens = tokenize(input);
    let mut parts: Vec<String> = Vec::new();
    for token in tokens {
        match token {
            Token::Phrase(phrase) => parts.push(format!("\"{}\"", phrase)),
            Token::Term(term) => {
                let cleaned = sanitize_term(&term);
                if !cleaned.is_empty() {
                    parts.push(format!("{cleaned}*"));
                }
            }
        }
    }
    if parts.is_empty() {
        return Err(QueryError::Empty);
    }
    Ok(parts.join(" AND "))
}

fn split_filter(token: &str) -> Option<(&str, &str)> {
    let (key, value) = token.split_once(':')?;
    if matches!(key, "session" | "role" | "before" | "after") && !value.is_empty() {
        Some((key, value))
    } else {
        None
    }
}

fn apply_filter(
    key: &str,
    value: &str,
    session_substring: &mut Option<String>,
    role: &mut Option<Role>,
    before: &mut Option<OffsetDateTime>,
    after: &mut Option<OffsetDateTime>,
) -> Result<(), QueryError> {
    match key {
        "session" => {
            *session_substring = Some(value.to_owned());
        }
        "role" => {
            let parsed = value
                .parse::<Role>()
                .map_err(|_| QueryError::BadFilter(format!("role:{value}")))?;
            *role = Some(parsed);
        }
        "before" => {
            *before = Some(parse_date(value)?);
        }
        "after" => {
            *after = Some(parse_date(value)?);
        }
        _ => unreachable!("split_filter guards the key set"),
    }
    Ok(())
}

fn parse_date(value: &str) -> Result<OffsetDateTime, QueryError> {
    if let Ok(dt) = OffsetDateTime::parse(value, &Rfc3339) {
        return Ok(dt);
    }
    let format = format_description!("[year]-[month]-[day]");
    if let Ok(date) = time::Date::parse(value, &format) {
        return Ok(OffsetDateTime::new_utc(date, time::Time::MIDNIGHT));
    }
    Err(QueryError::ParseDate(value.to_owned()))
}

#[derive(Debug)]
enum Token {
    Term(String),
    Phrase(String),
}

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

fn sanitize_term(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(*c, '"' | '*' | '(' | ')' | ':' | '^'))
        .collect()
}

pub fn compile(raw: &str, db: &Database) -> Result<CompiledQuery, QueryError> {
    let parsed = parse(raw)?;

    let session_ids = match parsed.session_substring.as_deref() {
        None => None,
        Some(substring) => {
            let ids = db
                .session_ids_matching_display_name(substring)
                .map_err(|e| QueryError::BadFilter(format!("session lookup failed: {e}")))?;
            if ids.is_empty() {
                return Err(QueryError::UnknownSession(substring.to_owned()));
            }
            Some(ids)
        }
    };

    Ok(CompiledQuery {
        match_expr: parsed.match_expr,
        session_ids,
        role: parsed.role,
        before: parsed.before,
        after: parsed.after,
    })
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
        assert_eq!(match_expr_only("\"redact pii\"").unwrap(), "\"redact pii\"");
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

    #[test]
    fn role_filter_lifts_out_of_terms() {
        let parsed = parse("role:user redact").unwrap();
        assert_eq!(parsed.match_expr, "redact*");
        assert_eq!(parsed.role, Some(Role::User));
    }

    #[test]
    fn role_filter_accepts_assistant_and_system() {
        assert_eq!(
            parse("role:assistant x").unwrap().role,
            Some(Role::Assistant)
        );
        assert_eq!(parse("role:system x").unwrap().role, Some(Role::System));
    }

    #[test]
    fn role_filter_unknown_value_errors() {
        let err = parse("role:bogus x").unwrap_err();
        assert_eq!(err, QueryError::BadFilter("role:bogus".into()));
    }

    #[test]
    fn before_filter_parses_iso_date() {
        let parsed = parse("before:2026-01-15 retry").unwrap();
        assert_eq!(parsed.match_expr, "retry*");
        assert_eq!(
            parsed.before.unwrap().format(&Rfc3339).unwrap(),
            "2026-01-15T00:00:00Z"
        );
    }

    #[test]
    fn after_filter_parses_iso_date() {
        let parsed = parse("after:2025-12-01 retry").unwrap();
        assert_eq!(
            parsed.after.unwrap().format(&Rfc3339).unwrap(),
            "2025-12-01T00:00:00Z"
        );
    }

    #[test]
    fn before_filter_bad_date_errors() {
        let err = parse("before:not-a-date x").unwrap_err();
        assert_eq!(err, QueryError::ParseDate("not-a-date".into()));
    }

    #[test]
    fn before_filter_parses_rfc3339_datetime() {
        let parsed = parse("before:2026-01-15T12:30:00Z retry").unwrap();
        assert_eq!(
            parsed.before.unwrap().format(&Rfc3339).unwrap(),
            "2026-01-15T12:30:00Z"
        );
    }

    #[test]
    fn session_filter_captures_substring() {
        let parsed = parse("session:feature bug").unwrap();
        assert_eq!(parsed.session_substring, Some("feature".to_owned()));
        assert_eq!(parsed.match_expr, "bug*");
    }

    #[test]
    fn quoted_phrase_with_colon_is_not_a_filter() {
        let parsed = parse("\"role:user friendly\"").unwrap();
        assert_eq!(parsed.match_expr, "\"role:user friendly\"");
        assert_eq!(parsed.role, None);
    }

    #[test]
    fn raw_mode_passes_through_match_expr() {
        let parsed = parse("m:redact NEAR/3 pii").unwrap();
        assert_eq!(parsed.match_expr, "redact NEAR/3 pii");
    }

    #[test]
    fn raw_mode_keeps_scope_filters() {
        let parsed = parse("role:user m:redact NEAR/3 pii").unwrap();
        assert_eq!(parsed.match_expr, "redact NEAR/3 pii");
        assert_eq!(parsed.role, Some(Role::User));
    }

    #[test]
    fn raw_mode_alone_with_no_match_is_empty() {
        let err = parse("m:").unwrap_err();
        assert_eq!(err, QueryError::Empty);
    }

    #[test]
    fn raw_mode_preserves_quoted_phrase_in_drain() {
        let parsed = parse("m:foo \"hello world\" bar").unwrap();
        assert_eq!(parsed.match_expr, "foo \"hello world\" bar");
    }

    #[test]
    fn raw_mode_second_m_prefix_is_literal() {
        let parsed = parse("m:a m:b").unwrap();
        assert_eq!(parsed.match_expr, "a m:b");
    }

    fn seed_db_with_sessions(
        names: &[(&str, &str)],
    ) -> (crate::db::Database, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(file.path(), None).unwrap();
        for (id, display) in names {
            db.execute_statement(&format!(
                "INSERT INTO sessions (id, display_name, created_at, updated_at) \
                 VALUES ('{id}', '{display}', 'now', 'now')"
            ))
            .unwrap();
        }
        (db, file)
    }

    #[test]
    fn session_substring_resolves_multiple_ids() {
        let (db, _file) = seed_db_with_sessions(&[
            ("a", "feature-x"),
            ("b", "feature-y"),
            ("c", "log-pipeline"),
        ]);
        let compiled = compile("session:feature bug", &db).unwrap();
        let ids = compiled.session_ids.unwrap();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(compiled.match_expr, "bug*");
    }

    #[test]
    fn session_substring_is_case_insensitive() {
        let (db, _file) = seed_db_with_sessions(&[("a", "Feature-X")]);
        let compiled = compile("session:feature bug", &db).unwrap();
        assert_eq!(compiled.session_ids.unwrap(), vec!["a".to_string()]);
    }

    #[test]
    fn session_substring_no_match_errors() {
        let (db, _file) = seed_db_with_sessions(&[("a", "feature-x")]);
        let err = compile("session:nonexistent x", &db).unwrap_err();
        assert_eq!(err, QueryError::UnknownSession("nonexistent".into()));
    }

    #[test]
    fn session_filter_literal_percent_matches_only_session_with_percent_in_name() {
        let (db, _file) = seed_db_with_sessions(&[("a", "100% done"), ("b", "something else")]);
        let compiled = compile("session:% done", &db).unwrap();
        assert_eq!(
            compiled.session_ids.unwrap(),
            vec!["a".to_string()],
            "session:% should match only the session whose name contains a literal '%'"
        );
    }

    #[test]
    fn session_filter_literal_underscore_matches_only_session_with_underscore_in_name() {
        let (db, _file) = seed_db_with_sessions(&[("a", "feature_x"), ("b", "plain")]);
        let compiled = compile("session:feature_x x", &db).unwrap();
        assert_eq!(
            compiled.session_ids.unwrap(),
            vec!["a".to_string()],
            "session:feature_x should match only the session whose name contains a literal '_'"
        );
    }

    #[test]
    fn session_filter_literal_backslash_in_name() {
        let (db, _file) = seed_db_with_sessions(&[("a", "foo\\bar")]);
        let compiled = compile("session:foo\\bar x", &db).unwrap();
        assert_eq!(compiled.session_ids.unwrap(), vec!["a".to_string()]);
    }
}
