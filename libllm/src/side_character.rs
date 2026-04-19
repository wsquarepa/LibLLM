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
}
