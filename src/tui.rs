use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

const DIALOGUE_COLOR: Color = Color::LightBlue;

use crate::client::{ApiClient, StreamToken};
use crate::context::ContextManager;
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, NodeId, Role, SaveMode, Session, SessionEntry};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Chat,
    Sidebar,
    PasskeyDialog,
    ConfigDialog,
    SelfDialog,
    CharacterDialog,
    WorldbookDialog,
}

enum Action {
    SendMessage(String),
    SlashCommand(String, String),
    Quit,
}

enum BackgroundEvent {
    KeyDerived(std::sync::Arc<crate::crypto::DerivedKey>, std::path::PathBuf),
    KeyDeriveFailed(String),
    PreviewLoaded { index: usize, preview: String },
}

const CONFIG_FIELDS: &[&str] = &[
    "API URL",
    "Template",
    "System Prompt",
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
];

struct App<'a> {
    client: &'a ApiClient,
    session: &'a mut Session,
    save_mode: SaveMode,
    template: Template,
    stop_tokens: &'static [&'static str],
    sampling: SamplingParams,
    context_mgr: ContextManager,

    focus: Focus,
    textarea: TextArea<'a>,
    chat_scroll: u16,
    auto_scroll: bool,
    sidebar_sessions: Vec<SessionEntry>,
    sidebar_state: ListState,
    streaming_buffer: String,
    is_streaming: bool,
    model_name: String,
    status_message: String,
    should_quit: bool,
    command_picker_selected: usize,

    passkey_input: String,
    passkey_error: String,

    config_fields: Vec<String>,
    config_selected: usize,
    config_editing: bool,

    self_fields: Vec<String>,
    self_selected: usize,
    self_editing: bool,

    character_names: Vec<String>,
    character_slugs: Vec<String>,
    character_selected: usize,

    worldbook_list: Vec<String>,
    worldbook_selected: usize,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    save_mode: SaveMode,
    template: Template,
    sampling: SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    let sidebar_sessions = discover_sidebar_sessions(&save_mode);

    let mut textarea = TextArea::default();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Alt+Enter for newline) "),
    );
    textarea.set_cursor_line_style(Style::default());

    let sidebar_state = ListState::default();

    let mut app = App {
        client,
        session,
        focus: if save_mode.needs_passkey() { Focus::PasskeyDialog } else { Focus::Input },
        save_mode,
        template,
        stop_tokens: template.stop_tokens(),
        sampling,
        context_mgr: ContextManager::default(),
        textarea,
        chat_scroll: 0,
        auto_scroll: true,
        sidebar_sessions,
        sidebar_state,
        streaming_buffer: String::new(),
        is_streaming: false,
        model_name,
        status_message: String::new(),
        should_quit: false,
        command_picker_selected: 0,
        passkey_input: String::new(),
        passkey_error: String::new(),
        config_fields: Vec::new(),
        config_selected: 0,
        config_editing: false,
        self_fields: Vec::new(),
        self_selected: 0,
        self_editing: false,
        character_names: Vec::new(),
        character_slugs: Vec::new(),
        character_selected: 0,
        worldbook_list: Vec::new(),
        worldbook_selected: 0,
    };

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let (token_tx, mut token_rx) = mpsc::channel::<StreamToken>(256);
    let (bg_tx, mut bg_rx) = mpsc::channel::<BackgroundEvent>(64);
    let mut event_stream = EventStream::new();

    if let SaveMode::Encrypted { key, .. } = &app.save_mode {
        for i in 0..app.sidebar_sessions.len() {
            if app.sidebar_sessions[i].is_new_chat {
                continue;
            }
            let entry_path = app.sidebar_sessions[i].path.clone();
            let key = key.clone();
            let tx = bg_tx.clone();
            tokio::spawn(async move {
                let preview = session::load_preview(&entry_path, &key);
                let _ = tx.send(BackgroundEvent::PreviewLoaded { index: i, preview }).await;
            });
        }
    }

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                if let Some(action) = handle_event(event, &mut app, bg_tx.clone()) {
                    match action {
                        Action::Quit => break,
                        Action::SendMessage(text) => {
                            start_streaming(&mut app, &text, token_tx.clone());
                        }
                        Action::SlashCommand(cmd, arg) => {
                            handle_slash_command(&cmd, &arg, &mut app, token_tx.clone());
                        }
                    }
                }
            }
            Some(stream_token) = token_rx.recv() => {
                handle_stream_token(stream_token, &mut app)?;
            }
            Some(bg_event) = bg_rx.recv() => {
                handle_background_event(bg_event, &mut app, bg_tx.clone());
            }
        }

        if app.should_quit {
            break;
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;

    Ok(())
}

