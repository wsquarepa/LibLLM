use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::session::{NodeId, Role};

use super::App;

const DIALOGUE_COLOR: Color = Color::LightBlue;
const NAV_CURSOR_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Yellow);
const HOVER_BG: Color = Color::Indexed(236);

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

pub fn render_hints_below_dialog(
    f: &mut ratatui::Frame,
    dialog: Rect,
    area: Rect,
    hints: &[Line<'_>],
) {
    if hints.is_empty() {
        return;
    }
    let hint_count = hints.len() as u16;
    let space_below = (area.y + area.height).saturating_sub(dialog.y + dialog.height);
    if space_below < hint_count {
        return;
    }
    let hint_area = Rect {
        x: dialog.x,
        y: dialog.y + dialog.height,
        width: dialog.width,
        height: hint_count,
    };
    let paragraph = Paragraph::new(Text::from(hints.to_vec()))
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(paragraph, hint_area);
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
) -> u16 {
    let char_name = app.session.character.as_deref().unwrap_or("");
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let has_replacements = app.session.character.is_some();

    let replace_vars = |text: &str| -> String {
        if has_replacements {
            super::business::apply_template_vars(text, char_name, user_name)
        } else {
            text.to_owned()
        }
    };

    let user_label = if has_replacements && app.active_persona_name.is_some() {
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
    let mut hover_height_start: Option<u16> = None;
    let mut hover_height_end: Option<u16> = None;
    let mut cumulative_height: u16 = 0;
    let static_height = cached
        .entries
        .iter()
        .map(|entry| entry.total_height)
        .sum::<u16>();

    for (entry, &node_id) in cached.entries.iter().zip(branch_ids.iter()) {
        let is_nav_selected = app.nav_cursor == Some(node_id);
        let is_hovered = !is_nav_selected && app.hover_node == Some(node_id);
        if is_nav_selected {
            nav_cursor_line = Some(lines.len());
        }
        if is_hovered {
            hover_height_start = Some(cumulative_height);
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

        cumulative_height += entry.total_height;

        if is_nav_selected {
            nav_cursor_end = Some(lines.len());
        }
        if is_hovered {
            hover_height_end = Some(cumulative_height);
        }
    }

    if app.is_streaming && !app.streaming_buffer.is_empty() {
        if app.is_continuation {
            if lines.last().is_some_and(|l| l.spans.is_empty()) {
                lines.pop();
            }
        } else {
            lines.push(Line::from(vec![Span::styled(
                format!("{assistant_label}: "),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )]));
        }
        let buffer = replace_vars(&app.streaming_buffer);
        for content_line in buffer.lines() {
            let styled = parse_styled_line(content_line);
            let mut indented = vec![Span::raw("  ")];
            indented.extend(styled.spans);
            lines.push(Line::from(indented));
        }
    }

    let visible_height = area.height.saturating_sub(2);
    let streaming_height = if app.is_streaming && !app.streaming_buffer.is_empty() {
        measure_wrapped_height(&lines, area).saturating_sub(static_height)
    } else {
        0
    };
    let content_height = static_height + streaming_height;
    let max_scroll = content_height.saturating_sub(visible_height);

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
                    crate::debug_log::log_kv(
                        "chat.measure",
                        &[
                            crate::debug_log::field("content_height", content_height),
                            crate::debug_log::field("visible_height", visible_height),
                        ],
                    );

                    *chat_scroll = max_scroll;
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

    *chat_scroll = (*chat_scroll).min(max_scroll);

    let chat_focused = app.focus == super::Focus::Chat;
    let mut chat_block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .border_style(border_style(chat_focused));
    if app.is_streaming {
        chat_block = chat_block.title_bottom(
            Line::from(Span::styled(
                " Generating... Esc to cancel ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
            .centered(),
        );
    } else if chat_focused {
        chat_block = chat_block
            .title_bottom(Line::from(" Up/Down: navigate, Left/Right: branch ").centered());
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((*chat_scroll, 0));

    f.render_widget(paragraph, area);

    if let (Some(h_start), Some(h_end)) = (hover_height_start, hover_height_end) {
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_w = area.width.saturating_sub(2);
        let inner_h = area.height.saturating_sub(2);
        let scroll = *chat_scroll;

        let vis_start = h_start.saturating_sub(scroll);
        let vis_end = h_end.saturating_sub(scroll).min(inner_h);

        if vis_start < vis_end {
            let buf = f.buffer_mut();
            for row in vis_start..vis_end {
                let y = inner_y + row;
                for col in 0..inner_w {
                    let x = inner_x + col;
                    let cell = &mut buf[(x, y)];
                    cell.set_style(cell.style().bg(HOVER_BG));
                }
            }
        }
    }

    max_scroll
}

fn queue_user_label(app: &App) -> String {
    let has_replacements = app.session.character.is_some();
    if has_replacements && app.active_persona_name.is_some() {
        app.active_persona_name.as_deref().unwrap_or("User").to_owned()
    } else {
        "You".to_owned()
    }
}

fn build_queue_lines(queue: &[String], user_label: &str) -> Vec<Line<'static>> {
    let dim_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (idx, msg) in queue.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            format!("{user_label}:"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM),
        )]));
        for content_line in msg.lines() {
            let styled = parse_styled_line(content_line);
            let mut indented: Vec<Span<'static>> = vec![Span::raw("  ")];
            for span in styled.spans {
                let merged = span.style.patch(dim_style);
                indented.push(Span::styled(span.content.into_owned(), merged));
            }
            lines.push(Line::from(indented));
        }
    }
    lines
}

