mod business;
mod commands;
mod dialogs;
mod input;
mod render;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::client::{ApiClient, StreamToken};
use crate::context::ContextManager;
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, NodeId, SaveMode, Session, SessionEntry};

use dialogs::FieldDialog;

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
    SystemDialog,
    EditDialog,
    BranchDialog,
}

enum Action {
    SendMessage(String),
    EditMessage(String),
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

const SELF_FIELDS: &[&str] = &["Name", "Persona"];

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
    sidebar_state: ratatui::widgets::ListState,
    streaming_buffer: String,
    is_streaming: bool,
    model_name: String,
    status_message: String,
    should_quit: bool,
    command_picker_selected: usize,

    passkey_input: String,
    passkey_error: String,

    config_dialog: Option<FieldDialog<'a>>,
    self_dialog: Option<FieldDialog<'a>>,
    system_editor: Option<TextArea<'a>>,
    edit_editor: Option<TextArea<'a>>,

    character_names: Vec<String>,
    character_slugs: Vec<String>,
    character_selected: usize,

    worldbook_list: Vec<String>,
    worldbook_selected: usize,

    nav_cursor: Option<NodeId>,
    branch_dialog_items: Vec<(NodeId, String)>,
    branch_dialog_selected: usize,
    user_name: Option<String>,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    save_mode: SaveMode,
    template: Template,
    sampling: SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    let sidebar_sessions = business::discover_sidebar_sessions(&save_mode);

    let mut textarea = TextArea::default();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Alt+Enter for newline) "),
    );
    textarea.set_cursor_line_style(Style::default());

    let sidebar_state = ratatui::widgets::ListState::default();

    let mut app = App {
        client,
        session,
        focus: if save_mode.needs_passkey() {
            Focus::PasskeyDialog
        } else {
            Focus::Input
        },
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
        config_dialog: None,
        self_dialog: None,
        system_editor: None,
        edit_editor: None,
        character_names: Vec::new(),
        character_slugs: Vec::new(),
        character_selected: 0,
        worldbook_list: Vec::new(),
        worldbook_selected: 0,
        nav_cursor: None,
        branch_dialog_items: Vec::new(),
        branch_dialog_selected: 0,
        user_name: crate::config::load().user_name,
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
                let _ = tx
                    .send(BackgroundEvent::PreviewLoaded {
                        index: i,
                        preview,
                    })
                    .await;
            });
        }
    }

    let mut frame_tick = tokio::time::interval(std::time::Duration::from_millis(16));
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut needs_redraw = false;

    terminal.draw(|f| render_frame(f, &mut app))?;

    loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                if let Some(action) = handle_event(event, &mut app, bg_tx.clone()) {
                    process_action(action, &mut app, token_tx.clone());
                }
                needs_redraw = true;
            }
            Some(stream_token) = token_rx.recv() => {
                commands::handle_stream_token(stream_token, &mut app)?;
                needs_redraw = true;
            }
            Some(bg_event) = bg_rx.recv() => {
                commands::handle_background_event(bg_event, &mut app, bg_tx.clone());
                needs_redraw = true;
            }
            _ = frame_tick.tick() => {
                if needs_redraw {
                    terminal.draw(|f| render_frame(f, &mut app))?;
                    needs_redraw = false;
                }
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

fn render_frame(f: &mut ratatui::Frame, app: &mut App) {
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

    render::render_sidebar(f, app, sidebar_area);

    let border = render::border_style(app.focus == Focus::Input);
    let input_title = if app.nav_cursor.is_some() {
        " Input (Enter to edit, Esc to cancel) "
    } else {
        " Input (Up arrow to edit, Enter to send) "
    };
    app.textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(input_title)
            .border_style(border),
    );
    f.render_widget(&app.textarea, input_area);

    let branch_path = app.session.tree.branch_path();
    let branch_ids = app.session.tree.branch_path_ids();
    let branch_info = app.session.tree.deepest_branch_info();

    let mut chat_scroll = app.chat_scroll;
    render::render_chat(f, app, chat_area, &mut chat_scroll, &branch_path, &branch_ids);
    render::render_status_bar(f, app, status_area, &branch_path, branch_info);
    app.chat_scroll = chat_scroll;

    if app.focus == Focus::Input && input::input_has_command_picker(app) {
        let input_text = app.textarea.lines().join("\n");
        render::render_command_picker(f, app, &input_text, chat_area);
    }

    if app.focus == Focus::PasskeyDialog {
        dialogs::passkey::render_passkey_dialog(f, app, f.area());
    }
    if app.focus == Focus::ConfigDialog {
        if let Some(ref dialog) = app.config_dialog {
            dialog.render(f, f.area());
        }
    }
    if app.focus == Focus::SelfDialog {
        if let Some(ref dialog) = app.self_dialog {
            dialog.render(f, f.area());
        }
    }
    if app.focus == Focus::CharacterDialog {
        dialogs::character::render_character_dialog(f, app, f.area());
    }
    if app.focus == Focus::WorldbookDialog {
        dialogs::worldbook::render_worldbook_dialog(f, app, f.area());
    }
    if app.focus == Focus::SystemDialog {
        dialogs::system::render_system_dialog(f, app, f.area());
    }
    if app.focus == Focus::EditDialog {
        dialogs::edit::render_edit_dialog(f, app, f.area());
    }
    if app.focus == Focus::BranchDialog {
        dialogs::branch::render_branch_dialog(f, app, f.area());
    }
}

fn handle_event(
    event: Event,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(key, app, bg_tx),
        _ => None,
    }
}