fn render(f: &mut ratatui::Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(f.area());

    let main_area = outer[0];
    let status_area = outer[1];

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(30)])
        .split(main_area);

    let sidebar_area = columns[0];
    let right_area = columns[1];

    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(right_area);

    let chat_area = right_split[0];
    let input_area = right_split[1];

    render_sidebar(f, app, sidebar_area);

    let border = border_style(app.focus == Focus::Input);
    app.textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Alt+Enter for newline) ")
            .border_style(border),
    );
    f.render_widget(&app.textarea, input_area);

    let branch_path = app.session.tree.branch_path();
    let branch_ids = app.session.tree.branch_path_ids();
    let branch_info = app.session.tree.deepest_branch_info();

    let mut chat_scroll = app.chat_scroll;
    render_chat(f, app, chat_area, &mut chat_scroll, &branch_path, &branch_ids);
    render_status_bar(f, app, status_area, &branch_path, branch_info);
    app.chat_scroll = chat_scroll;

    let input_text = app.textarea.lines().join("\n");
    if input_text.starts_with('/') && app.focus == Focus::Input && !app.is_streaming {
        render_command_picker(f, app, &input_text, chat_area);
    }

    if app.focus == Focus::PasskeyDialog {
        render_passkey_dialog(f, app, f.area());
    }
    if app.focus == Focus::ConfigDialog {
        render_config_dialog(f, app, f.area());
    }
    if app.focus == Focus::SelfDialog {
        render_self_dialog(f, app, f.area());
    }
    if app.focus == Focus::CharacterDialog {
        render_character_dialog(f, app, f.area());
    }
    if app.focus == Focus::WorldbookDialog {
        render_worldbook_dialog(f, app, f.area());
    }
}

fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .sidebar_sessions
        .iter()
        .map(|entry| {
            if entry.preview.is_empty() {
                ListItem::new(entry.filename.clone())
            } else {
                ListItem::new(format!("{}: {}", &entry.filename[..entry.filename.len().min(10)], entry.preview))
            }
        })
        .collect();

    let highlight_style = if app.focus == Focus::Sidebar {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Sessions ")
                .border_style(border_style(app.focus == Focus::Sidebar)),
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

fn render_chat(
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
            text.replace("{{char}}", char_name).replace("{{user}}", user_name)
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

        lines.push(Line::from(vec![
            Span::styled(
                format!("{role_label}{branch_indicator}: "),
                Style::default().fg(role_color).add_modifier(Modifier::BOLD),
            ),
        ]));

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
        lines.push(Line::from(vec![
            Span::styled(
                format!("{assistant_label}: "),
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
        ]));
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
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Chat ")
                .border_style(border_style(app.focus == Focus::Chat)),
        )
        .wrap(Wrap { trim: false })
        .scroll((*chat_scroll, 0));

    f.render_widget(paragraph, area);
}

fn render_command_picker(f: &mut ratatui::Frame, app: &App, prefix: &str, chat_area: Rect) {
    let matches = crate::commands::matching_commands(prefix.split_whitespace().next().unwrap_or("/"));
    if matches.is_empty() {
        return;
    }

    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            let label = if c.args.is_empty() {
                c.name.to_owned()
            } else {
                format!("{} {}", c.name, c.args)
            };
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

    let selected = app.command_picker_selected.min(matches.len().saturating_sub(1));
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

fn render_status_bar(
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

    let paragraph = Paragraph::new(status)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));

    f.render_widget(paragraph, area);
}

fn measure_wrapped_height(lines: &[Line], area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2);
    let max_height = (lines.len() as u16).saturating_mul(4).saturating_add(100);
    let measure_area = Rect::new(0, 0, inner_width, max_height);

    let paragraph = Paragraph::new(Text::from(lines.to_vec()))
        .wrap(Wrap { trim: false });

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

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn render_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = centered_rect(50, 7, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let masked: String = "*".repeat(app.passkey_input.len());
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Passkey: "),
            Span::styled(&masked, Style::default().fg(Color::Cyan)),
            Span::styled("_", Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK)),
        ]),
        Line::from(""),
    ];

    if !app.passkey_error.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", app.passkey_error),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Enter to submit, Esc to quit",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Unlock Sessions ")
                .border_style(Style::default().fg(Color::Yellow)),
        );

    f.render_widget(paragraph, dialog);
}

fn render_config_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let field_count = CONFIG_FIELDS.len();
    let dialog = centered_rect(60, field_count as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, &label) in CONFIG_FIELDS.iter().enumerate() {
        let value = &app.config_fields[i];
        let is_selected = i == app.config_selected;
        let cursor = if is_selected && app.config_editing { "_" } else { "" };

        let label_style = if is_selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let value_style = if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {label:<15}"), label_style),
            Span::styled(format!("{value}{cursor}"), value_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: edit  Esc: save & close",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Configuration ")
                .border_style(Style::default().fg(Color::Yellow)),
        );

    f.render_widget(paragraph, dialog);
}

