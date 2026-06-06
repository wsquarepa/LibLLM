use anyhow::{Context as _, Result, bail};

use super::{ApiSetup, DbSetup, Matcher, Scenario, Setup, Step};

pub fn parse(src: &str) -> Result<Scenario> {
    let mut setup = Setup::default();
    let mut steps: Vec<Step> = Vec::new();

    #[derive(PartialEq)]
    enum Section {
        Preamble,
        Setup,
        Steps,
    }

    let mut section = Section::Preamble;

    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed == "[setup]" {
            section = Section::Setup;
            continue;
        }
        if trimmed == "[steps]" {
            section = Section::Steps;
            continue;
        }

        match section {
            Section::Preamble => {
                bail!("line {lineno}: unexpected content before any section header: {trimmed:?}");
            }
            Section::Setup => {
                parse_setup_line(trimmed, lineno, &mut setup)?;
            }
            Section::Steps => {
                let step = parse_step_line(trimmed, lineno)?;
                steps.push(step);
            }
        }
    }

    Ok(Scenario { setup, steps })
}

fn parse_setup_line(line: &str, lineno: usize, setup: &mut Setup) -> Result<()> {
    let (key, rest) = split_first_token(line);
    match key {
        "size" => {
            setup.size = parse_size(rest.trim(), lineno)?;
        }
        "db" => {
            setup.db = parse_db_setup(rest.trim(), lineno)?;
        }
        "api" => {
            setup.api = parse_api_setup(rest.trim(), lineno)?;
        }
        "override" => {
            setup.overrides.push(rest.trim().to_string());
        }
        "seed" => {
            setup.seed = Some(rest.trim().to_string());
        }
        other => {
            bail!("line {lineno}: unknown setup key {other:?}");
        }
    }
    Ok(())
}

fn parse_size(s: &str, lineno: usize) -> Result<(u16, u16)> {
    let (w_str, h_str) = s
        .split_once('x')
        .with_context(|| format!("line {lineno}: size must be WxH (e.g. 80x24), got {s:?}"))?;
    let w: u16 = w_str
        .parse()
        .with_context(|| format!("line {lineno}: invalid width in size {s:?}"))?;
    let h: u16 = h_str
        .parse()
        .with_context(|| format!("line {lineno}: invalid height in size {s:?}"))?;
    Ok((w, h))
}

fn parse_db_setup(s: &str, lineno: usize) -> Result<DbSetup> {
    match s {
        "none" => Ok(DbSetup::None),
        "temp" => Ok(DbSetup::Temp),
        other => {
            if let Some(pk) = other.strip_prefix("encrypted:") {
                Ok(DbSetup::Encrypted(pk.to_string()))
            } else {
                bail!(
                    "line {lineno}: unknown db setup {other:?}; expected none, temp, or encrypted:<passkey>"
                );
            }
        }
    }
}

fn parse_api_setup(s: &str, lineno: usize) -> Result<ApiSetup> {
    match s {
        "none" => Ok(ApiSetup::None),
        "mock" => Ok(ApiSetup::Mock),
        other => {
            bail!("line {lineno}: unknown api setup {other:?}; expected none or mock");
        }
    }
}

fn parse_step_line(line: &str, lineno: usize) -> Result<Step> {
    let (verb, rest) = split_first_token(line);
    match verb {
        "key" => {
            let name = rest.trim();
            if name.is_empty() {
                bail!("line {lineno}: key requires a key name");
            }
            Ok(Step::Key(name.to_string()))
        }
        "type" => {
            let text = parse_quoted(rest.trim(), lineno)?;
            Ok(Step::Type(text))
        }
        "paste" => {
            let text = parse_quoted(rest.trim(), lineno)?;
            Ok(Step::Paste(text))
        }
        "resize" => {
            let size = parse_size(rest.trim(), lineno)?;
            Ok(Step::Resize(size.0, size.1))
        }
        "pump" => Ok(Step::Pump),
        "advance" => {
            let dur = parse_duration(rest.trim(), lineno)?;
            Ok(Step::Advance(dur))
        }
        "snapshot" => {
            let name = rest.trim();
            if name.is_empty() {
                bail!("line {lineno}: snapshot requires a name");
            }
            Ok(Step::Snapshot(name.to_string()))
        }
        "enqueue" => parse_enqueue(rest.trim(), lineno),
        "expect" => parse_expect(rest.trim(), lineno),
        other => {
            bail!("line {lineno}: unknown verb {other:?}");
        }
    }
}

fn parse_enqueue(rest: &str, lineno: usize) -> Result<Step> {
    let (kind, args) = split_first_token(rest);
    match kind {
        "completion" => {
            let tokens = parse_all_quoted(args.trim(), lineno)?;
            Ok(Step::EnqueueCompletion(tokens))
        }
        "error" => {
            let msg = parse_quoted(args.trim(), lineno)?;
            Ok(Step::EnqueueError(msg))
        }
        other => {
            bail!("line {lineno}: unknown enqueue kind {other:?}; expected completion or error");
        }
    }
}

