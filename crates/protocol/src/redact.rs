//! Redact secrets that can appear in transport error text (especially URLs).

/// Redact secrets from free-form error text that may embed request URLs.
///
/// Finds `http://` / `https://` substrings and, within each:
/// - replaces URL userinfo (`user:pass@`) with `REDACTED@`
/// - replaces every query-parameter *value* with `REDACTED` (names stay)
///
/// Does not require Auth context; works on any string that happens to contain URLs.
pub fn redact_error_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        if let Some(scheme_len) = scheme_prefix_len(&s[i..]) {
            let url_start = i;
            let url_end = url_start + scheme_len + url_body_len(&s[url_start + scheme_len..]);
            out.push_str(&redact_url(&s[url_start..url_end]));
            i = url_end;
            continue;
        }
        let Some(ch) = s[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn scheme_prefix_len(s: &str) -> Option<usize> {
    if s.starts_with("https://") {
        Some("https://".len())
    } else if s.starts_with("http://") {
        Some("http://".len())
    } else {
        None
    }
}

/// Length of the URL body after `http://` or `https://`, stopping at common
/// delimiters that appear around URLs in error messages (e.g. reqwest's
/// `error sending request for url (...): ...`).
fn url_body_len(after_scheme: &str) -> usize {
    after_scheme
        .chars()
        .take_while(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | ')' | '<' | '>' | '"' | '\''))
        .map(char::len_utf8)
        .sum()
}

fn redact_url(url: &str) -> String {
    let Some(scheme_sep) = url.find("://") else {
        return url.to_owned();
    };
    let after_scheme_idx = scheme_sep + "://".len();
    let rest = &url[after_scheme_idx..];

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let after_authority = &rest[authority_end..];

    let redacted_authority = match authority.rfind('@') {
        Some(at) => format!("REDACTED@{}", &authority[at + 1..]),
        None => authority.to_owned(),
    };

    let mut result = String::with_capacity(url.len());
    result.push_str(&url[..after_scheme_idx]);
    result.push_str(&redacted_authority);

    match after_authority.find('?') {
        None => {
            result.push_str(after_authority);
        }
        Some(qpos) => {
            result.push_str(&after_authority[..qpos]);
            result.push('?');
            let query_and_frag = &after_authority[qpos + 1..];
            match query_and_frag.find('#') {
                Some(h) => {
                    result.push_str(&redact_query(&query_and_frag[..h]));
                    result.push_str(&query_and_frag[h..]);
                }
                None => {
                    result.push_str(&redact_query(query_and_frag));
                }
            }
        }
    }
    result
}

fn redact_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(query.len());
    for (i, pair) in query.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        match pair.find('=') {
            Some(eq) => {
                out.push_str(&pair[..eq]);
                out.push_str("=REDACTED");
            }
            None => out.push_str(pair),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact_error_text;

    #[test]
    fn redacts_query_values_in_url_shaped_error_text() {
        let raw = "error sending request for url (http://127.0.0.1:9/v1/models?api_key=LIBLLM_SECRET_TOKEN_123): connection refused";
        let out = redact_error_text(raw);
        assert!(!out.contains("LIBLLM_SECRET_TOKEN_123"));
        assert!(out.contains("api_key="));
        assert!(out.contains("REDACTED") || out.contains("***"));
    }

    #[test]
    fn redacts_url_userinfo() {
        let raw =
            "error sending request for url (http://user:SECRET@127.0.0.1:9/): connection refused";
        let out = redact_error_text(raw);
        assert!(!out.contains("SECRET"));
    }

    #[test]
    fn redacts_multiple_query_params_and_preserves_names() {
        let raw = "url (https://example.com/path?a=one&b=two&c=): fail";
        let out = redact_error_text(raw);
        assert!(!out.contains("one"));
        assert!(!out.contains("two"));
        assert!(out.contains("a=REDACTED"));
        assert!(out.contains("b=REDACTED"));
        assert!(out.contains("c=REDACTED"));
        assert!(out.contains("https://example.com/path?"));
    }

    #[test]
    fn leaves_non_url_text_unchanged() {
        let raw = "plain connection refused without a url";
        assert_eq!(redact_error_text(raw), raw);
    }
}