const SELF_FIELDS: &[&str] = &["Name", "Persona"];

fn render_self_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = centered_rect(60, SELF_FIELDS.len() as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, &label) in SELF_FIELDS.iter().enumerate() {
        let value = &app.self_fields[i];
        let is_selected = i == app.self_selected;
        let cursor = if is_selected && app.self_editing { "_" } else { "" };

        let label_style = if is_selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let value_style = if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {label:<15}"), label_style),
            Span::styled(format!("{value}{cursor}"), value_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: edit  Esc: save & close",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" User Persona ")
                .border_style(Style::default().fg(Color::Yellow)),
        );

    f.render_widget(paragraph, dialog);
}

fn render_character_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.character_names.len();
    let dialog = centered_rect(50, count as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.character_names.iter().enumerate() {
        let is_selected = i == app.character_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{name}"), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: select  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Select Character ")
                .border_style(Style::default().fg(Color::Yellow)),
        );

    f.render_widget(paragraph, dialog);
}

fn render_worldbook_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.worldbook_list.len();
    let dialog = centered_rect(50, count as u16 + 4, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, name) in app.worldbook_list.iter().enumerate() {
        let is_selected = i == app.worldbook_selected;
        let is_active = app.session.worldbooks.contains(name);
        let checkbox = if is_active { "[x]" } else { "[ ]" };
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{checkbox} {name}"), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: toggle  Esc: close",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Worldbooks ")
                .border_style(Style::default().fg(Color::Yellow)),
        );

    f.render_widget(paragraph, dialog);
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn handle_event(event: Event, app: &mut App, bg_tx: mpsc::Sender<BackgroundEvent>) -> Option<Action> {
    match event {
        Event::Key(key) => handle_key(key, app, bg_tx),
        _ => None,
    }
}

fn handle_key(key: KeyEvent, app: &mut App, bg_tx: mpsc::Sender<BackgroundEvent>) -> Option<Action> {
    if app.focus == Focus::PasskeyDialog {
        return handle_passkey_key(key, app, bg_tx);
    }
    if app.focus == Focus::ConfigDialog {
        return handle_config_key(key, app);
    }
    if app.focus == Focus::SelfDialog {
        return handle_self_key(key, app);
    }
    if app.focus == Focus::CharacterDialog {
        return handle_character_dialog_key(key, app);
    }
    if app.focus == Focus::WorldbookDialog {
        return handle_worldbook_dialog_key(key, app);
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
        app.session.tree.switch_sibling(-1);
        let _ = app.session.maybe_save(&app.save_mode);
        app.status_message.clear();
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.session.tree.switch_sibling(1);
        let _ = app.session.maybe_save(&app.save_mode);
        app.status_message.clear();
        return None;
    }

    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Input => Focus::Chat,
            Focus::Chat => Focus::Sidebar,
            _ => Focus::Input,
        };
        return None;
    }

    if key.code == KeyCode::Esc {
        app.focus = Focus::Input;
        return None;
    }

    match app.focus {
        Focus::Input => handle_input_key(key, app),
        Focus::Chat => handle_chat_key(key, app),
        Focus::Sidebar => handle_sidebar_key(key, app),
        Focus::PasskeyDialog | Focus::ConfigDialog | Focus::SelfDialog
        | Focus::CharacterDialog | Focus::WorldbookDialog => None,
    }
}

fn input_has_command_picker(app: &App) -> bool {
    let text = app.textarea.lines().join("\n");
    text.starts_with('/') && !app.is_streaming
}

fn handle_input_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }

    let picker_active = input_has_command_picker(app);

    if picker_active {
        let matches = crate::commands::matching_commands(
            app.textarea.lines().join("\n").split_whitespace().next().unwrap_or("/"),
        );
        match key.code {
            KeyCode::Up => {
                app.command_picker_selected = app.command_picker_selected.saturating_sub(1);
                return None;
            }
            KeyCode::Down => {
                app.command_picker_selected = (app.command_picker_selected + 1).min(matches.len().saturating_sub(1));
                return None;
            }
            KeyCode::Tab if !matches.is_empty() => {
                let selected = app.command_picker_selected.min(matches.len().saturating_sub(1));
                let cmd_name = matches[selected].name;
                let suffix = if matches[selected].args.is_empty() { "" } else { " " };
                app.textarea = TextArea::from(vec![format!("{cmd_name}{suffix}")]);
                app.textarea.set_cursor_line_style(Style::default());
                app.textarea.move_cursor(tui_textarea::CursorMove::End);
                app.command_picker_selected = 0;
                return None;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
            let lines: Vec<String> = app.textarea.lines().to_vec();
            let text = lines.join("\n");
            let trimmed = text.trim().to_owned();

            if trimmed.is_empty() {
                return None;
            }

            app.textarea = TextArea::default();
            app.textarea.set_cursor_line_style(Style::default());
            app.command_picker_selected = 0;

            if trimmed.starts_with('/') {
                let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                let cmd = parts[0].to_owned();
                let arg = parts.get(1).map(|s| s.trim().to_owned()).unwrap_or_default();
                return Some(Action::SlashCommand(cmd, arg));
            }

            Some(Action::SendMessage(trimmed))
        }
        _ => {
            app.textarea.input(key);
            app.command_picker_selected = 0;
            None
        }
    }
}