fn parse_expect(rest: &str, lineno: usize) -> Result<Step> {
    let (first, remainder) = split_first_token(rest);
    match first {
        "screen" => parse_expect_screen(remainder.trim(), lineno),
        "line" => parse_expect_line(remainder.trim(), lineno),
        probe => parse_expect_state(probe, remainder.trim(), lineno),
    }
}

fn parse_expect_screen(rest: &str, lineno: usize) -> Result<Step> {
    let (op, remainder) = split_first_token(rest);
    match op {
        "contains" => {
            let text = parse_quoted(remainder.trim(), lineno)?;
            Ok(Step::ExpectScreenContains(text))
        }
        "excludes" => {
            let text = parse_quoted(remainder.trim(), lineno)?;
            Ok(Step::ExpectScreenExcludes(text))
        }
        other => {
            bail!("line {lineno}: expect screen requires contains or excludes, got {other:?}");
        }
    }
}

fn parse_expect_line(rest: &str, lineno: usize) -> Result<Step> {
    let (n_str, remainder) = split_first_token(rest);
    let n: usize = n_str.parse().with_context(|| {
        format!("line {lineno}: expect line requires a line number, got {n_str:?}")
    })?;
    let matcher = parse_matcher(remainder.trim(), lineno)?;
    Ok(Step::ExpectLine { n, matcher })
}

fn parse_expect_state(probe: &str, rest: &str, lineno: usize) -> Result<Step> {
    let matcher = parse_matcher(rest, lineno)?;
    Ok(Step::ExpectState {
        probe: probe.to_string(),
        matcher,
    })
}

fn parse_matcher(rest: &str, lineno: usize) -> Result<Matcher> {
    let (op, remainder) = split_first_token(rest);
    match op {
        "==" => {
            let value = remainder.trim();
            if value == "null" {
                Ok(Matcher::Null)
            } else if value.starts_with('"') {
                let text = parse_quoted(value, lineno)?;
                Ok(Matcher::Eq(text))
            } else {
                Ok(Matcher::Eq(value.to_string()))
            }
        }
        "contains" => {
            let text = parse_quoted(remainder.trim(), lineno)?;
            Ok(Matcher::Contains(text))
        }
        other => {
            bail!("line {lineno}: expected == or contains, got {other:?}");
        }
    }
}

fn parse_duration(s: &str, lineno: usize) -> Result<std::time::Duration> {
    if let Some(ms_str) = s.strip_suffix("ms") {
        let ms: u64 = ms_str
            .parse()
            .with_context(|| format!("line {lineno}: invalid milliseconds in duration {s:?}"))?;
        Ok(std::time::Duration::from_millis(ms))
    } else if let Some(s_str) = s.strip_suffix('s') {
        let secs: u64 = s_str
            .parse()
            .with_context(|| format!("line {lineno}: invalid seconds in duration {s:?}"))?;
        Ok(std::time::Duration::from_secs(secs))
    } else {
        bail!("line {lineno}: duration must end with ms or s, got {s:?}");
    }
}

/// Splits off the first whitespace-delimited token from a string.
/// Returns `(token, rest)` where `rest` may be empty.
fn split_first_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(|c: char| c.is_ascii_whitespace()) {
        Some(pos) => (&s[..pos], &s[pos..]),
        None => (s, ""),
    }
}

/// Parses a single double-quoted string from `s`, supporting `\"` and `\n` escapes.
fn parse_quoted(s: &str, lineno: usize) -> Result<String> {
    let s = s.trim();
    if !s.starts_with('"') {
        bail!("line {lineno}: expected a double-quoted string, got {s:?}");
    }
    let inner = &s[1..];
    let mut out = String::new();
    let mut chars = inner.chars();
    loop {
        match chars.next() {
            None => bail!("line {lineno}: unterminated quoted string"),
            Some('"') => break,
            Some('\\') => match chars.next() {
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(c) => bail!("line {lineno}: unknown escape \\{c}"),
                None => bail!("line {lineno}: unterminated escape sequence"),
            },
            Some(c) => out.push(c),
        }
    }
    Ok(out)
}

