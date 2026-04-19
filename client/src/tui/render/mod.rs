//! TUI rendering: chat messages, sidebar, dialogs, and status bar.

mod measurement;
mod status_bar;
mod text;

pub use measurement::hit_test_chat_message;
use measurement::{measure_wrapped_height, measure_wrapped_offset, wrapped_line_height};
pub use status_bar::render_status_bar;
use text::parse_styled_line;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use libllm::session::{NodeId, Role};

use super::App;

use super::theme::Theme;

pub struct SidebarCache {
    selected_idx: Option<usize>,
    filter_query: String,
    filter_active: bool,
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

pub struct ChatRenderState<'a> {
    pub chat_scroll: &'a mut u16,
    pub scroll_dirty: bool,
    pub cache: &'a mut Option<ChatContentCache>,
}

pub fn border_style(focused: bool, theme: &Theme) -> Style {
    if focused {
        Style::default().fg(theme.border_focused)
    } else {
        Style::default().fg(theme.border_unfocused)
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
    f.render_widget(ratatui::widgets::Clear, hint_area);
    let paragraph = Paragraph::new(Text::from(hints.to_vec()))
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(paragraph, hint_area);
}

pub fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let selected_idx = app.sidebar_state.selected();
    let filter_query = app.sidebar_search.query.clone();
    let filter_active = app.sidebar_search.active;

    let cache_valid = app.sidebar_cache.as_ref().is_some_and(|cache| {
        cache.selected_idx == selected_idx
            && cache.filter_query == filter_query
            && cache.filter_active == filter_active
    });

    if !cache_valid {
        let display_names: Vec<String> = app
            .sidebar_sessions
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let visible_indices: Vec<usize> = if app.sidebar_search.is_filtering() {
            (0..app.sidebar_sessions.len())
                .filter(|&i| {
                    app.sidebar_sessions[i].is_new_chat
                        || app.sidebar_search.matches(&display_names[i])
                })
                .collect()
        } else {
            (0..app.sidebar_sessions.len()).collect()
        };

        let items: Vec<ListItem<'static>> = visible_indices
            .iter()
            .map(|&i| {
                let entry = &app.sidebar_sessions[i];
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
            filter_query: filter_query.clone(),
            filter_active,
            items,
        });
    }

    let highlight_style = if app.focus == super::Focus::Sidebar {
        Style::default()
            .fg(app.theme.sidebar_highlight_fg)
            .bg(app.theme.sidebar_highlight_bg)
    } else {
        Style::default().fg(app.theme.sidebar_highlight_bg)
    };

    let sidebar_focused = app.focus == super::Focus::Sidebar;
    let title_color = if sidebar_focused {
        app.theme.border_focused
    } else {
        app.theme.border_unfocused
    };
    let mut sidebar_block = Block::default()
        .borders(Borders::ALL)
        .title(" Sessions ")
        .border_style(border_style(sidebar_focused, &app.theme));
    let search_visible = app.sidebar_search.active || app.sidebar_search.is_filtering();
    if search_visible || !sidebar_focused {
        let search_max = area.width.saturating_sub(2);
        sidebar_block = sidebar_block.title_bottom(super::dialogs::search_title_line(
            &app.sidebar_search,
            title_color,
            &app.theme,
            search_max,
        ));
    }
    if sidebar_focused && !search_visible {
        sidebar_block =
            sidebar_block.title_bottom(Line::from(" Del: delete  Ctrl+F: search ").right_aligned());
    }

    let list_area = sidebar_block.inner(area);
    f.render_widget(sidebar_block, area);

    let list = List::new(app.sidebar_cache.as_ref().unwrap().items.clone())
        .highlight_style(highlight_style)
        .highlight_symbol("> ");

    let mut local_state = ListState::default();
    if let Some(orig_idx) = selected_idx {
        let display_names: Vec<String> = app
            .sidebar_sessions
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let visible_indices: Vec<usize> = if app.sidebar_search.is_filtering() {
            (0..app.sidebar_sessions.len())
                .filter(|&i| {
                    app.sidebar_sessions[i].is_new_chat
                        || app.sidebar_search.matches(&display_names[i])
                })
                .collect()
        } else {
            (0..app.sidebar_sessions.len()).collect()
        };
        if let Some(pos) = visible_indices.iter().position(|&i| i == orig_idx) {
            local_state.select(Some(pos));
        }
    }

    f.render_stateful_widget(list, list_area, &mut local_state);
}