fn handle_chat_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Up => {
            app.chat_scroll = app.chat_scroll.saturating_sub(1);
            app.auto_scroll = false;
            None
        }
        KeyCode::Down => {
            app.chat_scroll = app.chat_scroll.saturating_add(1);
            None
        }
        KeyCode::PageUp => {
            app.chat_scroll = app.chat_scroll.saturating_sub(10);
            app.auto_scroll = false;
            None
        }
        KeyCode::PageDown => {
            app.chat_scroll = app.chat_scroll.saturating_add(10);
            None
        }
        KeyCode::End => {
            app.auto_scroll = true;
            None
        }
        _ => None,
    }
}

fn load_sidebar_selection(app: &mut App) {
    let Some(selected) = app.sidebar_state.selected() else { return };
    let entry = &app.sidebar_sessions[selected];
    if entry.is_new_chat {
        *app.session = Session::default();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path = crate::config::sessions_dir().join(session::generate_session_name());
        app.save_mode.set_path(new_path);
        app.status_message = "New conversation started.".to_owned();
    } else {
        let path = entry.path.clone();
        let load_result = match &app.save_mode {
            SaveMode::Encrypted { key, .. } => session::load_encrypted(&path, key),
            _ => session::load(&path),
        };
        match load_result {
            Ok(loaded) => {
                *app.session = loaded;
                app.status_message = format!("Loaded: {}", entry.filename);
                app.save_mode.set_path(path);
                app.chat_scroll = 0;
                app.auto_scroll = true;
            }
            Err(e) => {
                app.status_message = format!("Error loading: {e}");
            }
        }
    }
}

fn handle_sidebar_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let count = app.sidebar_sessions.len();
    if count == 0 {
        return None;
    }

    match key.code {
        KeyCode::Up => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let new = if selected == 0 { count - 1 } else { selected - 1 };
            app.sidebar_state.select(Some(new));
            load_sidebar_selection(app);
            None
        }
        KeyCode::Down => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let new = (selected + 1) % count;
            app.sidebar_state.select(Some(new));
            load_sidebar_selection(app);
            None
        }
        _ => None,
    }
}

fn handle_passkey_key(key: KeyEvent, app: &mut App, bg_tx: mpsc::Sender<BackgroundEvent>) -> Option<Action> {
    match key.code {
        KeyCode::Enter => {
            let passkey = app.passkey_input.clone();
            let path = match &app.save_mode {
                SaveMode::PendingPasskey(p) => p.clone(),
                _ => return None,
            };
            app.passkey_input.clear();
            app.passkey_error.clear();
            app.status_message = "Deriving key...".to_owned();

            tokio::spawn(async move {
                let salt_path = crate::config::salt_path();
                let check_path = crate::config::key_check_path();
                let result = crate::crypto::load_or_create_salt(&salt_path)
                    .and_then(|salt| crate::crypto::derive_key(&passkey, &salt));
                match result {
                    Ok(derived_key) => {
                        match crate::crypto::verify_or_set_key(&check_path, &derived_key) {
                            Ok(true) => {
                                let key = std::sync::Arc::new(derived_key);
                                let _ = bg_tx.send(BackgroundEvent::KeyDerived(key, path)).await;
                            }
                            Ok(false) => {
                                let _ = bg_tx.send(BackgroundEvent::KeyDeriveFailed(
                                    "Wrong passkey.".to_owned(),
                                )).await;
                            }
                            Err(e) => {
                                let _ = bg_tx.send(BackgroundEvent::KeyDeriveFailed(e.to_string())).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = bg_tx.send(BackgroundEvent::KeyDeriveFailed(e.to_string())).await;
                    }
                }
            });
            None
        }
        KeyCode::Char(c) => {
            app.passkey_input.push(c);
            app.passkey_error.clear();
            None
        }
        KeyCode::Backspace => {
            app.passkey_input.pop();
            app.passkey_error.clear();
            None
        }
        KeyCode::Esc => Some(Action::Quit),
        _ => None,
    }
}

fn handle_config_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.config_editing {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                app.config_editing = false;
            }
            KeyCode::Char(c) => {
                app.config_fields[app.config_selected].push(c);
            }
            KeyCode::Backspace => {
                app.config_fields[app.config_selected].pop();
            }
            _ => {}
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.config_selected = app.config_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.config_selected = (app.config_selected + 1).min(CONFIG_FIELDS.len() - 1);
        }
        KeyCode::Enter => {
            app.config_editing = true;
        }
        KeyCode::Esc => {
            save_config_from_fields(app);
            apply_config(app);
            app.focus = Focus::Input;
            app.status_message = "Configuration saved.".to_owned();
        }
        _ => {}
    }
    None
}