fn process_action(action: Action, app: &mut App, token_tx: mpsc::Sender<StreamToken>) {
    match action {
        Action::Quit => {
            app.should_quit = true;
        }
        Action::SendMessage(text) => {
            app.nav_cursor = None;
            commands::start_streaming(app, &text, token_tx);
        }
        Action::EditMessage(text) => {
            app.nav_cursor = None;
            app.session.retreat_trailing_assistant();
            if app.session
                .tree
                .head()
                .and_then(|id| app.session.tree.node(id))
                .is_some_and(|n| n.message.role == crate::session::Role::User)
            {
                app.session.tree.retreat_head();
            }
            commands::start_streaming(app, &text, token_tx);
        }
        Action::SlashCommand(cmd, arg) => {
            commands::handle_slash_command(&cmd, &arg, app, token_tx);
        }
    }
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    if app.focus == Focus::PasskeyDialog {
        return dialogs::passkey::handle_passkey_key(key, app, bg_tx);
    }
    if app.focus == Focus::ConfigDialog {
        return handle_field_dialog_key(key, app, DialogKind::Config);
    }
    if app.focus == Focus::SelfDialog {
        return handle_field_dialog_key(key, app, DialogKind::SelfPersona);
    }
    if app.focus == Focus::CharacterDialog {
        return dialogs::character::handle_character_dialog_key(key, app);
    }
    if app.focus == Focus::WorldbookDialog {
        return dialogs::worldbook::handle_worldbook_dialog_key(key, app);
    }
    if app.focus == Focus::SystemDialog {
        return dialogs::system::handle_system_key(key, app);
    }
    if app.focus == Focus::EditDialog {
        return dialogs::edit::handle_edit_key(key, app);
    }
    if app.focus == Focus::BranchDialog {
        return dialogs::branch::handle_branch_dialog_key(key, app);
    }

    if app.is_streaming {
        return handle_streaming_key(key, app);
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        app.session.tree.switch_sibling(-1);
        let _ = app.session.maybe_save(&app.save_mode);
        app.status_message.clear();
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        app.session.tree.switch_sibling(1);
        let _ = app.session.maybe_save(&app.save_mode);
        app.status_message.clear();
        return None;
    }

    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Input => {
                let last_user = app.session.tree.branch_path_ids()
                    .into_iter()
                    .rev()
                    .find(|&id| app.session.tree.node(id)
                        .is_some_and(|n| n.message.role == crate::session::Role::User));
                app.nav_cursor = last_user;
                app.auto_scroll = false;
                Focus::Chat
            }
            Focus::Chat => {
                app.nav_cursor = None;
                Focus::Sidebar
            }
            _ => {
                app.nav_cursor = None;
                Focus::Input
            }
        };
        return None;
    }

    if key.code == KeyCode::Esc {
        app.nav_cursor = None;
        app.focus = Focus::Input;
        app.auto_scroll = true;
        return None;
    }

    match app.focus {
        Focus::Input => input::handle_input_key(key, app),
        Focus::Chat => input::handle_chat_key(key, app),
        Focus::Sidebar => input::handle_sidebar_key(key, app),
        _ => None,
    }
}

fn handle_streaming_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Esc => {
            cancel_generation(app);
            None
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Quit)
        }
        _ => None,
    }
}

fn cancel_generation(app: &mut App) {
    app.streaming_buffer.clear();
    app.is_streaming = false;
    app.session.tree.pop_head();
    app.auto_scroll = true;
    app.status_message = "Generation cancelled.".to_owned();
}

fn open_edit_dialog(app: &mut App) {
    let last_user_content = app
        .session
        .tree
        .head()
        .and_then(|id| {
            let node = app.session.tree.node(id)?;
            if node.message.role == crate::session::Role::Assistant {
                let parent = node.parent?;
                app.session.tree.node(parent)
            } else {
                Some(node)
            }
        })
        .filter(|n| n.message.role == crate::session::Role::User)
        .map(|n| n.message.content.clone())
        .unwrap_or_default();
    open_edit_dialog_with(app, &last_user_content);
}

fn open_edit_dialog_with(app: &mut App, content: &str) {
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let mut editor = TextArea::from(if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    });
    editor.set_cursor_line_style(ratatui::style::Style::default());
    editor.move_cursor(tui_textarea::CursorMove::Bottom);
    editor.move_cursor(tui_textarea::CursorMove::End);
    app.edit_editor = Some(editor);
    app.focus = Focus::EditDialog;
}

enum DialogKind {
    Config,
    SelfPersona,
}

fn handle_field_dialog_key(
    key: KeyEvent,
    app: &mut App,
    kind: DialogKind,
) -> Option<Action> {
    let dialog = match kind {
        DialogKind::Config => app.config_dialog.as_mut(),
        DialogKind::SelfPersona => app.self_dialog.as_mut(),
    };

    let Some(dialog) = dialog else {
        return None;
    };

    match dialog.handle_key(key) {
        dialogs::FieldDialogAction::Continue => None,
        dialogs::FieldDialogAction::Close => {
            match kind {
                DialogKind::Config => {
                    let values = &app.config_dialog.as_ref().unwrap().values;
                    business::save_config_from_fields(values);
                    business::apply_config(app);
                    app.config_dialog = None;
                    app.status_message = "Configuration saved.".to_owned();
                }
                DialogKind::SelfPersona => {
                    let values = &app.self_dialog.as_ref().unwrap().values;
                    business::save_self_fields(values);
                    app.user_name = crate::config::load().user_name;
                    app.self_dialog = None;
                    app.status_message = "User persona saved.".to_owned();
                }
            }
            app.focus = Focus::Input;
            None
        }
    }
}
