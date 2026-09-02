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

use libllm_core::session::{NodeId, Role};

use super::App;
use super::input::match_next_candidates;

use super::theme::Theme;

fn thought_summary_label(thought_seconds: Option<u32>) -> String {
    match thought_seconds {
        Some(1) => "(Thought for 1 second)".to_owned(),
        Some(seconds) => format!("(Thought for {seconds} seconds)"),
        None => "(Thought for a moment)".to_owned(),
    }
}

fn focused_chat_hint(app: &App) -> String {
    format!(
        " Up/Down: navigate, {}, Enter: edit, Del: delete ",
        focused_chat_branch_hint(app)
    )
}

fn focused_chat_branch_hint(app: &App) -> &'static str {
    let Some(node_id) = app.nav_cursor else {
        return "Left/Right: branch";
    };
    let Some(node) = app.session.tree.node(node_id) else {
        return "Left/Right: branch";
    };
    match node.message.role {
        Role::User => {
            let (index, total) = app.session.tree.sibling_info(node_id);
            if index + 1 >= total {
                if index == 0 {
                    "Right: retry"
                } else {
                    "Left: branch, Right: retry"
                }
            } else if index == 0 {
                "Right: branch"
            } else {
                "Left/Right: branch"
            }
        }
        Role::Assistant => "Right: retry",
        _ => "Left/Right: branch",
    }
}

fn render_indented_text_lines(
    text: &str,
    base_style: Style,
    dialogue_color: Color,
    file_reference_color: Color,
) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| {
            let styled = parse_styled_line(line, dialogue_color, file_reference_color);
            let mut indented: Vec<Span<'static>> = vec![Span::raw("  ")];
            for span in styled.spans {
                indented.push(Span::styled(
                    span.content.into_owned(),
                    span.style.patch(base_style),
                ));
            }
            Line::from(indented)
        })
        .collect()
}

fn render_assistant_lines(
    content: &str,
    thought_seconds: Option<u32>,
    theme: &Theme,
    preset: Option<&libllm_core::preset::ReasoningPreset>,
    collapse_completed: bool,
    highlight_incomplete_thought: bool,
) -> Vec<Line<'static>> {
    let split = libllm_core::thought::split_first_think_block(content, preset);

    let Some(thought) = split.thought else {
        return render_indented_text_lines(
            content,
            Style::default(),
            theme.dialogue,
            theme.file_reference_fg,
        );
    };

    if !split.closed {
        if highlight_incomplete_thought {
            return render_indented_text_lines(
                thought,
                Style::default().fg(theme.summary_indicator),
                theme.summary_indicator,
                theme.summary_indicator,
            );
        }
        return render_indented_text_lines(
            thought,
            Style::default(),
            theme.dialogue,
            theme.file_reference_fg,
        );
    }

    let tail = split.after.trim_start();

    if !collapse_completed {
        let mut lines = render_indented_text_lines(
            thought,
            Style::default(),
            theme.dialogue,
            theme.file_reference_fg,
        );
        if !tail.is_empty() {
            lines.extend(render_indented_text_lines(
                tail,
                Style::default(),
                theme.dialogue,
                theme.file_reference_fg,
            ));
        }
        return lines;
    }

    let mut lines = vec![Line::from(vec![
        Span::raw("  "),
        Span::styled(
            thought_summary_label(thought_seconds),
            Style::default().fg(theme.summary_indicator),
        ),
    ])];
    if !tail.is_empty() {
        lines.extend(render_indented_text_lines(
            tail,
            Style::default(),
            theme.dialogue,
            theme.file_reference_fg,
        ));
    }
    lines
}