fn handle_self_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.self_editing {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                app.self_editing = false;
            }
            KeyCode::Char(c) => {
                app.self_fields[app.self_selected].push(c);
            }
            KeyCode::Backspace => {
                app.self_fields[app.self_selected].pop();
            }
            _ => {}
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.self_selected = app.self_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.self_selected = (app.self_selected + 1).min(SELF_FIELDS.len() - 1);
        }
        KeyCode::Enter => {
            app.self_editing = true;
        }
        KeyCode::Esc => {
            save_self_fields(app);
            app.focus = Focus::Input;
            app.status_message = "User persona saved.".to_owned();
        }
        _ => {}
    }
    None
}

fn handle_character_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.character_names.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.character_selected = app.character_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.character_selected = (app.character_selected + 1).min(app.character_names.len() - 1);
        }
        KeyCode::Enter => {
            let slug = app.character_slugs[app.character_selected].clone();
            let card_path = crate::config::characters_dir().join(format!("{slug}.json"));
            match crate::character::load_card(&card_path) {
                Ok(card) => {
                    app.session.tree.clear();
                    app.session.system_prompt = Some(crate::character::build_system_prompt(&card));
                    app.session.character = Some(card.name.clone());
                    app.session.worldbooks.clear();
                    if !card.first_mes.is_empty() {
                        app.session.tree.push(None, Message::new(Role::Assistant, card.first_mes));
                    }
                    app.chat_scroll = 0;
                    app.auto_scroll = true;
                    let new_path = crate::config::sessions_dir()
                        .join(session::generate_session_name_for_character(&card.name));
                    app.save_mode.set_path(new_path);
                    app.status_message = format!("Loaded character: {}", card.name);
                    app.focus = Focus::Input;
                }
                Err(e) => {
                    app.status_message = format!("Error: {e}");
                    app.focus = Focus::Input;
                }
            }
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn handle_worldbook_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.worldbook_list.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            app.worldbook_selected = app.worldbook_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.worldbook_selected = (app.worldbook_selected + 1).min(app.worldbook_list.len() - 1);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let name = app.worldbook_list[app.worldbook_selected].clone();
            if app.session.worldbooks.contains(&name) {
                app.session.worldbooks.retain(|n| n != &name);
                app.status_message = format!("Disabled: {name}");
            } else {
                app.session.worldbooks.push(name.clone());
                app.status_message = format!("Enabled: {name}");
            }
            let _ = app.session.maybe_save(&app.save_mode);
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

fn load_self_fields() -> Vec<String> {
    let cfg = crate::config::load();
    vec![
        cfg.user_name.unwrap_or_default(),
        cfg.user_persona.unwrap_or_default(),
    ]
}

fn save_self_fields(app: &App) {
    let mut cfg = crate::config::load();
    cfg.user_name = non_empty(&app.self_fields[0]);
    cfg.user_persona = non_empty(&app.self_fields[1]);

    let path = crate::config::config_path();
    if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, toml_str);
    }
}

