//! Markdown-style inline formatting parser for bold, italic, dialogue, and file-reference spans.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub(super) fn parse_styled_line(
    text: &str,
    dialogue_color: Color,
    file_reference_color: Color,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut plain_start = 0;

    while let Some(&(i, ch)) = chars.peek() {
        match ch {
            '*' => {
                let star_start = i;
                chars.next();
                let is_bold = chars.peek().is_some_and(|&(_, c)| c == '*');
                if is_bold {
                    chars.next();
                    let content_start = star_start + 2;
                    let close = find_closing(&text[content_start..], "**");
                    if let Some(rel_end) = close {
                        if plain_start < star_start {
                            spans.push(Span::raw(text[plain_start..star_start].to_owned()));
                        }
                        let abs_end = content_start + rel_end;
                        spans.push(Span::styled(
                            text[content_start..abs_end].to_owned(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                        let skip_to = abs_end + 2;
                        while chars.peek().is_some_and(|&(idx, _)| idx < skip_to) {
                            chars.next();
                        }
                        plain_start = skip_to;
                    }
                } else {
                    let content_start = star_start + 1;
                    let close = find_closing(&text[content_start..], "*");
                    if let Some(rel_end) = close {
                        if plain_start < star_start {
                            spans.push(Span::raw(text[plain_start..star_start].to_owned()));
                        }
                        let abs_end = content_start + rel_end;
                        spans.push(Span::styled(
                            text[content_start..abs_end].to_owned(),
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                        let skip_to = abs_end + 1;
                        while chars.peek().is_some_and(|&(idx, _)| idx < skip_to) {
                            chars.next();
                        }
                        plain_start = skip_to;
                    }
                }
            }
            '"' => {
                let quote_start = i;
                chars.next();
                let content_start = quote_start + 1;
                let close = find_closing(&text[content_start..], "\"");
                if let Some(rel_end) = close {
                    if plain_start < quote_start {
                        spans.push(Span::raw(text[plain_start..quote_start].to_owned()));
                    }
                    let abs_end = content_start + rel_end;
                    spans.push(Span::styled(
                        text[quote_start..abs_end + 1].to_owned(),
                        Style::default().fg(dialogue_color),
                    ));
                    let skip_to = abs_end + 1;
                    while chars.peek().is_some_and(|&(idx, _)| idx < skip_to) {
                        chars.next();
                    }
                    plain_start = skip_to;
                }
            }
            '@' => {
                let at_start = i;
                let is_word_boundary = at_start == 0
                    || text
                        .as_bytes()
                        .get(at_start.saturating_sub(1))
                        .is_some_and(|b| b.is_ascii_whitespace());
                if !is_word_boundary {
                    chars.next();
                    continue;
                }
                chars.next();
                let is_quoted = chars.peek().is_some_and(|&(_, c)| c == '"');
                let end_opt = if is_quoted {
                    chars.next();
                    let content_start = at_start + 2;
                    text[content_start..].find('"').map(|rel| {
                        let close_abs = content_start + rel;
                        let skip_to = close_abs + 1;
                        while chars.peek().is_some_and(|&(idx, _)| idx < skip_to) {
                            chars.next();
                        }
                        skip_to
                    })
                } else {
                    let mut end = at_start + 1;
                    while let Some(&(j, next_ch)) = chars.peek() {
                        if next_ch.is_whitespace() {
                            end = j;
                            break;
                        }
                        chars.next();
                        end = j + next_ch.len_utf8();
                    }
                    Some(end)
                };
                if let Some(end) = end_opt
                    && end > at_start + 1
                {
                    if plain_start < at_start {
                        spans.push(Span::raw(text[plain_start..at_start].to_owned()));
                    }
                    spans.push(Span::styled(
                        text[at_start..end].to_owned(),
                        Style::default().fg(file_reference_color),
                    ));
                    plain_start = end;
                }
            }
            _ => {
                chars.next();
            }
        }
    }

    if plain_start < text.len() {
        spans.push(Span::raw(text[plain_start..].to_owned()));
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

fn find_closing(text: &str, delimiter: &str) -> Option<usize> {
    let start = text.char_indices().nth(1).map(|(i, _)| i)?;
    text[start..].find(delimiter).map(|pos| pos + start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_token_at_start_is_styled() {
        let line = parse_styled_line("@notes.md please", Color::Red, Color::Blue);
        let styled = line.spans.iter().find(|s| s.content.starts_with("@"));
        let styled = styled.expect("expected a styled @ span");
        assert_eq!(styled.content, "@notes.md");
        assert_eq!(styled.style.fg, Some(Color::Blue));
    }

    #[test]
    fn at_token_mid_word_is_plain() {
        let line = parse_styled_line("email@example.com", Color::Red, Color::Blue);
        for span in &line.spans {
            assert_ne!(span.style.fg, Some(Color::Blue), "mid-word @ must not be styled");
        }
    }

    #[test]
    fn at_token_after_whitespace_is_styled() {
        let line = parse_styled_line("see @a.md and @b.md", Color::Red, Color::Blue);
        let styled_count = line
            .spans
            .iter()
            .filter(|s| s.style.fg == Some(Color::Blue))
            .count();
        assert_eq!(styled_count, 2);
    }

    #[test]
    fn bare_at_with_whitespace_after_is_plain() {
        let line = parse_styled_line("say @ hello", Color::Red, Color::Blue);
        for span in &line.spans {
            assert_ne!(span.style.fg, Some(Color::Blue), "bare @ must not style");
        }
    }

    #[test]
    fn existing_italic_still_works() {
        let line = parse_styled_line("plain *italic*", Color::Red, Color::Blue);
        let italic = line
            .spans
            .iter()
            .find(|s| s.style.add_modifier.contains(Modifier::ITALIC));
        assert!(italic.is_some(), "italic parsing must still work");
    }

    #[test]
    fn existing_dialogue_still_works() {
        let line = parse_styled_line(r#"he said "hello" loudly"#, Color::Red, Color::Blue);
        let dialogue = line.spans.iter().find(|s| s.style.fg == Some(Color::Red));
        assert!(dialogue.is_some(), "dialogue parsing must still work");
    }

    #[test]
    fn quoted_at_token_captures_spaces_in_styled_span() {
        let line = parse_styled_line(
            r#"read @"Lecture 29 notes.pdf" now"#,
            Color::Red,
            Color::Blue,
        );
        let styled = line
            .spans
            .iter()
            .find(|s| s.style.fg == Some(Color::Blue))
            .expect("expected a styled @-token span");
        assert_eq!(styled.content, r#"@"Lecture 29 notes.pdf""#);
    }

    #[test]
    fn quoted_at_token_does_not_trigger_dialogue_style() {
        // The quoted @-token opens with `@"` — the dialogue parser
        // would otherwise want to style the inner `"`. Confirm no
        // span gets the dialogue colour.
        let line = parse_styled_line(
            r#"read @"Lecture 29 notes.pdf""#,
            Color::Red,
            Color::Blue,
        );
        for span in &line.spans {
            assert_ne!(
                span.style.fg,
                Some(Color::Red),
                "content inside the @-quoted token must not use dialogue colour",
            );
        }
    }
}