fn format_file_snapshot_block(
    content: &str,
    summary: Option<&libllm_core::files::FileSummary>,
    summarization_enabled: bool,
) -> Vec<String> {
    let basename = libllm_core::files::snapshot_basename(content).unwrap_or_default();
    let inner_size = inner_snapshot_size(content);
    let header = format!("--- File: {basename} ({}) ---", format_bytes(inner_size));

    tracing::trace!(
        basename = %basename,
        summarization_enabled,
        summary_status = ?summary.map(|s| s.status),
        summary_bytes = summary.map(|s| s.summary.len()).unwrap_or(0),
        "tui.render.file_snapshot"
    );

    if !summarization_enabled {
        return vec![header];
    }

    let body_line = match summary {
        Some(s)
            if s.status == libllm_core::files::FileSummaryStatus::Done && !s.summary.is_empty() =>
        {
            format!("Summary: {}", s.summary)
        }
        Some(s) if s.status == libllm_core::files::FileSummaryStatus::Done => {
            "Summary: (empty)".to_owned()
        }
        Some(s) if s.status == libllm_core::files::FileSummaryStatus::Failed => {
            "Summary: (unavailable)".to_owned()
        }
        Some(_) | None => "Summary: (generating...)".to_owned(),
    };

    vec![header, format!("  {body_line}")]
}

/// Byte count of the text sitting between the `<<<FILE name>>>` /
/// `<<<END name>>>` markers, exclusive of the marker lines themselves.
fn inner_snapshot_size(content: &str) -> usize {
    let mut inside = false;
    let mut size: usize = 0;
    let mut seen_any_inner = false;
    for line in content.lines() {
        if line.starts_with("<<<FILE ") && line.ends_with(">>>") {
            inside = true;
            continue;
        }
        if line.starts_with("<<<END ") && line.ends_with(">>>") {
            inside = false;
            continue;
        }
        if inside {
            if seen_any_inner {
                size += 1;
            }
            size += line.len();
            seen_any_inner = true;
        }
    }
    size
}

fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

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
    roster_fingerprint: Vec<String>,
    card_names_fingerprint: Vec<(String, String)>,
    entries: Vec<CachedMessageLines>,
    pub(super) banner_height: u16,
}

pub struct ChatRenderState<'a> {
    pub chat_scroll: &'a mut u16,
    pub scroll_dirty: bool,
    pub cache: &'a mut Option<ChatContentCache>,
}

