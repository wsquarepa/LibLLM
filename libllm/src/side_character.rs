//! Parser that splits a user input into segments when it contains
//! `[Name]: voice` blocks delimited by blank lines. Pure; never errors.

/// Split `raw` into an ordered list of message segments.
///
/// The first segment (if non-empty) is the user's own voice — everything
/// before the first side-character block. Each subsequent segment is a
/// side-character block, stored verbatim including the `[Name]:` header.
///
/// If `raw` contains no well-formed side-character block, the returned vec
/// holds a single element equal to `raw.trim_end()` (or is empty if the
/// trimmed input is empty).
///
/// Header recognition: a line is a header if and only if
///   (a) it is preceded by a blank line (or is the first non-blank line),
///   (b) its first non-whitespace character is `[`, and
///   (c) it contains `]:` on the same line after a non-empty name.
/// A line whose first non-whitespace sequence is `\[` is never a header;
/// the leading `\` is stripped from that line in the produced segment.
pub fn split_user_input(raw: &str) -> Vec<String> {
    let lines: Vec<&str> = raw.split('\n').collect();

    let mut header_indices: Vec<usize> = Vec::new();
    let mut prev_blank = true;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let is_blank = trimmed.is_empty();
        if prev_blank && is_header_line(trimmed) {
            header_indices.push(idx);
        }
        prev_blank = is_blank;
    }

    if header_indices.is_empty() {
        let trimmed = raw.trim_end();
        if trimmed.trim().is_empty() {
            return Vec::new();
        }
        let unescaped = unescape_bracket_prefixes(trimmed);
        return vec![unescaped];
    }

    let mut segments: Vec<String> = Vec::new();

    let first_header = header_indices[0];
    if first_header > 0 {
        let user_text = lines[..first_header].join("\n");
        let trimmed = user_text.trim_end();
        if !trimmed.trim().is_empty() {
            segments.push(unescape_bracket_prefixes(trimmed));
        }
    }

    for (i, &start) in header_indices.iter().enumerate() {
        let end = header_indices
            .get(i + 1)
            .copied()
            .unwrap_or(lines.len());
        let block_lines = &lines[start..end];
        let mut block = block_lines.join("\n");
        block = block.trim_end().to_owned();
        if !block.is_empty() {
            segments.push(block);
        }
    }

    segments
}

/// Parse a single stored side-character block into `(name, body)`.
///
/// Returns `Some((name, body))` when the first non-blank line of `block` is a
/// recognised `[Name]:` header (same rules as the internal header check used
/// by `split_user_input`, except the `\[` escape yields `None` because
/// rendered blocks have already been through unescape on the way in).
///
/// `name` is the content between `[` and `]:`, trimmed on both sides.
/// `body` is everything after `]:` on the first line (with exactly one
/// leading space consumed if present), concatenated with the remaining lines
/// verbatim and joined with `\n`.
///
/// Returns `None` for plain text, empty-name headers (`[]:`), escaped
/// headers (`\[Name]:`), or any line whose first non-whitespace char is not
/// `[`.
pub fn parse_side_character_block(block: &str) -> Option<(String, String)> {
    let mut lines = block.split('\n');
    let first = lines.next()?;
    let trimmed = first.trim_start();
    if trimmed.starts_with("\\[") || !trimmed.starts_with('[') {
        return None;
    }
    let close_idx = trimmed.find("]:")?;
    if close_idx <= 1 {
        return None;
    }
    let name = trimmed[1..close_idx].trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let after = &trimmed[close_idx + 2..];
    let first_body = after.strip_prefix(' ').unwrap_or(after);

    let rest: Vec<&str> = lines.collect();
    let body = if rest.is_empty() {
        first_body.to_owned()
    } else {
        let mut out = String::with_capacity(block.len());
        out.push_str(first_body);
        for line in rest {
            out.push('\n');
            out.push_str(line);
        }
        out
    };

    Some((name, body))
}

/// Byte range of one `[Name]:` header prefix within a raw multi-line input,
/// expressed as (`line`, `start`..`end`) where offsets index into the line
/// returned by `raw.split('\n')`. `end` is exclusive and points one past the
/// `:` of `]:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderPrefix {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Find every `[Name]:` header prefix the side-character parser would
/// recognise in `raw`, using the same rules as `split_user_input`
/// (blank-line-preceded, `\[` escape suppresses, empty name rejected).
///
/// Returned ranges are over bytes within each line, suitable for feeding
/// directly to `tui-textarea`'s `custom_highlight` coordinates.
pub fn header_prefix_ranges(raw: &str) -> Vec<HeaderPrefix> {
    let mut out: Vec<HeaderPrefix> = Vec::new();
    let mut prev_blank = true;
    for (idx, line) in raw.split('\n').enumerate() {
        let leading_ws = line
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        let trimmed = &line[leading_ws..];
        let is_blank = trimmed.is_empty();
        if prev_blank && is_header_line(trimmed) {
            let close_idx = trimmed
                .find("]:")
                .expect("is_header_line guarantees ]: is present");
            out.push(HeaderPrefix {
                line: idx,
                start: leading_ws,
                end: leading_ws + close_idx + 2,
            });
        }
        prev_blank = is_blank;
    }
    out
}