fn build_effective_system_prompt(session: &Session) -> Option<String> {
    let cfg = crate::config::load();
    let base = session.system_prompt.as_deref().unwrap_or("");
    let is_character = session.character.is_some();
    let has_persona = is_character && (cfg.user_name.is_some() || cfg.user_persona.is_some());

    if base.is_empty() && !has_persona {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    if !base.is_empty() {
        parts.push(base.to_owned());
    }
    if has_persona {
        let name = cfg.user_name.as_deref().unwrap_or("the user");
        let mut persona_line = format!("The user's name is {name}.");
        if let Some(ref desc) = cfg.user_persona {
            if !desc.is_empty() {
                persona_line.push_str(&format!(" {desc}"));
            }
        }
        parts.push(persona_line);
    }

    let mut result = parts.join("\n\n");
    if is_character {
        let char_name = session.character.as_deref().unwrap_or("");
        let user_name = cfg.user_name.as_deref().unwrap_or("User");
        result = result.replace("{{char}}", char_name).replace("{{user}}", user_name);
    }

    Some(result)
}

fn inject_worldbook_entries<'a>(
    session: &Session,
    messages: &[&'a Message],
) -> Vec<Message> {
    if session.character.is_none() || session.worldbooks.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    let cfg = crate::config::load();
    let char_name = session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");

    let msg_texts: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    let wi_dir = crate::config::worldinfo_dir();

    let mut all_activated: Vec<crate::worldinfo::ActivatedEntry> = Vec::new();
    for wb_name in &session.worldbooks {
        let wb_path = wi_dir.join(format!("{wb_name}.json"));
        if let Ok(wb) = crate::worldinfo::load_worldbook(&wb_path) {
            all_activated.extend(crate::worldinfo::scan_entries(&wb, &msg_texts));
        }
    }

    if all_activated.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    all_activated.sort_by_key(|e| e.order);

    let mut result: Vec<Message> = messages.iter().map(|m| (*m).clone()).collect();
    let len = result.len();

    for entry in all_activated.into_iter().rev() {
        let content = entry.content
            .replace("{{char}}", char_name)
            .replace("{{user}}", user_name);
        let insert_pos = if entry.depth == 0 || entry.depth >= len {
            0
        } else {
            len - entry.depth
        };
        result.insert(insert_pos, Message::new(Role::System, content));
    }

    result
}

fn load_config_fields() -> Vec<String> {
    let cfg = crate::config::load();
    let defaults = crate::sampling::SamplingParams::default();
    vec![
        cfg.api_url.unwrap_or_else(|| crate::config::Config::default().api_url().to_owned()),
        cfg.template.unwrap_or_else(|| "llama2".to_owned()),
        cfg.system_prompt.unwrap_or_default(),
        cfg.sampling.temperature.unwrap_or(defaults.temperature).to_string(),
        cfg.sampling.top_k.unwrap_or(defaults.top_k).to_string(),
        cfg.sampling.top_p.unwrap_or(defaults.top_p).to_string(),
        cfg.sampling.min_p.unwrap_or(defaults.min_p).to_string(),
        cfg.sampling.repeat_last_n.unwrap_or(defaults.repeat_last_n).to_string(),
        cfg.sampling.repeat_penalty.unwrap_or(defaults.repeat_penalty).to_string(),
        cfg.sampling.max_tokens.unwrap_or(defaults.max_tokens).to_string(),
    ]
}

fn save_config_from_fields(app: &App) {
    let existing = crate::config::load();
    let fields = &app.config_fields;
    let cfg = crate::config::Config {
        api_url: non_empty(&fields[0]),
        template: non_empty(&fields[1]),
        system_prompt: non_empty(&fields[2]),
        user_name: existing.user_name,
        user_persona: existing.user_persona,
        sampling: crate::sampling::SamplingOverrides {
            temperature: fields[3].parse().ok(),
            top_k: fields[4].parse().ok(),
            top_p: fields[5].parse().ok(),
            min_p: fields[6].parse().ok(),
            repeat_last_n: fields[7].parse().ok(),
            repeat_penalty: fields[8].parse().ok(),
            max_tokens: fields[9].parse().ok(),
        },
    };

    let path = crate::config::config_path();
    if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, toml_str);
    }
}

fn apply_config(app: &mut App) {
    let cfg = crate::config::load();
    let template_name = cfg.template.as_deref().unwrap_or("llama2");
    app.template = Template::from_name(template_name);
    app.stop_tokens = app.template.stop_tokens();
    app.sampling = SamplingParams::default().with_overrides(&cfg.sampling);

    if let Some(sp) = cfg.system_prompt {
        app.session.system_prompt = Some(sp);
    }
}

fn non_empty(s: &str) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s.to_owned()) }
}

fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    let parent = app.session.tree.head();
    app.session.tree.push(parent, Message::new(Role::User, content.to_owned()));
    app.is_streaming = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;
    app.status_message = "Generating...".to_owned();

    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt = build_effective_system_prompt(app.session);
    let injected = inject_worldbook_entries(app.session, truncated);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app.template.render(&injected_refs, effective_prompt.as_deref());
    let stop_tokens = app.stop_tokens;
    let sampling = app.sampling.clone();

    let client = app.client.clone();
    tokio::spawn(async move {
        client
            .stream_completion_to_channel(&prompt, stop_tokens, &sampling, sender)
            .await;
    });
}

fn handle_stream_token(token: StreamToken, app: &mut App) -> Result<()> {
    match token {
        StreamToken::Token(text) => {
            app.streaming_buffer.push_str(&text);
            app.auto_scroll = true;
        }
        StreamToken::Done(full_response) => {
            let head = app.session.tree.head().unwrap();
            app.session.tree.push(Some(head), Message::new(Role::Assistant, full_response));
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.status_message.clear();
            app.session.maybe_save(&app.save_mode)?;
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.status_message = format!("Error: {err}");
        }
    }
    Ok(())
}