pub struct TokenDisplayParams {
    pub token_state: libllm_protocol::tokenizer::CountState,
    pub is_heuristic: bool,
    pub budget: usize,
    pub trigger_percent: u8,
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

#[expect(
    clippy::expect_used,
    reason = "the sidebar cache is rebuilt above before it is read"
)]
pub fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let selected_idx = app.sidebar.list_state.selected();
    let filter_query = app.sidebar.search.query.clone();
    let filter_active = app.sidebar.search.active;

    let cache_valid = app.sidebar.cache.as_ref().is_some_and(|cache| {
        cache.selected_idx == selected_idx
            && cache.filter_query == filter_query
            && cache.filter_active == filter_active
    });

    if !cache_valid {
        let display_names: Vec<String> = app
            .sidebar
            .sessions
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let visible_indices: Vec<usize> = if app.sidebar.search.is_filtering() {
            (0..app.sidebar.sessions.len())
                .filter(|&i| {
                    app.sidebar.sessions[i].is_new_chat
                        || app.sidebar.search.matches(&display_names[i])
                })
                .collect()
        } else {
            (0..app.sidebar.sessions.len()).collect()
        };

        let items: Vec<ListItem<'static>> = visible_indices
            .iter()
            .map(|&i| {
                let entry = &app.sidebar.sessions[i];
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
        app.sidebar.cache = Some(SidebarCache {
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
    let search_visible = app.sidebar.search.active || app.sidebar.search.is_filtering();
    if search_visible || !sidebar_focused {
        let search_max = area.width.saturating_sub(2);
        sidebar_block = sidebar_block.title_bottom(super::dialogs::search_title_line(
            &app.sidebar.search,
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

    let list = List::new(
        app.sidebar
            .cache
            .as_ref()
            .expect("sidebar_cache populated above")
            .items
            .clone(),
    )
    .highlight_style(highlight_style)
    .highlight_symbol("> ");

    let mut local_state = ListState::default();
    if let Some(orig_idx) = selected_idx {
        let display_names: Vec<String> = app
            .sidebar
            .sessions
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let visible_indices: Vec<usize> = if app.sidebar.search.is_filtering() {
            (0..app.sidebar.sessions.len())
                .filter(|&i| {
                    app.sidebar.sessions[i].is_new_chat
                        || app.sidebar.search.matches(&display_names[i])
                })
                .collect()
        } else {
            (0..app.sidebar.sessions.len()).collect()
        };
        if let Some(pos) = visible_indices.iter().position(|&i| i == orig_idx) {
            local_state.select(Some(pos));
        }
    }

    f.render_stateful_widget(list, list_area, &mut local_state);
}

#[expect(
    clippy::expect_used,
    reason = "cache is Some: either just assigned in the miss branch or already Some in the hit branch"
)]
pub fn render_chat(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    branch_ids: &[NodeId],
    token_display: TokenDisplayParams,
    state: ChatRenderState<'_>,
) -> u16 {
    let ChatRenderState {
        chat_scroll,
        scroll_dirty,
        cache,
    } = state;
    let TokenDisplayParams {
        token_state,
        is_heuristic,
        budget,
        trigger_percent,
    } = token_display;
    let char_name = app.session.character.as_deref().unwrap_or("");
    let user_name = app.persona.active_name.as_deref().unwrap_or("User");
    let in_group = app.session.characters.len() >= 2;
    let has_replacements = app.session.character.is_some() || !app.session.characters.is_empty();
    let fallback_group_char_name: String = app
        .session
        .characters
        .first()
        .and_then(|c| {
            app.character
                .cards_cache
                .get(&c.slug)
                .map(|card| card.name.clone())
                .or_else(|| Some(c.slug.clone()))
        })
        .unwrap_or_default();

    let resolve_char_name = |speaker: Option<&str>| -> &str {
        if let Some(slug) = speaker
            && let Some(card) = app.character.cards_cache.get(slug)
        {
            return card.name.as_str();
        }
        if in_group {
            fallback_group_char_name.as_str()
        } else {
            char_name
        }
    };

    let replace_vars = |text: &str, speaker: Option<&str>| -> String {
        if has_replacements {
            super::business::apply_template_vars(text, resolve_char_name(speaker), user_name)
        } else {
            text.to_owned()
        }
    };

    let user_label = if has_replacements && app.persona.active_name.is_some() {
        user_name.to_owned()
    } else {
        "You".to_owned()
    };

    let assistant_label = if app.session.characters.len() >= 2 {
        match super::business::predict_next_speaker(app) {
            Some(name) => format!("Next: {name}"),
            None => "Next: (waiting)".to_owned(),
        }
    } else if has_replacements && !char_name.is_empty() {
        char_name.to_owned()
    } else {
        "Assistant".to_owned()
    };
    let worldbook_count = super::business::enabled_worldbook_names(app.session, &app.config).len();

    let roster_fingerprint: Vec<String> = app
        .session
        .characters
        .iter()
        .map(|c| c.slug.clone())
        .collect();

    let card_names_fingerprint: Vec<(String, String)> = app
        .session
        .characters
        .iter()
        .map(|c| {
            (
                c.slug.clone(),
                app.character
                    .cards_cache
                    .get(&c.slug)
                    .map(|card| card.name.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();

    let cache_valid = cache.as_ref().is_some_and(|c| {
        c.branch_ids == branch_ids
            && c.char_name == char_name
            && c.user_name == user_name
            && c.width == area.width
            && c.roster_fingerprint == roster_fingerprint
            && c.card_names_fingerprint == card_names_fingerprint
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
            .filter_map(|(idx, &node_id)| {
                let Some(node) = app.session.tree.node(node_id) else {
                    tracing::debug!(node_id = ?node_id, "tui.render.missing_node");
                    return None;
                };
                let msg = &node.message;
                let side = if msg.role == Role::User && app.session.character.is_some() {
                    libllm_core::side_character::parse_side_character_block(&msg.content)
                } else {
                    None
                };
                let in_group = app.session.characters.len() >= 2;
                let group_speaker = if in_group {
                    msg.speaker.as_deref()
                } else {
                    None
                };

                let (role_label, base_role_style) = match msg.role {
                    Role::User => {
                        let (label, fg, bg) = match &side {
                            Some((name, _)) => (
                                name.clone(),
                                app.theme.side_character_fg,
                                app.theme.side_character_bg,
                            ),
                            None => (
                                user_label.clone(),
                                app.theme.user_character_fg,
                                app.theme.user_character_bg,
                            ),
                        };
                        (
                            label,
                            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                        )
                    }
                    Role::Assistant => {
                        let (label, fg, bg) = if let Some(slug) = group_speaker {
                            let attach_index = app
                                .session
                                .characters
                                .iter()
                                .position(|c| c.slug == slug)
                                .unwrap_or(0);
                            let palette_idx = attach_index % 8;
                            let fg = app.theme.group_character_palette()[palette_idx];
                            let bg = app.theme.group_character_bg_palette()[palette_idx];
                            let display_name = app
                                .character
                                .cards_cache
                                .get(slug)
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| slug.to_owned());
                            (display_name, fg, bg)
                        } else {
                            (
                                assistant_label.clone(),
                                app.theme.assistant_message_fg,
                                app.theme.assistant_message_bg,
                            )
                        };
                        (
                            label,
                            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                        )
                    }
                    Role::System => {
                        let label = if libllm_core::files::is_snapshot(&msg.content) {
                            "File".to_owned()
                        } else {
                            "System".to_owned()
                        };
                        (
                            label,
                            Style::default()
                                .fg(app.theme.system_message)
                                .add_modifier(Modifier::BOLD),
                        )
                    }
                    Role::Summary => (
                        "Summary".to_owned(),
                        Style::default()
                            .fg(app.theme.summary_indicator)
                            .add_modifier(Modifier::BOLD),
                    ),
                };

                let (sib_idx, sib_total) = app.session.tree.sibling_info(node_id);
                let branch_indicator = if msg.role == Role::User && sib_total > 1 {
                    format!(" [{}/{}]", sib_idx + 1, sib_total)
                } else {
                    String::new()
                };

                let content_lines: Vec<Line<'static>> = if msg.role == Role::Summary {
                    let summary_line = format!("--- Summary of {} earlier messages ---", idx);
                    vec![Line::from(Span::styled(
                        format!("  {summary_line}"),
                        Style::default()
                            .fg(app.theme.summary_indicator)
                            .add_modifier(Modifier::DIM),
                    ))]
                } else if msg.role == Role::System && libllm_core::files::is_snapshot(&msg.content)
                {
                    let inner = libllm_core::files::snapshot_inner_text(&msg.content);
                    let hash = libllm_core::files::content_hash_hex(inner.as_bytes());
                    let summary = match (app.save_mode.id(), app.file_summary.summarizer.as_ref()) {
                        (Some(sid), Some(s)) => s.lookup(sid, &hash),
                        _ => None,
                    };
                    let lines = format_file_snapshot_block(
                        &msg.content,
                        summary.as_ref(),
                        app.summarize.enabled,
                    );
                    lines
                        .into_iter()
                        .map(|line| {
                            Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default()
                                    .fg(app.theme.system_message)
                                    .add_modifier(Modifier::DIM),
                            ))
                        })
                        .collect()
                } else {
                    let speaker = msg.speaker.as_deref();
                    let content: String = match &side {
                        Some((_, body)) => replace_vars(body.as_str(), speaker),
                        None => {
                            let raw = app.display_content_for(node_id).unwrap_or(&msg.content);
                            replace_vars(raw, speaker)
                        }
                    };
                    if msg.role == Role::Assistant {
                        let displayed = if let Some(slug) = group_speaker {
                            let display_name = app
                                .character
                                .cards_cache
                                .get(slug)
                                .map(|c| c.name.as_str())
                                .unwrap_or(slug);
                            let prefix = format!("{display_name}: ");
                            content
                                .strip_prefix(prefix.as_str())
                                .map(str::to_owned)
                                .unwrap_or(content)
                        } else {
                            content
                        };
                        render_assistant_lines(
                            &displayed,
                            msg.thought_seconds,
                            &app.theme,
                            app.reasoning_preset.as_ref(),
                            true,
                            false,
                        )
                    } else {
                        render_indented_text_lines(
                            &content,
                            Style::default(),
                            app.theme.dialogue,
                            app.theme.file_reference_fg,
                        )
                    }
                };

                let total_height = content_lines
                    .iter()
                    .map(|line| wrapped_line_height(line, area))
                    .sum::<u16>()
                    + 2;

                Some(CachedMessageLines {
                    role_label,
                    base_role_style,
                    branch_indicator,
                    content_lines,
                    total_height,
                })
            })
            .collect();

        *cache = Some(ChatContentCache {
            branch_ids: branch_ids.to_vec(),
            char_name: char_name.to_owned(),
            user_name: user_name.to_owned(),
            width: area.width,
            roster_fingerprint,
            card_names_fingerprint,
            entries,
            banner_height: 0,
        });
    } else {
        tracing::trace!(
            result = "hit",
            message_count = branch_ids.len(),
            width = area.width,
            "chat.cache",
        );
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut nav_cursor_line: Option<usize> = None;
    let mut nav_cursor_end: Option<usize> = None;
    let mut hover_height_start: Option<u16> = None;
    let mut hover_height_end: Option<u16> = None;

    if let Some(scenario) = app
        .session
        .scenario
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let style = Style::default().fg(app.theme.system_message);
        lines.push(Line::from(Span::styled("─── Scenario ───", style)));
        for paragraph in scenario.split('\n') {
            lines.push(Line::from(Span::styled(format!("  {paragraph}"), style)));
        }
        lines.push(Line::from(""));
    }

    let banner_height = measure_wrapped_height(&lines, area);
    cache.as_mut().expect("cache is Some: either just assigned in the miss branch above or already Some in the hit branch").banner_height = banner_height;
    let cached = cache.as_ref().expect("cache is Some: either just assigned in the miss branch above or already Some in the hit branch");
    let mut cumulative_height: u16 = banner_height;

    let static_height = cached
        .entries
        .iter()
        .map(|entry| entry.total_height)
        .sum::<u16>()
        + banner_height;

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

    if app.streaming.active && !app.streaming.buffer.is_empty() {
        if !app.streaming.is_continuation {
            lines.push(Line::from(vec![Span::styled(
                format!("{assistant_label}: "),
                Style::default()
                    .fg(app.theme.assistant_message_fg)
                    .bg(app.theme.assistant_message_bg)
                    .add_modifier(Modifier::BOLD),
            )]));
        }
        let buffer_after_regex = libllm_core::regex_rules::apply(
            &app.compiled_regex,
            libllm_core::regex_rules::Scope::Display,
            libllm_core::session::Role::Assistant,
            &app.streaming.buffer,
        );
        let streaming_speaker: Option<String> = app
            .session
            .tree
            .head()
            .and_then(|id| app.session.tree.node(id))
            .and_then(|n| n.message.speaker.clone());
        let raw_buffer = replace_vars(&buffer_after_regex, streaming_speaker.as_deref());
        let buffer = if app.streaming.is_continuation {
            raw_buffer
        } else {
            libllm_core::thought::normalize_assistant_content(
                &raw_buffer,
                app.reasoning_preset.as_ref(),
            )
            .into_owned()
        };
        let thought_seconds = libllm_core::thought::measured_thought_seconds(
            app.streaming.started_at,
            app.streaming.first_think_closed_at,
        );
        let stream_lines = render_assistant_lines(
            &buffer,
            thought_seconds,
            &app.theme,
            app.reasoning_preset.as_ref(),
            app.streaming.first_think_closed_at.is_some(),
            true,
        );
        if app.streaming.is_continuation {
            if lines.last().is_some_and(|l| l.spans.is_empty()) {
                lines.pop();
            }
            let mut stream_iter = stream_lines.into_iter();
            if let (Some(last), Some(first_stream)) = (lines.last_mut(), stream_iter.next()) {
                let mut spans = first_stream.spans.into_iter();
                if let Some(first_span) = spans.next()
                    && first_span.content.as_ref() != "  "
                {
                    last.spans.push(first_span);
                }
                last.spans.extend(spans);
            }
            lines.extend(stream_iter);
        } else {
            lines.extend(stream_lines);
        }
    }

    let visible_height = area.height.saturating_sub(2);
    let streaming_height = if app.streaming.active && !app.streaming.buffer.is_empty() {
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
    if app.streaming.active {
        chat_block = chat_block.title_bottom(
            Line::from(Span::styled(
                " Generating... Esc to cancel ",
                Style::default()
                    .fg(app.theme.streaming_indicator)
                    .add_modifier(Modifier::BOLD),
            ))
            .centered(),
        );
    } else if app.summarize.in_progress {
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
        chat_block = chat_block.title_bottom(Line::from(focused_chat_hint(app)).centered());
    } else {
        let worldbook_label = format_count(worldbook_count, "worldbook");
        let model_label =
            truncate_with_ellipsis(app.model_name.as_deref().unwrap_or("connecting..."), 50);
        let model_style = if app.api_available {
            Style::default().fg(app.theme.status_bar_fg)
        } else {
            Style::default().fg(app.theme.api_unavailable)
        };
        let (count, from_estimate) = match token_state {
            libllm_protocol::tokenizer::CountState::Authoritative(n) => (n, false),
            libllm_protocol::tokenizer::CountState::Stale(n) => (n, false),
            libllm_protocol::tokenizer::CountState::Estimated(n) => (n, true),
        };
        let show_est_prefix = from_estimate || is_heuristic;
        let prefix = if show_est_prefix { "Est. " } else { "" };
        let effective_budget = budget.max(1);
        let pct = (count as f64 / effective_budget as f64) * 100.0;
        let token_color = token_band_color(&app.theme, pct, trigger_percent);
        let token_label = format!(" {prefix}{count} tokens ({pct:.0}%) ");

        let session_note = app.session.author_note.as_ref();
        let card_note = app.active_card_author_note.as_ref();
        let note_indicator: Option<String> = match (session_note, card_note) {
            (None, None) => None,
            (s, c) => {
                let total: usize = s.map(|n| estimate_note_tokens(&n.text)).unwrap_or(0)
                    + c.map(|n| estimate_note_tokens(&n.text)).unwrap_or(0);
                let depth_marker = format_depth_marker(s, c);
                Some(format!("note: {total}t @{depth_marker}"))
            }
        };

        chat_block = chat_block.title_bottom(Line::from(Span::styled(
            format!(" {worldbook_label} "),
            Style::default().fg(app.theme.dimmed),
        )));

        if let Some(text) = note_indicator {
            chat_block = chat_block.title_bottom(Line::from(Span::styled(
                format!(" {text} "),
                Style::default().fg(app.theme.dimmed),
            )));
        }

        chat_block = chat_block
            .title_bottom(
                Line::from(Span::styled(format!(" {model_label} "), model_style)).centered(),
            )
            .title_bottom(
                Line::from(Span::styled(token_label, Style::default().fg(token_color)))
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

const TOKEN_BAND_WARN_FRAC: f64 = 0.55;
const TOKEN_BAND_OVER_FRAC: f64 = 0.90;

fn token_band_color(
    theme: &crate::theme::Theme,
    pct_of_context: f64,
    trigger_percent: u8,
) -> ratatui::style::Color {
    let trigger = trigger_percent as f64;
    let warn_at = trigger * TOKEN_BAND_WARN_FRAC;
    let over_at = trigger * TOKEN_BAND_OVER_FRAC;
    if pct_of_context < warn_at {
        theme.token_band_ok
    } else if pct_of_context < over_at {
        theme.token_band_warn
    } else {
        theme.token_band_over
    }
}

fn estimate_note_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    ((chars as f64) * 0.303).ceil() as usize
}

fn format_depth_marker(
    session: Option<&libllm_core::author_note::AuthorNote>,
    card: Option<&libllm_core::author_note::AuthorNote>,
) -> String {
    let session_top = session.is_some_and(|n| n.at_top);
    let card_top = card.is_some_and(|n| n.at_top);
    if session_top || card_top {
        return "top".to_owned();
    }
    match (session, card) {
        (Some(s), Some(c)) if s.depth == c.depth => format!("d{}", s.depth),
        (Some(s), Some(c)) => format!("d{}/d{}", s.depth, c.depth),
        (Some(s), None) => format!("d{}", s.depth),
        (None, Some(c)) => format!("d{}", c.depth),
        (None, None) => unreachable!("format_depth_marker called with both notes None"),
    }
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
    let has_replacements = app.session.character.is_some() || !app.session.characters.is_empty();
    if has_replacements && app.persona.active_name.is_some() {
        app.persona
            .active_name
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
                .fg(theme.user_character_fg)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM),
        )]));
        for content_line in msg.lines() {
            let styled = parse_styled_line(content_line, theme.dialogue, theme.file_reference_fg);
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
    if app.streaming.message_queue.is_empty() || chat_area.height < 8 {
        return (chat_area, None);
    }

    let user_label = queue_user_label(app);
    let queue_lines = build_queue_lines(&app.streaming.message_queue, &user_label, &app.theme);
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
    let lines = build_queue_lines(&app.streaming.message_queue, &user_label, &app.theme);
    let title = format!(" Queued ({}) ", app.streaming.message_queue.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(app.theme.border_unfocused));
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

pub fn render_next_arg_picker(f: &mut ratatui::Frame, app: &App, arg: &str, chat_area: Rect) {
    let cast: Vec<(&str, &str)> = app
        .session
        .characters
        .iter()
        .filter_map(|c| {
            app.character
                .cards_cache
                .get(&c.slug)
                .map(|card| (c.slug.as_str(), card.name.as_str()))
        })
        .collect();
    let matches = match_next_candidates(arg, &cast);
    if matches.is_empty() {
        return;
    }

    let items: Vec<ListItem> = matches
        .iter()
        .map(|name| ListItem::new((*name).to_owned()))
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
        .block(dialog_block(" Characters ", app.theme.command_picker_bg))
        .highlight_style(
            Style::default()
                .fg(app.theme.command_picker_fg)
                .bg(app.theme.command_picker_bg),
        );

    f.render_widget(ratatui::widgets::Clear, picker_area);
    f.render_stateful_widget(list, picker_area, &mut state);
}

pub fn render_command_picker(f: &mut ratatui::Frame, app: &App, prefix: &str, chat_area: Rect) {
    let hidden: &[&str] = &[];
    let matches = libllm_core::commands::matching_commands(
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

    #[test]
    fn token_band_color_picks_ok_below_warn_frac_of_threshold() {
        let theme = crate::theme::Theme::dark();
        // threshold 90; warn at 49.5, over at 81.
        assert_eq!(token_band_color(&theme, 0.0, 90), theme.token_band_ok);
        assert_eq!(token_band_color(&theme, 49.4, 90), theme.token_band_ok);
    }

    #[test]
    fn token_band_color_picks_warn_between_55_and_90_frac_of_threshold() {
        let theme = crate::theme::Theme::dark();
        // threshold 90; warn at 90*0.55=49.5, over at 90*0.90=81.
        assert_eq!(token_band_color(&theme, 50.0, 90), theme.token_band_warn);
        assert_eq!(token_band_color(&theme, 80.0, 90), theme.token_band_warn);
    }

    #[test]
    fn token_band_color_picks_over_at_or_above_90_frac_of_threshold() {
        let theme = crate::theme::Theme::dark();
        assert_eq!(token_band_color(&theme, 82.0, 90), theme.token_band_over);
        assert_eq!(token_band_color(&theme, 200.0, 90), theme.token_band_over);
    }

    #[test]
    fn token_band_color_scales_with_custom_threshold() {
        let theme = crate::theme::Theme::dark();
        // threshold 50; warn at 50*0.55=27.5, over at 50*0.90=45.
        assert_eq!(token_band_color(&theme, 0.0, 50), theme.token_band_ok);
        assert_eq!(token_band_color(&theme, 27.0, 50), theme.token_band_ok);
        assert_eq!(token_band_color(&theme, 28.0, 50), theme.token_band_warn);
        assert_eq!(token_band_color(&theme, 44.0, 50), theme.token_band_warn);
        assert_eq!(token_band_color(&theme, 46.0, 50), theme.token_band_over);
    }

    #[test]
    fn format_file_snapshot_block_shows_done_summary_on_second_line() {
        let body = libllm_core::files::build_snapshot_body("notes.md", "hello");
        let summary = libllm_core::files::FileSummary {
            basename: "notes.md".to_owned(),
            summary: "Cached summary.".to_owned(),
            status: libllm_core::files::FileSummaryStatus::Done,
        };
        let lines = format_file_snapshot_block(&body, Some(&summary), true);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("--- File: notes.md ("));
        assert!(lines[1].contains("Summary: Cached summary."));
    }

    #[test]
    fn format_file_snapshot_block_shows_generating_for_pending() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello");
        let summary = libllm_core::files::FileSummary {
            basename: "x.md".to_owned(),
            summary: "".to_owned(),
            status: libllm_core::files::FileSummaryStatus::Pending,
        };
        let lines = format_file_snapshot_block(&body, Some(&summary), true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Summary: (generating...)"));
    }

    #[test]
    fn format_file_snapshot_block_shows_unavailable_for_failed() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello");
        let summary = libllm_core::files::FileSummary {
            basename: "x.md".to_owned(),
            summary: "".to_owned(),
            status: libllm_core::files::FileSummaryStatus::Failed,
        };
        let lines = format_file_snapshot_block(&body, Some(&summary), true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Summary: (unavailable)"));
    }

    #[test]
    fn format_file_snapshot_block_shows_empty_for_done_empty() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello");
        let summary = libllm_core::files::FileSummary {
            basename: "x.md".to_owned(),
            summary: "".to_owned(),
            status: libllm_core::files::FileSummaryStatus::Done,
        };
        let lines = format_file_snapshot_block(&body, Some(&summary), true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Summary: (empty)"));
    }

    #[test]
    fn format_file_snapshot_block_single_line_when_summarization_disabled() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello");
        let lines = format_file_snapshot_block(&body, None, false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("--- File: x.md ("));
    }

    #[test]
    fn format_file_snapshot_block_generating_when_no_row() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello");
        let lines = format_file_snapshot_block(&body, None, true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Summary: (generating...)"));
    }

    #[test]
    fn format_file_snapshot_block_uses_kb_for_midsize() {
        let body = libllm_core::files::build_snapshot_body("big.md", &"x".repeat(20_000));
        let lines = format_file_snapshot_block(&body, None, false);
        assert!(lines[0].contains(" KB"));
    }

    #[test]
    fn format_bytes_boundary_values() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert!(format_bytes(1_048_576).contains("1.0 MB"));
    }

    #[test]
    fn inner_snapshot_size_matches_body_length() {
        let body = libllm_core::files::build_snapshot_body("x.md", "hello\nworld");
        let size = inner_snapshot_size(&body);
        assert_eq!(size, "hello\nworld".len());
    }

    fn deepseek_preset() -> libllm_core::preset::ReasoningPreset {
        libllm_core::preset::ReasoningPreset {
            name: "DeepSeek".to_owned(),
            prefix: "<think>\n".to_owned(),
            suffix: "\n</think>".to_owned(),
            separator: "\n\n".to_owned(),
        }
    }

    #[test]
    fn render_assistant_lines_collapses_completed_thought() {
        let theme = crate::theme::Theme::dark();
        let preset = deepseek_preset();
        let lines = render_assistant_lines(
            "<think>brainstorm</think>Answer",
            Some(12),
            &theme,
            Some(&preset),
            true,
            false,
        );

        assert_eq!(lines[0].spans[1].content, "(Thought for 12 seconds)");
        assert_eq!(lines[1].spans[1].content, "Answer");
    }

    #[test]
    fn render_assistant_lines_shows_inflight_thought_in_gray() {
        let theme = crate::theme::Theme::dark();
        let preset = deepseek_preset();
        let lines = render_assistant_lines(
            "<think>planning answer",
            None,
            &theme,
            Some(&preset),
            false,
            true,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[1].content, "planning answer");
        assert_eq!(lines[0].spans[1].style.fg, Some(theme.summary_indicator));
    }

    #[test]
    fn render_assistant_lines_hides_unclosed_explicit_tags_in_final_message() {
        let theme = crate::theme::Theme::dark();
        let preset = deepseek_preset();
        let lines = render_assistant_lines(
            "<think>unfinished",
            None,
            &theme,
            Some(&preset),
            true,
            false,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[1].content, "unfinished");
        assert_ne!(lines[0].spans[1].style.fg, Some(theme.summary_indicator));
    }

    #[test]
    fn render_assistant_lines_without_preset_renders_content_verbatim() {
        let theme = crate::theme::Theme::dark();
        let lines =
            render_assistant_lines("<think>a</think>answer", Some(5), &theme, None, true, false);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[1].content, "<think>a</think>answer");
    }
}