/// Parses zero or more successive double-quoted strings from `s`.
fn parse_all_quoted(s: &str, lineno: usize) -> Result<Vec<String>> {
    let mut results = Vec::new();
    let mut remaining = s.trim();
    while !remaining.is_empty() {
        if !remaining.starts_with('"') {
            bail!("line {lineno}: expected a double-quoted string, got {remaining:?}");
        }
        let text = parse_quoted(remaining, lineno)?;
        results.push(text);
        // Advance past the consumed quoted string.
        let inner = &remaining[1..];
        let mut chars = inner.char_indices();
        let mut end = inner.len();
        let mut escaped = false;
        for (i, c) in chars.by_ref() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                end = i + 1;
                break;
            }
        }
        remaining = inner[end..].trim_start();
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::parse;

    #[test]
    fn parses_setup_and_steps() {
        let src = "\
[setup]
size 80x24
db temp
api mock

[steps]
type \"/persona\"
key Enter
expect focus == PersonaDialog
expect screen contains \"Persona\"
advance 6s
";
        let s = parse(src).unwrap();
        assert_eq!(s.setup.size, (80, 24));
        assert_eq!(s.setup.db, DbSetup::Temp);
        assert_eq!(s.setup.api, ApiSetup::Mock);
        assert_eq!(s.steps[0], Step::Type("/persona".into()));
        assert_eq!(s.steps[1], Step::Key("Enter".into()));
        assert_eq!(
            s.steps[2],
            Step::ExpectState {
                probe: "focus".into(),
                matcher: Matcher::Eq("PersonaDialog".into())
            }
        );
        assert_eq!(s.steps[3], Step::ExpectScreenContains("Persona".into()));
        assert_eq!(s.steps[4], Step::Advance(std::time::Duration::from_secs(6)));
    }

    #[test]
    fn rejects_unknown_verb() {
        assert!(parse("[steps]\nfrobnicate x\n").is_err());
    }

    #[test]
    fn parses_null_matcher() {
        let s = parse("[steps]\nexpect status_message == null\n").unwrap();
        assert_eq!(
            s.steps[0],
            Step::ExpectState {
                probe: "status_message".into(),
                matcher: Matcher::Null
            }
        );
    }

    #[test]
    fn parses_screen_excludes_and_line() {
        let s = parse("[steps]\nexpect screen excludes \"err\"\nexpect line 3 == \"hello\"\nexpect line 0 contains \"hi\"\n").unwrap();
        assert_eq!(s.steps[0], Step::ExpectScreenExcludes("err".into()));
        assert_eq!(
            s.steps[1],
            Step::ExpectLine {
                n: 3,
                matcher: Matcher::Eq("hello".into())
            }
        );
        assert_eq!(
            s.steps[2],
            Step::ExpectLine {
                n: 0,
                matcher: Matcher::Contains("hi".into())
            }
        );
    }

    #[test]
    fn parses_enqueue_and_resize_and_advance_ms() {
        let s = parse("[steps]\nenqueue completion \"a\" \"b\"\nenqueue error \"boom\"\nresize 120x40\nadvance 500ms\npump\n").unwrap();
        assert_eq!(
            s.steps[0],
            Step::EnqueueCompletion(vec!["a".into(), "b".into()])
        );
        assert_eq!(s.steps[1], Step::EnqueueError("boom".into()));
        assert_eq!(s.steps[2], Step::Resize(120, 40));
        assert_eq!(
            s.steps[3],
            Step::Advance(std::time::Duration::from_millis(500))
        );
        assert_eq!(s.steps[4], Step::Pump);
    }

    #[test]
    fn ignores_comments_and_blanks() {
        let s = parse("# a comment\n[setup]\n# another\nsize 80x24\n\n[steps]\n\npump\n").unwrap();
        assert_eq!(s.setup.size, (80, 24));
        assert_eq!(s.steps, vec![Step::Pump]);
    }

    #[test]
    fn parses_overrides_and_seed_and_db_none_api_none() {
        let s = parse("[setup]\ndb none\napi none\noverride persona_readonly\nseed fixtures/x.sql\n[steps]\npump\n").unwrap();
        assert_eq!(s.setup.db, DbSetup::None);
        assert_eq!(s.setup.api, ApiSetup::None);
        assert_eq!(s.setup.overrides, vec!["persona_readonly".to_string()]);
        assert_eq!(s.setup.seed.as_deref(), Some("fixtures/x.sql"));
    }

    #[test]
    fn parses_encrypted_db() {
        let s = parse("[setup]\ndb encrypted:hunter2\n[steps]\npump\n").unwrap();
        assert_eq!(s.setup.db, DbSetup::Encrypted("hunter2".into()));
    }

    #[test]
    fn parses_snapshot_and_paste() {
        let s = parse("[steps]\npaste \"a\\nb\"\nsnapshot persona_open\n").unwrap();
        assert_eq!(s.steps[0], Step::Paste("a\nb".into()));
        assert_eq!(s.steps[1], Step::Snapshot("persona_open".into()));
    }

    #[test]
    fn error_reports_one_based_line_number() {
        let err = parse("[steps]\nfrobnicate x\n").unwrap_err();
        assert!(
            err.to_string().contains("line 2:"),
            "expected a 1-based line number in the error, got: {err}"
        );
    }

    #[test]
    fn rejects_unterminated_quoted_string() {
        let err = parse("[steps]\ntype \"unclosed\n").unwrap_err();
        assert!(
            err.to_string().contains("unterminated quoted string"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_content_before_any_section() {
        assert!(parse("pump\n").is_err());
    }
}