fn handle_background_event(event: BackgroundEvent, app: &mut App, bg_tx: mpsc::Sender<BackgroundEvent>) {
    match event {
        BackgroundEvent::KeyDerived(key, path) => {
            app.save_mode = SaveMode::Encrypted {
                path,
                key: key.clone(),
            };
            app.focus = Focus::Input;
            app.status_message.clear();

            let sessions_dir = crate::config::sessions_dir();
            let mut sessions = session::list_session_paths(&sessions_dir);
            sessions.insert(0, new_chat_entry());
            app.sidebar_sessions = sessions;
            app.sidebar_state.select(Some(0));

            for i in 0..app.sidebar_sessions.len() {
                if app.sidebar_sessions[i].is_new_chat {
                    continue;
                }
                let entry_path = app.sidebar_sessions[i].path.clone();
                let key = key.clone();
                let tx = bg_tx.clone();
                tokio::spawn(async move {
                    let preview = session::load_preview(&entry_path, &key);
                    let _ = tx.send(BackgroundEvent::PreviewLoaded { index: i, preview }).await;
                });
            }
        }
        BackgroundEvent::KeyDeriveFailed(err) => {
            app.passkey_error = format!("Failed: {err}");
            app.status_message.clear();
        }
        BackgroundEvent::PreviewLoaded { index, preview } => {
            if index < app.sidebar_sessions.len() {
                app.sidebar_sessions[index].preview = preview;
            }
        }
    }
}

