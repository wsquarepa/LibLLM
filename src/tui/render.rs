use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Widget, Wrap};

use crate::context::ContextManager;
use crate::session::{Message, NodeId, Role};

use super::App;

const DIALOGUE_COLOR: Color = Color::LightBlue;

pub fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .sidebar_sessions
        .iter()
        .map(|entry| {
            if entry.preview.is_empty() {
                ListItem::new(entry.filename.clone())
            } else {
                ListItem::new(format!(
                    "{}: {}",
                    &entry.filename[..entry.filename.len().min(10)],
                    entry.preview
                ))
            }
        })
        .collect();

    let highlight_style = if app.focus == super::Focus::Sidebar {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Sessions ")
                .border_style(border_style(app.focus == super::Focus::Sidebar)),
        )
        .highlight_style(highlight_style)
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut app.sidebar_state);
}

fn parse_styled_line(text: &str) -> Line<'static> {
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
                        Style::default().fg(DIALOGUE_COLOR),
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
    if text.is_empty() {
        return None;
    }
    text[1..].find(delimiter).map(|pos| pos + 1)
}

pub fn render_chat(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    chat_scroll: &mut u16,
    branch_path: &[&Message],
    branch_ids: &[NodeId],
) {
    let cfg = crate::config::load();
    let char_name = app.session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");
    let has_replacements = app.session.character.is_some();

    let replace_vars = |text: &str| -> String {
        if has_replacements {
            text.replace("{{char}}", char_name)
                .replace("{{user}}", user_name)
        } else {
            text.to_owned()
        }
    };

    let user_label = if has_replacements && cfg.user_name.is_some() {
        user_name.to_owned()
    } else {
        "You".to_owned()
    };

    let assistant_label = if has_replacements && !char_name.is_empty() {
        char_name.to_owned()
    } else {
        "Assistant".to_owned()
    };

    let mut lines: Vec<Line> = Vec::new();
    let mut nav_cursor_line: Option<usize> = None;

    for (msg, &node_id) in branch_path.iter().zip(branch_ids.iter()) {
        let (role_label, role_color) = match msg.role {
            Role::User => (user_label.as_str(), Color::Green),
            Role::Assistant => (assistant_label.as_str(), Color::Blue),
            Role::System => ("System", Color::DarkGray),
        };

        let (sib_idx, sib_total) = app.session.tree.sibling_info(node_id);
        let branch_indicator = if sib_total > 1 {
            format!(" [{}/{}]", sib_idx + 1, sib_total)
        } else {
            String::new()
        };

        let is_nav_selected = app.nav_cursor == Some(node_id);
        if is_nav_selected {
            nav_cursor_line = Some(lines.len());
        }
        let nav_marker = if is_nav_selected { ">> " } else { "" };
        let role_style = if is_nav_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(role_color)
                .add_modifier(Modifier::BOLD)
        };

        lines.push(Line::from(vec![Span::styled(
            format!("{nav_marker}{role_label}{branch_indicator}: "),
            role_style,
        )]));

        let content = replace_vars(&msg.content);
        for content_line in content.lines() {
            let styled = parse_styled_line(content_line);
            let mut indented = vec![Span::raw("  ")];
            indented.extend(styled.spans);
            lines.push(Line::from(indented));
        }
        lines.push(Line::from(""));
    }

    if app.is_streaming && !app.streaming_buffer.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("{assistant_label}: "),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]));
        let buffer = replace_vars(&app.streaming_buffer);
        for content_line in buffer.lines() {
            let styled = parse_styled_line(content_line);
            let mut indented = vec![Span::raw("  ")];
            indented.extend(styled.spans);
            lines.push(Line::from(indented));
        }
    }

    let visible_height = area.height.saturating_sub(2);

    if app.auto_scroll {
        let content_height = measure_wrapped_height(&lines, area);

        if content_height > visible_height {
            *chat_scroll = content_height.saturating_sub(visible_height);
        } else {
            *chat_scroll = 0;
        }
    } else if let Some(cursor_line_idx) = nav_cursor_line {
        let wrapped_offset = measure_wrapped_offset(&lines, cursor_line_idx, area);

        if wrapped_offset < *chat_scroll {
            *chat_scroll = wrapped_offset;
        } else if wrapped_offset >= *chat_scroll + visible_height {
            *chat_scroll = wrapped_offset.saturating_sub(visible_height) + 1;
        }
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Chat ")
                .border_style(border_style(app.focus == super::Focus::Chat)),
        )
        .wrap(Wrap { trim: false })
        .scroll((*chat_scroll, 0));

    f.render_widget(paragraph, area);
}

pub fn render_command_picker(f: &mut ratatui::Frame, app: &App, prefix: &str, chat_area: Rect) {
    let matches =
        crate::commands::matching_commands(prefix.split_whitespace().next().unwrap_or("/"));
    if matches.is_empty() {
        return;
    }

    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            let mut label = c.name.to_owned();
            if !c.args.is_empty() {
                label.push_str(&format!(" {}", c.args));
            }
            if !c.aliases.is_empty() {
                label.push_str(&format!(" ({})", c.aliases.join(", ")));
            }
            ListItem::new(format!("{label}  {}", c.description))
        })
        .collect();

    let height = (items.len() as u16 + 2).min(chat_area.height);
    let picker_area = Rect {
        x: chat_area.x,
        y: chat_area.y + chat_area.height - height,
        width: chat_area.width,
        height,
    };

    let selected = app
        .command_picker_selected
        .min(matches.len().saturating_sub(1));
    let mut state = ListState::default();
    state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Commands ")
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow));

    f.render_widget(ratatui::widgets::Clear, picker_area);
    f.render_stateful_widget(list, picker_area, &mut state);
}

pub fn render_status_bar(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    branch_path: &[&Message],
    branch_info: Option<(usize, usize)>,
) {
    let token_count = ContextManager::estimate_message_tokens(branch_path);

    let branch_info = match branch_info {
        Some((idx, total)) => format!("Branch {}/{total}", idx + 1),
        None => "Linear".to_owned(),
    };

    let status = if app.status_message.is_empty() {
        format!(
            " {} | {} | ~{} tokens | {} | Tab: switch focus | Ctrl+C: quit",
            app.model_name,
            app.template.name(),
            token_count,
            branch_info,
        )
    } else {
        format!(" {} ", app.status_message)
    };

    let paragraph =
        Paragraph::new(status).style(Style::default().fg(Color::White).bg(Color::DarkGray));

    f.render_widget(paragraph, area);
}

fn measure_wrapped_offset(lines: &[Line], up_to: usize, area: Rect) -> u16 {
    if up_to == 0 {
        return 0;
    }
    let mut slice = lines[..up_to].to_vec();
    slice.push(Line::from("X"));
    measure_wrapped_height(&slice, area).saturating_sub(1)
}

fn measure_wrapped_height(lines: &[Line], area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2);
    let max_height = (lines.len() as u16).saturating_mul(4).saturating_add(100);
    let measure_area = Rect::new(0, 0, inner_width, max_height);

    let paragraph = Paragraph::new(Text::from(lines.to_vec())).wrap(Wrap { trim: false });

    let mut buf = ratatui::buffer::Buffer::empty(measure_area);
    paragraph.render(measure_area, &mut buf);

    let mut last_non_empty: u16 = 0;
    for y in 0..max_height {
        for x in 0..inner_width {
            let cell = &buf[(x, y)];
            if cell.symbol() != " " {
                last_non_empty = y + 1;
                break;
            }
        }
    }

    last_non_empty
}
