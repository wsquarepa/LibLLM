use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::client::{ApiClient, StreamToken};
use crate::context::ContextManager;
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, Role, Session};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Chat,
    Sidebar,
}

enum Action {
    SendMessage(String),
    SlashCommand(String, String),
    Quit,
}

struct App<'a> {
    client: &'a ApiClient,
    session: &'a mut Session,
    session_path: Option<&'a Path>,
    template: Template,
    stop_tokens: &'static [&'static str],
    sampling: &'a SamplingParams,
    context_mgr: ContextManager,

    focus: Focus,
    textarea: TextArea<'a>,
    chat_scroll: u16,
    auto_scroll: bool,
    sidebar_sessions: Vec<PathBuf>,
    sidebar_state: ListState,
    streaming_buffer: String,
    is_streaming: bool,
    model_name: String,
    status_message: String,
    should_quit: bool,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: Template,
    sampling: &SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    let sidebar_sessions = discover_sessions(session_path);

    let mut textarea = TextArea::default();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Alt+Enter for newline) "),
    );
    textarea.set_cursor_line_style(Style::default());

    let mut sidebar_state = ListState::default();
    if !sidebar_sessions.is_empty() {
        let selected = session_path
            .and_then(|sp| sidebar_sessions.iter().position(|p| p == sp))
            .unwrap_or(0);
        sidebar_state.select(Some(selected));
    }

    let mut app = App {
        client,
        session,
        session_path,
        template,
        stop_tokens: template.stop_tokens(),
        sampling,
        context_mgr: ContextManager::default(),
        focus: Focus::Input,
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
    let mut event_stream = EventStream::new();

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                if let Some(action) = handle_event(event, &mut app) {
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
        .constraints([Constraint::Length(22), Constraint::Min(30)])
        .split(main_area);

    let sidebar_area = columns[0];
    let right_area = columns[1];

    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(right_area);

    let chat_area = right_split[0];
    let input_area = right_split[1];

    let mut chat_scroll = app.chat_scroll;
    render_sidebar(f, app, sidebar_area);
    render_chat(f, app, chat_area, &mut chat_scroll);
    render_input(f, app, input_area);
    render_status_bar(f, app, status_area);
    app.chat_scroll = chat_scroll;
}

fn render_sidebar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .sidebar_sessions
        .iter()
        .map(|p| {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.to_string_lossy().to_string());
            ListItem::new(name)
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

fn render_chat(f: &mut ratatui::Frame, app: &App, area: Rect, chat_scroll: &mut u16) {
    let mut lines: Vec<Line> = Vec::new();

    let branch_path = app.session.tree.branch_path();
    let branch_ids = app.session.tree.branch_path_ids();

    for (msg, &node_id) in branch_path.iter().zip(branch_ids.iter()) {
        let (role_label, role_color) = match msg.role {
            Role::User => ("You", Color::Green),
            Role::Assistant => ("Assistant", Color::Blue),
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

        for content_line in msg.content.lines() {
            lines.push(Line::from(format!("  {content_line}")));
        }
        lines.push(Line::from(""));
    }

    if app.is_streaming && !app.streaming_buffer.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                "Assistant: ",
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
        ]));
        for content_line in app.streaming_buffer.lines() {
            lines.push(Line::from(format!("  {content_line}")));
        }
    }

    let content_height = lines.len() as u16;
    let visible_height = area.height.saturating_sub(2);

    if app.auto_scroll && content_height > visible_height {
        *chat_scroll = content_height.saturating_sub(visible_height);
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

fn render_input(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let border = border_style(app.focus == Focus::Input);
    let mut textarea = app.textarea.clone();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Alt+Enter for newline) ")
            .border_style(border),
    );
    f.render_widget(&textarea, area);
}

fn render_status_bar(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let branch_path = app.session.tree.branch_path();
    let token_count = ContextManager::estimate_message_tokens(&branch_path);

    let branch_info = match app.session.tree.deepest_branch_info() {
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

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn handle_event(event: Event, app: &mut App) -> Option<Action> {
    match event {
        Event::Key(key) => handle_key(key, app),
        _ => None,
    }
}

fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
        app.session.tree.switch_sibling(-1);
        let _ = app.session.maybe_save(app.session_path);
        app.status_message.clear();
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.session.tree.switch_sibling(1);
        let _ = app.session.maybe_save(app.session_path);
        app.status_message.clear();
        return None;
    }

    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Input => Focus::Chat,
            Focus::Chat => Focus::Sidebar,
            Focus::Sidebar => Focus::Input,
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
    }
}

fn handle_input_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }

    match key.code {
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
            let lines: Vec<String> = app.textarea.lines().to_vec();
            let text = lines.join("\n");
            let trimmed = text.trim().to_owned();

            if trimmed.is_empty() {
                return None;
            }

            // Clear textarea
            app.textarea = TextArea::default();
            app.textarea.set_cursor_line_style(Style::default());

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
            None
        }
        KeyCode::Down => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let new = (selected + 1) % count;
            app.sidebar_state.select(Some(new));
            None
        }
        KeyCode::Enter => {
            if let Some(selected) = app.sidebar_state.selected() {
                let path = app.sidebar_sessions[selected].clone();
                match session::load(&path) {
                    Ok(loaded) => {
                        *app.session = loaded;
                        app.status_message = format!("Loaded: {}", path.display());
                        app.auto_scroll = true;
                    }
                    Err(e) => {
                        app.status_message = format!("Error loading: {e}");
                    }
                }
            }
            None
        }
        _ => None,
    }
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
    let prompt = app.template.render(truncated, app.session.system_prompt.as_deref());
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
            app.session.maybe_save(app.session_path)?;
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.status_message = format!("Error: {err}");
        }
    }
    Ok(())
}

fn handle_slash_command(cmd: &str, arg: &str, app: &mut App, sender: mpsc::Sender<StreamToken>) {
    match cmd {
        "/quit" | "/exit" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.session.tree.clear();
            app.status_message = "Conversation cleared.".to_owned();
            let _ = app.session.maybe_save(app.session_path);
        }
        "/retry" => {
            app.session.pop_trailing_assistant();

            let last_user_content = app.session.tree.head()
                .filter(|&id| app.session.tree.node(id).message.role == Role::User)
                .map(|id| app.session.tree.node(id).message.content.clone());

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
                    .is_some_and(|id| app.session.tree.node(id).message.role == Role::User)
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
                let _ = app.session.maybe_save(app.session_path);
            }
        }
        "/save" => {
            if arg.is_empty() {
                match app.session_path {
                    Some(path) => {
                        match session::save(path, app.session) {
                            Ok(()) => app.status_message = format!("Saved to {}.", path.display()),
                            Err(e) => app.status_message = format!("Save error: {e}"),
                        }
                    }
                    None => app.status_message = "Usage: /save <path>".to_owned(),
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
        _ => {
            app.status_message = format!("Unknown command: {cmd}");
        }
    }
}

fn discover_sessions(session_path: Option<&Path>) -> Vec<PathBuf> {
    let dir = match session_path.and_then(|p| p.parent()) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut sessions: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    sessions.sort();
    sessions
}