fn handle_slash_command(cmd: &str, arg: &str, app: &mut App, sender: mpsc::Sender<StreamToken>) {
    match cmd {
        "/help" => {
            app.status_message = "Use Tab to complete commands, Up/Down to navigate.".to_owned();
        }
        "/quit" | "/exit" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.session.tree.clear();
            app.session.system_prompt = None;
            app.chat_scroll = 0;
            app.auto_scroll = true;
            let new_name = session::generate_session_name();
            let new_path = crate::config::sessions_dir().join(&new_name);
            app.save_mode.set_path(new_path);
            app.status_message = "New conversation started.".to_owned();
        }
        "/retry" => {
            app.session.pop_trailing_assistant();

            let last_user_content = app.session.tree.head()
                .and_then(|id| app.session.tree.node(id))
                .filter(|n| n.message.role == Role::User)
                .map(|n| n.message.content.clone());

            match last_user_content {
                Some(content) => {
                    app.session.tree.pop_head();
                    start_streaming(app, &content, sender);
                }
                None => {
                    app.status_message = "No user message to retry.".to_owned();
                }
            }
        }
        "/edit" => {
            if arg.is_empty() {
                app.status_message = "Usage: /edit <new message text>".to_owned();
            } else {
                app.session.pop_trailing_assistant();
                if app.session.tree.head()
                    .and_then(|id| app.session.tree.node(id))
                    .is_some_and(|n| n.message.role == Role::User)
                {
                    app.session.tree.pop_head();
                }
                start_streaming(app, arg, sender);
            }
        }
        "/system" => {
            if arg.is_empty() {
                match &app.session.system_prompt {
                    Some(sp) => app.status_message = format!("System prompt: {sp}"),
                    None => app.status_message = "No system prompt set.".to_owned(),
                }
            } else {
                app.session.system_prompt = Some(arg.to_owned());
                app.status_message = "System prompt updated.".to_owned();
                let _ = app.session.maybe_save(&app.save_mode);
            }
        }
        "/save" => {
            if arg.is_empty() {
                match app.session.maybe_save(&app.save_mode) {
                    Ok(()) => match app.save_mode.path() {
                        Some(p) => app.status_message = format!("Saved to {}.", p.display()),
                        None => app.status_message = "No session path set.".to_owned(),
                    },
                    Err(e) => app.status_message = format!("Save error: {e}"),
                }
            } else {
                let path = PathBuf::from(arg);
                match session::save(&path, app.session) {
                    Ok(()) => app.status_message = format!("Saved to {arg}."),
                    Err(e) => app.status_message = format!("Save error: {e}"),
                }
            }
        }
        "/model" => {
            app.status_message = format!("Model: {}", app.model_name);
        }
        "/config" => {
            app.config_fields = load_config_fields();
            app.config_selected = 0;
            app.config_editing = false;
            app.focus = Focus::ConfigDialog;
        }
        "/load" => {
            if arg.is_empty() {
                app.status_message = "Usage: /load <path>".to_owned();
            } else {
                let path = PathBuf::from(arg);
                match session::load(&path) {
                    Ok(loaded) => {
                        *app.session = loaded;
                        let count = app.session.tree.branch_path().len();
                        app.status_message = format!("Loaded from {arg} ({count} messages).");
                        app.auto_scroll = true;
                    }
                    Err(e) => app.status_message = format!("Load error: {e}"),
                }
            }
        }
        "/branch" => {
            match arg {
                "next" => {
                    app.session.tree.switch_sibling(1);
                    app.status_message = "Switched to next branch.".to_owned();
                    let _ = app.session.maybe_save(&app.save_mode);
                }
                "prev" => {
                    app.session.tree.switch_sibling(-1);
                    app.status_message = "Switched to previous branch.".to_owned();
                    let _ = app.session.maybe_save(&app.save_mode);
                }
                "list" => {
                    let path_ids = app.session.tree.branch_path_ids();
                    let mut parts: Vec<String> = Vec::new();
                    for &node_id in &path_ids {
                        let (idx, total) = app.session.tree.sibling_info(node_id);
                        if total > 1 {
                            if let Some(node) = app.session.tree.node(node_id) {
                                parts.push(format!("#{node_id} ({}): {}/{total}", node.message.role, idx + 1));
                            }
                        }
                    }
                    if parts.is_empty() {
                        app.status_message = "No branch points.".to_owned();
                    } else {
                        app.status_message = format!("Branches: {}", parts.join(" | "));
                    }
                }
                _ => {
                    if let Ok(id) = arg.parse::<usize>() {
                        app.session.tree.switch_to(id);
                        app.status_message = format!("Switched to node {id}.");
                        let _ = app.session.maybe_save(&app.save_mode);
                    } else {
                        app.status_message = "Usage: /branch list|next|prev|<id>".to_owned();
                    }
                }
            }
        }
        "/self" => {
            app.self_fields = load_self_fields();
            app.self_selected = 0;
            app.self_editing = false;
            app.focus = Focus::SelfDialog;
        }
        "/worldbook" => {
            if app.session.character.is_none() {
                app.status_message = "Worldbooks are only available in character sessions.".to_owned();
            } else {
                let books = crate::worldinfo::list_worldbooks(&crate::config::worldinfo_dir());
                if books.is_empty() {
                    app.status_message = "No worldbooks found in worldinfo/ directory.".to_owned();
                } else {
                    app.worldbook_list = books.into_iter().map(|b| b.name).collect();
                    app.worldbook_selected = 0;
                    app.focus = Focus::WorldbookDialog;
                }
            }
        }
        "/character" => {
            if arg.starts_with("import") {
                let path_str = arg.strip_prefix("import").unwrap_or("").trim();
                if path_str.is_empty() {
                    app.status_message = "Usage: /character import <path>".to_owned();
                } else {
                    let source = std::path::Path::new(path_str);
                    match crate::character::import_card(source) {
                        Ok(card) => {
                            let name = card.name.clone();
                            match crate::character::save_card(&card, &crate::config::characters_dir()) {
                                Ok(_) => app.status_message = format!("Imported character: {name}"),
                                Err(e) => app.status_message = format!("Save error: {e}"),
                            }
                        }
                        Err(e) => app.status_message = format!("Import error: {e}"),
                    }
                }
            } else {
                let cards = crate::character::list_cards(&crate::config::characters_dir());
                if cards.is_empty() {
                    app.status_message = "No characters found. Use /character import <path>".to_owned();
                } else {
                    app.character_names = cards.iter().map(|c| c.name.clone()).collect();
                    app.character_slugs = cards.into_iter().map(|c| c.slug).collect();
                    app.character_selected = 0;
                    app.focus = Focus::CharacterDialog;
                }
            }
        }
        _ => {
            app.status_message = format!("Unknown command: {cmd}");
        }
    }
}

fn new_chat_entry() -> SessionEntry {
    SessionEntry {
        path: std::path::PathBuf::new(),
        preview: String::new(),
        filename: "+ New Chat".to_owned(),
        is_new_chat: true,
    }
}

fn discover_sidebar_sessions(save_mode: &SaveMode) -> Vec<SessionEntry> {
    let mut sessions = match save_mode {
        SaveMode::Encrypted { .. } => {
            session::list_session_paths(&crate::config::sessions_dir())
        }
        SaveMode::Plaintext(path) => {
            let dir = match path.parent() {
                Some(d) => d,
                None => return Vec::new(),
            };
            let mut entries: Vec<SessionEntry> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
                .map(|p| {
                    let filename = p.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    SessionEntry { path: p, preview: String::new(), filename, is_new_chat: false }
                })
                .collect();
            entries.sort_by(|a, b| b.filename.cmp(&a.filename));
            entries
        }
        SaveMode::None | SaveMode::PendingPasskey(_) => Vec::new(),
    };
    sessions.insert(0, new_chat_entry());
    sessions
}
