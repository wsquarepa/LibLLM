use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::context::ContextManager;
use crate::session::{Message, NodeId, Role};

use super::App;

const DIALOGUE_COLOR: Color = Color::LightBlue;
const SIDEBAR_PREVIEW_CHARS: usize = 28;

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
    let selected_idx = app.sidebar_state.selected();
    let mut name_totals: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for entry in &app.sidebar_sessions {
        if !entry.is_new_chat {
            *name_totals.entry(&entry.display_name).or_insert(0) += 1;
        }
    }
    let mut name_remaining = name_totals.clone();
    let items: Vec<ListItem> = app
        .sidebar_sessions
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            if entry.is_new_chat {
                return ListItem::new(entry.display_name.clone());
            }
            let rem = name_remaining.get_mut(entry.display_name.as_str()).unwrap();
            let idx = *rem;
            *rem -= 1;
            let count_str = match entry.message_count {
                Some(n) => format!(" ({n})"),
                None => String::new(),
            };
            let label = format!("[{idx}] {}{count_str}", entry.display_name);
            if selected_idx == Some(i) {
                let mut lines = vec![Line::from(label)];
                if let Some(ref msg) = entry.first_message {
                    let truncated: String = msg.chars().take(SIDEBAR_PREVIEW_CHARS).collect();
                    let display = if msg.chars().count() > SIDEBAR_PREVIEW_CHARS {
                        format!("  {truncated}...")
                    } else {
                        format!("  {truncated}")
                    };
                    lines.push(Line::from(Span::styled(
                        display,
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                ListItem::new(Text::from(lines))
            } else {
                ListItem::new(label)
            }
        })
        .collect();

    let highlight_style = if app.focus == super::Focus::Sidebar {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let sidebar_focused = app.focus == super::Focus::Sidebar;
    let sidebar_title = if sidebar_focused {
        " Sessions (Del: delete) "
    } else {
        " Sessions "
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(sidebar_title)
                .border_style(border_style(sidebar_focused)),
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
    let char_name = app.session.character.as_deref().unwrap_or("");
    let user_name = app.user_name.as_deref().unwrap_or("User");
    let has_replacements = app.session.character.is_some();

    let replace_vars = |text: &str| -> String {
        if has_replacements {
            super::business::apply_template_vars(text, char_name, user_name)
        } else {
            text.to_owned()
        }
    };

    let user_label = if has_replacements && app.user_name.is_some() {
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
    let mut nav_cursor_end: Option<usize> = None;

    for (msg, &node_id) in branch_path.iter().zip(branch_ids.iter()) {
        let (role_label, base_role_style) = match msg.role {
            Role::User => (
                user_label.as_str(),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => (
                assistant_label.as_str(),
                Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
            Role::System => (
                "System",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
            ),
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
            base_role_style
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
        if is_nav_selected {
            nav_cursor_end = Some(lines.len());
        }
    }

    if app.is_streaming && !app.streaming_buffer.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("{assistant_label}: "),
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
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
            *chat_scroll = if wrapped_offset <= visible_height {
                0
            } else {
                wrapped_offset
            };
        } else {
            let end_idx = nav_cursor_end.unwrap_or(cursor_line_idx + 1);
            let wrapped_end = measure_wrapped_offset(&lines, end_idx, area);
            if wrapped_end > *chat_scroll + visible_height {
                *chat_scroll = wrapped_offset;
            }
        }
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.focus == super::Focus::Chat {
                    " Chat (Up/Down: navigate, Left/Right: branch) "
                } else {
                    " Chat "
                })
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
    measure_wrapped_height(&lines[..up_to], area)
}

fn measure_wrapped_height(lines: &[Line], area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2).max(1);
    Paragraph::new(Text::from(lines.to_vec()))
        .wrap(Wrap { trim: false })
        .line_count(inner_width) as u16
}