pub fn render_chat(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    branch_ids: &[NodeId],
    token_count: usize,
    state: ChatRenderState<'_>,
) -> u16 {
    let ChatRenderState {
        chat_scroll,
        scroll_dirty,
        cache,
    } = state;
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
    let worldbook_count = super::business::enabled_worldbook_names(app.session, &app.config).len();

    let cache_valid = cache.as_ref().is_some_and(|c| {
        c.branch_ids == branch_ids
            && c.char_name == char_name
            && c.user_name == user_name
            && c.width == area.width
    });

    if !cache_valid {
        tracing::trace!(
            result = "miss",
            action = "rebuild",
            message_count = branch_ids.len(),
            width = area.width,
            "chat.cache",
        );
        let entries: Vec<CachedMessageLines> = branch_ids
            .iter()
            .enumerate()
            .map(|(idx, &node_id)| {
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
                            .fg(app.theme.user_message)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Role::Assistant => (
                        assistant_label.clone(),
                        Style::default()
                            .fg(app.theme.assistant_message_fg)
                            .bg(app.theme.assistant_message_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Role::System => (
                        "System".to_owned(),
                        Style::default()
                            .fg(app.theme.system_message)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Role::Summary => (
                        "Summary".to_owned(),
                        Style::default()
                            .fg(app.theme.summary_indicator)
                            .add_modifier(Modifier::BOLD),
                    ),
                };

                let (sib_idx, sib_total) = app.session.tree.sibling_info(node_id);
                let branch_indicator = if sib_total > 1 {
                    format!(" [{}/{}]", sib_idx + 1, sib_total)
                } else {
                    String::new()
                };

                let (content_lines, total_height) = if msg.role == Role::Summary {
                    let msg_count = idx;
                    let summary_line = format!("--- Summary of {} earlier messages ---", msg_count);
                    let lines = vec![Line::from(Span::styled(
                        format!("  {summary_line}"),
                        Style::default()
                            .fg(app.theme.summary_indicator)
                            .add_modifier(Modifier::DIM),
                    ))];
                    (lines, 2u16)
                } else {
                    let content = replace_vars(&msg.content);
                    let dialogue_color = app.theme.dialogue;
                    let lines: Vec<Line<'static>> = content
                        .lines()
                        .map(|line| {
                            let styled = parse_styled_line(line, dialogue_color);
                            let mut indented = vec![Span::raw("  ")];
                            indented.extend(styled.spans);
                            Line::from(indented)
                        })
                        .collect();
                    let height = lines
                        .iter()
                        .map(|line| wrapped_line_height(line, area))
                        .sum::<u16>()
                        + 2;
                    (lines, height)
                };

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
        tracing::trace!(
            result = "hit",
            message_count = branch_ids.len(),
            width = area.width,
            "chat.cache",
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
            Style::default()
                .fg(app.theme.nav_cursor_fg)
                .bg(app.theme.nav_cursor_bg)
                .add_modifier(Modifier::BOLD)
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
                    .fg(app.theme.assistant_message_fg)
                    .bg(app.theme.assistant_message_bg)
                    .add_modifier(Modifier::BOLD),
            )]));
        }
        let buffer = replace_vars(&app.streaming_buffer);
        for content_line in buffer.lines() {
            let styled = parse_styled_line(content_line, app.theme.dialogue);
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
        {
            let _span = tracing::trace_span!(
                "scroll",
                phase = "adjust",
                dirty = scroll_dirty,
                auto = app.auto_scroll
            )
            .entered();
            if app.auto_scroll {
                tracing::trace!(content_height, visible_height, "chat.measure");
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
        }
        tracing::trace!(phase = "value", value = *chat_scroll, "scroll");
    }

    *chat_scroll = (*chat_scroll).min(max_scroll);

    let chat_focused = app.focus == super::Focus::Chat;
    let mut chat_block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .title(
            Line::from(Span::styled(
                format!(" {assistant_label} "),
                Style::default()
                    .fg(app.theme.assistant_message_fg)
                    .add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        )
        .border_style(border_style(chat_focused, &app.theme));
    if app.is_streaming {
        chat_block = chat_block.title_bottom(
            Line::from(Span::styled(
                " Generating... Esc to cancel ",
                Style::default()
                    .fg(app.theme.streaming_indicator)
                    .add_modifier(Modifier::BOLD),
            ))
            .centered(),
        );
    } else if app.is_summarizing {
        chat_block = chat_block.title_bottom(
            Line::from(Span::styled(
                " Summarizing... ",
                Style::default()
                    .fg(app.theme.streaming_indicator)
                    .add_modifier(Modifier::BOLD),
            ))
            .centered(),
        );
    } else if chat_focused {
        chat_block = chat_block
            .title_bottom(Line::from(" Up/Down: navigate, Left/Right: branch ").centered());
    } else {
        let worldbook_label = format_count(worldbook_count, "worldbook");
        let model_label =
            truncate_with_ellipsis(app.model_name.as_deref().unwrap_or("connecting..."), 50);
        let model_style = if app.api_available {
            Style::default().fg(app.theme.status_bar_fg)
        } else {
            Style::default().fg(app.theme.api_unavailable)
        };
        let token_label = format_count(token_count, "token");

        chat_block = chat_block
            .title_bottom(Line::from(Span::styled(
                format!(" {worldbook_label} "),
                Style::default().fg(app.theme.dimmed),
            )))
            .title_bottom(
                Line::from(Span::styled(format!(" {model_label} "), model_style)).centered(),
            )
            .title_bottom(
                Line::from(Span::styled(
                    format!(" {token_label} "),
                    Style::default().fg(app.theme.dimmed),
                ))
                .right_aligned(),
            );
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
                    cell.set_style(cell.style().bg(app.theme.hover_bg));
                }
            }
        }
    }

    max_scroll
}

fn format_count(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }

    let visible_chars = max_chars - 3;
    let end = text
        .char_indices()
        .nth(visible_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    format!("{}...", &text[..end])
}

fn queue_user_label(app: &App) -> String {
    let has_replacements = app.session.character.is_some();
    if has_replacements && app.active_persona_name.is_some() {
        app.active_persona_name
            .as_deref()
            .unwrap_or("User")
            .to_owned()
    } else {
        "You".to_owned()
    }
}

fn build_queue_lines(queue: &[String], user_label: &str, theme: &Theme) -> Vec<Line<'static>> {
    let dim_style = Style::default()
        .fg(theme.dimmed)
        .add_modifier(Modifier::ITALIC);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (idx, msg) in queue.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            format!("{user_label}:"),
            Style::default()
                .fg(theme.user_message)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM),
        )]));
        for content_line in msg.lines() {
            let styled = parse_styled_line(content_line, theme.dialogue);
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
    let queue_lines = build_queue_lines(&app.message_queue, &user_label, &app.theme);
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
    let lines = build_queue_lines(&app.message_queue, &user_label, &app.theme);
    let title = format!(" Queued ({}) ", app.message_queue.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(app.theme.border_unfocused));
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

pub fn render_command_picker(f: &mut ratatui::Frame, app: &App, prefix: &str, chat_area: Rect) {
    let hidden: &[&str] = &[];
    let matches = libllm::commands::matching_commands(
        prefix.split_whitespace().next().unwrap_or("/"),
        hidden,
    );
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
        .block(dialog_block(" Commands ", app.theme.command_picker_bg))
        .highlight_style(
            Style::default()
                .fg(app.theme.command_picker_fg)
                .bg(app.theme.command_picker_bg),
        );

    f.render_widget(ratatui::widgets::Clear, picker_area);
    f.render_stateful_widget(list, picker_area, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_count_uses_singular_for_one() {
        assert_eq!(format_count(1, "token"), "1 token");
    }

    #[test]
    fn format_count_uses_plural_for_zero_and_many() {
        assert_eq!(format_count(0, "worldbook"), "0 worldbooks");
        assert_eq!(format_count(2, "worldbook"), "2 worldbooks");
    }

    #[test]
    fn truncate_with_ellipsis_leaves_short_text_unchanged() {
        assert_eq!(truncate_with_ellipsis("model", 10), "model");
    }

    #[test]
    fn truncate_with_ellipsis_caps_long_text() {
        assert_eq!(
            truncate_with_ellipsis("abcdefghijklmnopqrstuvwxyz", 10),
            "abcdefg..."
        );
    }

    #[test]
    fn truncate_with_ellipsis_skips_suffix_for_tiny_limit() {
        assert_eq!(truncate_with_ellipsis("abcdef", 3), "abc");
    }
}