pub fn split_chat_area_for_queue(chat_area: Rect, app: &App) -> (Rect, Option<Rect>) {
    if app.message_queue.is_empty() || chat_area.height < 8 {
        return (chat_area, None);
    }

    let user_label = queue_user_label(app);
    let queue_lines = build_queue_lines(&app.message_queue, &user_label);
    let content_rows = measure_wrapped_height(&queue_lines, chat_area);
    let desired = content_rows.saturating_add(2);

    let max_queue_height = (chat_area.height / 2).max(3);
    let queue_height = desired.min(max_queue_height).max(3);

    if queue_height + 5 > chat_area.height {
        return (chat_area, None);
    }

    let messages_height = chat_area.height - queue_height;
    let messages_area = Rect {
        x: chat_area.x,
        y: chat_area.y,
        width: chat_area.width,
        height: messages_height,
    };
    let queue_area = Rect {
        x: chat_area.x,
        y: chat_area.y + messages_height,
        width: chat_area.width,
        height: queue_height,
    };
    (messages_area, Some(queue_area))
}

pub fn render_message_queue(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let user_label = queue_user_label(app);
    let lines = build_queue_lines(&app.message_queue, &user_label);
    let title = format!(" Queued ({}) ", app.message_queue.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

pub fn render_command_picker(f: &mut ratatui::Frame, app: &App, prefix: &str, chat_area: Rect) {
    let hidden: &[&str] = if crate::config::load().debug_log {
        &[]
    } else {
        &["/report"]
    };
    let matches =
        crate::commands::matching_commands(prefix.split_whitespace().next().unwrap_or("/"), hidden);
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
    let bg_style = Style::default().fg(Color::White).bg(Color::DarkGray);

    if let Some(msg) = &app.status_message {
        if matches!(msg.level, super::StatusLevel::Error) {
            let style = Style::default().fg(Color::White).bg(Color::Red);
            let paragraph = Paragraph::new(format!(" {} ", msg.text))
                .style(style)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, area);
            return;
        }
    }

    let branch_text = match branch_info {
        Some((idx, total)) => format!("Branch {}/{total}", idx + 1),
        None => "Linear".to_owned(),
    };

    let worldbook_text = if app.session.character.is_some() {
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

    let display_name = app.model_name.as_deref().unwrap_or("connecting...");
    let left_text = format!(
        " {} | {} | ~{} tokens | {}{}",
        display_name, app.instruct_preset.name, token_count, branch_text, worldbook_text,
    );

    let left_style = if !app.api_available {
        Style::default().fg(Color::Red).bg(Color::DarkGray)
    } else {
        bg_style
    };

    let hints_text = "Tab: switch focus | Ctrl+C: quit ";

    let total_width = area.width as usize;
    if total_width < 20 {
        let paragraph = Paragraph::new(left_text).style(left_style);
        f.render_widget(paragraph, area);
        return;
    }

    let notification = app.status_message.as_ref().map(|msg| {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(msg.created);
        let remaining = msg.expires.saturating_duration_since(now);
        let slide_dur = super::NOTIFICATION_SLIDE_DURATION.as_secs_f64();

        let progress = if elapsed.as_secs_f64() < slide_dur {
            elapsed.as_secs_f64() / slide_dur
        } else if remaining.as_secs_f64() < slide_dur {
            remaining.as_secs_f64() / slide_dur
        } else {
            1.0
        };

        let (fg, bg) = match msg.level {
            super::StatusLevel::Info => (Color::White, Color::Blue),
            super::StatusLevel::Warning => (Color::Black, Color::Yellow),
            super::StatusLevel::Error => unreachable!(),
        };

        (msg.text.as_str(), fg, bg, progress)
    });

    let right_spans = build_right_spans(hints_text, notification, total_width);
    let right_width: usize = right_spans.iter().map(|s| s.content.len()).sum();

    let left_max = total_width.saturating_sub(right_width).saturating_sub(1);
    let truncated_left = truncate_str(&left_text, left_max);

    let left_area = Rect::new(area.x, area.y, left_max as u16, 1);
    let right_area = Rect::new(
        area.x + (total_width - right_width) as u16,
        area.y,
        right_width as u16,
        1,
    );

    f.render_widget(Paragraph::new("").style(bg_style), area);
    f.render_widget(Paragraph::new(truncated_left).style(left_style), left_area);
    f.render_widget(
        Paragraph::new(Line::from(right_spans)).style(bg_style),
        right_area,
    );
}

fn build_right_spans<'a>(
    hints: &'a str,
    notification: Option<(&'a str, Color, Color, f64)>,
    max_width: usize,
) -> Vec<Span<'a>> {
    let Some((text, fg, bg, progress)) = notification else {
        return vec![Span::styled(
            hints,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )];
    };

    let padded = format!(" {} ", text);
    let notif_full_width = padded.len();
    let visible_width = ((progress * notif_full_width as f64).round() as usize).min(max_width);

    if visible_width == 0 {
        return vec![Span::styled(
            hints,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )];
    }

    let hints_width = max_width.saturating_sub(visible_width);
    let visible_hints = truncate_str(hints, hints_width);

    let visible_text: String = if visible_width >= padded.len() {
        format!("{:width$}", padded, width = visible_width)
    } else {
        padded[..padded.floor_char_boundary(visible_width)].to_owned()
    };

    let notif_style = Style::default().fg(fg).bg(bg);
    let mut spans = Vec::new();

    if !visible_hints.is_empty() {
        spans.push(Span::styled(
            visible_hints,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ));
    }

    spans.push(Span::styled(visible_text, notif_style));

    spans
}

fn truncate_str(s: &str, max_len: usize) -> String {
    s[..s.floor_char_boundary(max_len)].to_owned()
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

// -----------------------------------------------------------------------
// This MUST use Paragraph::line_count() -- not a manual width calculation.
// Ratatui's WordWrapper breaks at word boundaries and measures unicode
// display width, both of which differ from ceil(byte_len / columns).
// A naive approximation underestimates height, and the error accumulates
// across messages, causing auto-scroll to miss the actual bottom of text.
// -----------------------------------------------------------------------
fn wrapped_line_height(line: &Line, area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2).max(1);
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner_width) as u16
}

pub fn hit_test_chat_message(
    cache: &ChatContentCache,
    branch_ids: &[NodeId],
    chat_area: Rect,
    chat_scroll: u16,
    screen_row: u16,
) -> Option<NodeId> {
    let inner_top = chat_area.y + 1;
    let inner_bottom = chat_area.y + chat_area.height.saturating_sub(1);
    if screen_row < inner_top || screen_row >= inner_bottom {
        return None;
    }
    let content_row = (screen_row - inner_top) as u32 + chat_scroll as u32;
    let mut cumulative: u32 = 0;
    for (i, entry) in cache.entries.iter().enumerate() {
        cumulative += entry.total_height as u32;
        if content_row < cumulative {
            return branch_ids.get(i).copied();
        }
    }
    None
}