fn is_header_line(trimmed: &str) -> bool {
    if trimmed.starts_with("\\[") {
        return false;
    }
    if !trimmed.starts_with('[') {
        return false;
    }
    let Some(close_idx) = trimmed.find("]:") else {
        return false;
    };
    close_idx > 1
}

fn unescape_bracket_prefixes(text: &str) -> String {
    let mut lines = text.split('\n');
    let Some(first) = lines.next() else {
        return String::new();
    };
    let rest: Vec<&str> = lines.collect();
    let first_rewritten = rewrite_escape(first);
    if rest.is_empty() {
        return first_rewritten;
    }
    let mut out = first_rewritten;
    for line in rest {
        out.push('\n');
        out.push_str(&rewrite_escape(line));
    }
    out
}

fn rewrite_escape(line: &str) -> String {
    let leading_ws_end = line
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    let (ws, tail) = line.split_at(leading_ws_end);
    if tail.starts_with("\\[") {
        let mut out = String::with_capacity(line.len());
        out.push_str(ws);
        out.push_str(&tail[1..]);
        out
    } else {
        line.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::split_user_input;

    #[test]
    fn plain_input_returns_single_segment() {
        assert_eq!(split_user_input("hello world"), vec!["hello world"]);
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        assert!(split_user_input("").is_empty());
        assert!(split_user_input("   \n  ").is_empty());
    }

    #[test]
    fn one_side_character_splits_into_two_segments() {
        let raw = "I walk into the tavern.\n\n[Barkeep]: Welcome, stranger.";
        assert_eq!(
            split_user_input(raw),
            vec![
                "I walk into the tavern.".to_owned(),
                "[Barkeep]: Welcome, stranger.".to_owned(),
            ]
        );
    }

    #[test]
    fn two_side_characters_produce_three_segments() {
        let raw = "I open the door.\n\n[Alice]: Hi.\n\n[Bob]: Hello there.";
        assert_eq!(
            split_user_input(raw),
            vec![
                "I open the door.".to_owned(),
                "[Alice]: Hi.".to_owned(),
                "[Bob]: Hello there.".to_owned(),
            ]
        );
    }

    #[test]
    fn leading_side_character_produces_only_side_segments() {
        let raw = "[Alice]: First line.\n\n[Bob]: Second line.";
        assert_eq!(
            split_user_input(raw),
            vec![
                "[Alice]: First line.".to_owned(),
                "[Bob]: Second line.".to_owned(),
            ]
        );
    }

    #[test]
    fn multi_line_voice_is_preserved_until_next_header() {
        let raw = "User voice.\n\n[Alice]: line 1\nline 2\nline 3";
        assert_eq!(
            split_user_input(raw),
            vec![
                "User voice.".to_owned(),
                "[Alice]: line 1\nline 2\nline 3".to_owned(),
            ]
        );
    }

    #[test]
    fn blank_line_terminates_voice_only_when_next_line_is_header() {
        let raw =
            "User.\n\n[Alice]: line 1\n\nline 2\n\n[Bob]: line 3";
        assert_eq!(
            split_user_input(raw),
            vec![
                "User.".to_owned(),
                "[Alice]: line 1\n\nline 2".to_owned(),
                "[Bob]: line 3".to_owned(),
            ]
        );
    }

    #[test]
    fn escape_backslash_prevents_header_and_is_stripped() {
        let raw = "User.\n\n\\[NotAHeader]: still user voice";
        assert_eq!(
            split_user_input(raw),
            vec!["User.\n\n[NotAHeader]: still user voice".to_owned()]
        );
    }

    #[test]
    fn bracket_without_blank_line_is_not_a_header() {
        let raw = "I said [quote]: not a side character";
        assert_eq!(
            split_user_input(raw),
            vec!["I said [quote]: not a side character".to_owned()]
        );
    }

    #[test]
    fn empty_brackets_are_not_a_header() {
        let raw = "User.\n\n[]: nope";
        assert_eq!(
            split_user_input(raw),
            vec!["User.\n\n[]: nope".to_owned()]
        );
    }

    #[test]
    fn header_with_leading_whitespace_is_recognized() {
        let raw = "User.\n\n   [Alice]: voice";
        assert_eq!(
            split_user_input(raw),
            vec!["User.".to_owned(), "   [Alice]: voice".to_owned()]
        );
    }

    #[test]
    fn trailing_blank_lines_in_user_voice_are_stripped() {
        let raw = "User voice.\n\n\n\n[Alice]: voice";
        assert_eq!(
            split_user_input(raw),
            vec![
                "User voice.".to_owned(),
                "[Alice]: voice".to_owned(),
            ]
        );
    }

    use super::parse_side_character_block;

    #[test]
    fn parse_side_character_block_extracts_name_and_body() {
        let out = parse_side_character_block("[Alice]: hello world");
        assert_eq!(out, Some(("Alice".to_owned(), "hello world".to_owned())));
    }

    #[test]
    fn parse_side_character_block_preserves_multi_line_body() {
        let out = parse_side_character_block("[Alice]: line 1\nline 2\n\nline 4");
        assert_eq!(
            out,
            Some((
                "Alice".to_owned(),
                "line 1\nline 2\n\nline 4".to_owned(),
            )),
        );
    }

    #[test]
    fn parse_side_character_block_returns_none_for_plain_text() {
        assert_eq!(parse_side_character_block("hello world"), None);
    }

    #[test]
    fn parse_side_character_block_returns_none_for_escaped_header() {
        assert_eq!(parse_side_character_block("\\[Alice]: hi"), None);
    }

    #[test]
    fn parse_side_character_block_rejects_empty_name() {
        assert_eq!(parse_side_character_block("[]: nope"), None);
    }

    #[test]
    fn parse_side_character_block_handles_leading_whitespace() {
        let out = parse_side_character_block("   [Alice]: hi");
        assert_eq!(out, Some(("Alice".to_owned(), "hi".to_owned())));
    }

    #[test]
    fn parse_side_character_block_trims_name_whitespace() {
        let out = parse_side_character_block("[  Alice  ]: hi");
        assert_eq!(out, Some(("Alice".to_owned(), "hi".to_owned())));
    }

    #[test]
    fn parse_side_character_block_handles_missing_body() {
        let out = parse_side_character_block("[Alice]:");
        assert_eq!(out, Some(("Alice".to_owned(), "".to_owned())));
    }

    #[test]
    fn parse_side_character_block_consumes_single_leading_space_only() {
        let out = parse_side_character_block("[Alice]:  two spaces");
        assert_eq!(out, Some(("Alice".to_owned(), " two spaces".to_owned())));
    }

    use super::{header_prefix_ranges, HeaderPrefix};

    #[test]
    fn header_prefix_ranges_returns_empty_for_plain_text() {
        assert!(header_prefix_ranges("hello world").is_empty());
        assert!(header_prefix_ranges("").is_empty());
    }

    #[test]
    fn header_prefix_ranges_finds_single_header_at_start() {
        let ranges = header_prefix_ranges("[Alice]: hi");
        assert_eq!(
            ranges,
            vec![HeaderPrefix { line: 0, start: 0, end: 8 }],
        );
    }

    #[test]
    fn header_prefix_ranges_finds_multiple_headers_after_blank_lines() {
        let raw = "User voice.\n\n[Alice]: hi\n\n[Bob]: hello";
        let ranges = header_prefix_ranges(raw);
        assert_eq!(
            ranges,
            vec![
                HeaderPrefix { line: 2, start: 0, end: 8 },
                HeaderPrefix { line: 4, start: 0, end: 6 },
            ],
        );
    }

    #[test]
    fn header_prefix_ranges_ignores_bracket_without_blank_line() {
        let raw = "User voice.\n[Alice]: hi";
        assert!(header_prefix_ranges(raw).is_empty());
    }

    #[test]
    fn header_prefix_ranges_ignores_escaped_header() {
        let raw = "User.\n\n\\[Alice]: hi";
        assert!(header_prefix_ranges(raw).is_empty());
    }

    #[test]
    fn header_prefix_ranges_handles_leading_whitespace() {
        let raw = "User.\n\n   [Alice]: hi";
        let ranges = header_prefix_ranges(raw);
        assert_eq!(
            ranges,
            vec![HeaderPrefix { line: 2, start: 3, end: 11 }],
        );
    }

    #[test]
    fn header_prefix_ranges_ignores_empty_brackets() {
        let raw = "User.\n\n[]: nope";
        assert!(header_prefix_ranges(raw).is_empty());
    }

    #[test]
    fn header_prefix_ranges_supports_utf8_name() {
        let raw = "[Éloïse]: bonjour";
        let ranges = header_prefix_ranges(raw);
        let name_bytes = "Éloïse".len();
        let expected_end = 1 + name_bytes + 2;
        assert_eq!(
            ranges,
            vec![HeaderPrefix { line: 0, start: 0, end: expected_end }],
        );
    }
}
