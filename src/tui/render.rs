use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::session::{NodeId, Role};

use super::App;

const DIALOGUE_COLOR: Color = Color::LightBlue;
const NAV_CURSOR_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Yellow);

pub struct SidebarCache {
    selected_idx: Option<usize>,
    items: Vec<ListItem<'static>>,
}

struct CachedMessageLines {
    role_label: String,
    base_role_style: Style,
    branch_indicator: String,
    content_lines: Vec<Line<'static>>,
    total_height: u16,
}

pub struct ChatContentCache {
    branch_ids: Vec<NodeId>,
    char_name: String,
    user_name: String,
    width: u16,
    entries: Vec<CachedMessageLines>,
}

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

pub fn clear_centered(f: &mut ratatui::Frame, width: u16, height: u16, area: Rect) -> Rect {
    let dialog = centered_rect(width, height, area);
    f.render_widget(ratatui::widgets::Clear, dialog);
    dialog
}

pub fn dialog_block(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(color))
}

pub fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let selected_idx = app.sidebar_state.selected();
    let cache_valid = app
        .sidebar_cache
        .as_ref()
        .is_some_and(|cache| cache.selected_idx == selected_idx);

    if !cache_valid {
        let items = app
            .sidebar_sessions
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                if selected_idx == Some(i) {
                    let mut lines = vec![Line::from(entry.sidebar_label.clone())];
                    if let Some(ref preview) = entry.sidebar_preview {
                        lines.push(Line::from(Span::styled(
                            preview.clone(),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    ListItem::new(Text::from(lines))
                } else {
                    ListItem::new(entry.sidebar_label.clone())
                }
            })
            .collect();
        app.sidebar_cache = Some(SidebarCache {
            selected_idx,
            items,
        });
    }

    let highlight_style = if app.focus == super::Focus::Sidebar {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let sidebar_focused = app.focus == super::Focus::Sidebar;
    let mut sidebar_block = Block::default()
        .borders(Borders::ALL)
        .title(" Sessions ")
        .border_style(border_style(sidebar_focused));
    if sidebar_focused {
        sidebar_block = sidebar_block.title_bottom(Line::from(" Del: delete ").centered());
    }

    let list = List::new(app.sidebar_cache.as_ref().unwrap().items.clone())
        .block(sidebar_block)
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
    let start = text.char_indices().nth(1).map(|(i, _)| i)?;
    text[start..].find(delimiter).map(|pos| pos + start)
}

pub fn render_chat(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    chat_scroll: &mut u16,
    branch_ids: &[NodeId],
    scroll_dirty: bool,
    cache: &mut Option<ChatContentCache>,
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

    let cache_valid = cache.as_ref().is_some_and(|c| {
        c.branch_ids == branch_ids
            && c.char_name == char_name
            && c.user_name == user_name
            && c.width == area.width
    });

    if !cache_valid {
        crate::debug_log::log_kv(
            "chat.cache",
            &[
                crate::debug_log::field("result", "miss"),
                crate::debug_log::field("action", "rebuild"),
                crate::debug_log::field("message_count", branch_ids.len()),
                crate::debug_log::field("width", area.width),
            ],
        );
        let entries: Vec<CachedMessageLines> = branch_ids
            .iter()
            .map(|&node_id| {
                let msg = &app
                    .session
                    .tree
                    .node(node_id)
                    .expect("branch id should resolve to a message node")
                    .message;
                let (role_label, base_role_style) = match msg.role {
                    Role::User => (
                        user_label.clone(),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Role::Assistant => (
                        assistant_label.clone(),
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Role::System => (
                        "System".to_owned(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ),
                };

                let (sib_idx, sib_total) = app.session.tree.sibling_info(node_id);
                let branch_indicator = if sib_total > 1 {
                    format!(" [{}/{}]", sib_idx + 1, sib_total)
                } else {
                    String::new()
                };

                let content = replace_vars(&msg.content);
                let content_lines: Vec<Line<'static>> = content
                    .lines()
                    .map(|line| {
                        let styled = parse_styled_line(line);
                        let mut indented = vec![Span::raw("  ")];
                        indented.extend(styled.spans);
                        Line::from(indented)
                    })
                    .collect();

                let total_height = content_lines
                    .iter()
                    .map(|line| wrapped_line_height(line, area))
                    .sum::<u16>()
                    + 2;

                CachedMessageLines {
                    role_label,
                    base_role_style,
                    branch_indicator,
                    content_lines,
                    total_height,
                }
            })
            .collect();

        *cache = Some(ChatContentCache {
            branch_ids: branch_ids.to_vec(),
            char_name: char_name.to_owned(),
            user_name: user_name.to_owned(),
            width: area.width,
            entries,
        });
    } else {
        crate::debug_log::log_kv(
            "chat.cache",
            &[
                crate::debug_log::field("result", "hit"),
                crate::debug_log::field("message_count", branch_ids.len()),
                crate::debug_log::field("width", area.width),
            ],
        );
    }

    let cached = cache.as_ref().unwrap();

    let mut lines: Vec<Line> = Vec::new();
    let mut nav_cursor_line: Option<usize> = None;
    let mut nav_cursor_end: Option<usize> = None;
    let static_height = cached
        .entries
        .iter()
        .map(|entry| entry.total_height)
        .sum::<u16>();

    for (entry, &node_id) in cached.entries.iter().zip(branch_ids.iter()) {
        let is_nav_selected = app.nav_cursor == Some(node_id);
        if is_nav_selected {
            nav_cursor_line = Some(lines.len());
        }

        let nav_marker = if is_nav_selected { ">> " } else { "" };
        let role_style = if is_nav_selected {
            NAV_CURSOR_STYLE.add_modifier(Modifier::BOLD)
        } else {
            entry.base_role_style
        };

        lines.push(Line::from(vec![Span::styled(
            format!(
                "{nav_marker}{}{}: ",
                entry.role_label, entry.branch_indicator
            ),
            role_style,
        )]));

        lines.extend(entry.content_lines.iter().cloned());
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

    if scroll_dirty {
        crate::debug_log::timed_kv(
            "scroll",
            &[
                crate::debug_log::field("phase", "adjust"),
                crate::debug_log::field("dirty", scroll_dirty),
                crate::debug_log::field("auto", app.auto_scroll),
            ],
            || {
                if app.auto_scroll {
                    let streaming_height = if app.is_streaming && !app.streaming_buffer.is_empty() {
                        measure_wrapped_height(&lines, area).saturating_sub(static_height)
                    } else {
                        0
                    };
                    let content_height = static_height + streaming_height;
                    crate::debug_log::log_kv(
                        "chat.measure",
                        &[
                            crate::debug_log::field("content_height", content_height),
                            crate::debug_log::field("visible_height", visible_height),
                        ],
                    );

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
            },
        );
        crate::debug_log::log_kv(
            "scroll",
            &[
                crate::debug_log::field("phase", "value"),
                crate::debug_log::field("value", *chat_scroll),
            ],
        );
    }

    let chat_focused = app.focus == super::Focus::Chat;
    let mut chat_block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .border_style(border_style(chat_focused));
    if chat_focused {
        chat_block = chat_block
            .title_bottom(Line::from(" Up/Down: navigate, Left/Right: branch ").centered());
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(chat_block)
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
        .block(dialog_block(" Commands ", Color::Yellow))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow));

    f.render_widget(ratatui::widgets::Clear, picker_area);
    f.render_stateful_widget(list, picker_area, &mut state);
}

pub fn render_status_bar(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    branch_info: Option<(usize, usize)>,
    token_count: usize,
) {
    let branch_info = match branch_info {
        Some((idx, total)) => format!("Branch {}/{total}", idx + 1),
        None => "Linear".to_owned(),
    };

    let worldbook_info = if app.session.character.is_some() {
        let mut count = app.config.worldbooks.len();
        for name in &app.session.worldbooks {
            if !app.config.worldbooks.contains(name) {
                count += 1;
            }
        }
        format!(" | {count} worldbooks")
    } else {
        String::new()
    };

    let (status, style) = match &app.status_message {
        Some(msg) => {
            let style = match msg.level {
                super::StatusLevel::Info => Style::default().fg(Color::White).bg(Color::Blue),
                super::StatusLevel::Warning => Style::default().fg(Color::Black).bg(Color::Yellow),
                super::StatusLevel::Error => Style::default().fg(Color::White).bg(Color::Red),
            };
            (format!(" {} ", msg.text), style)
        }
        None => {
            let display_name = app.model_name.as_deref().unwrap_or("connecting...");
            let text = format!(
                " {} | {} | ~{} tokens | {}{} | Tab: switch focus | Ctrl+C: quit",
                display_name,
                app.template.name(),
                token_count,
                branch_info,
                worldbook_info,
            );
            (text, Style::default().fg(Color::White).bg(Color::DarkGray))
        }
    };

    let paragraph = Paragraph::new(status).style(style);

    f.render_widget(paragraph, area);
}

fn measure_wrapped_offset(lines: &[Line], up_to: usize, area: Rect) -> u16 {
    if up_to == 0 {
        return 0;
    }
    measure_wrapped_height(&lines[..up_to], area)
}

fn measure_wrapped_height(lines: &[Line], area: Rect) -> u16 {
    lines
        .iter()
        .map(|line| wrapped_line_height(line, area))
        .sum()
}

fn wrapped_line_height(line: &Line, area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    let width: usize = line.spans.iter().map(|s| s.content.len()).sum();
    if width == 0 {
        1
    } else {
        ((width + inner_width - 1) / inner_width) as u16
    }
}
