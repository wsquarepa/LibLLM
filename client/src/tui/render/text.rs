//! Markdown-style inline formatting parser for bold, italic, and dialogue spans.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub(super) fn parse_styled_line(text: &str, dialogue_color: Color) -> Line<'static> {
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
