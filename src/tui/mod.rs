mod business;
mod commands;
mod dialogs;
mod input;
mod render;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::client::{ApiClient, StreamToken};
use crate::context::ContextManager;
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, NodeId, SaveMode, Session, SessionEntry};
use crate::worldinfo::RuntimeWorldBook;

use dialogs::FieldDialog;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Chat,
    Sidebar,
    PasskeyDialog,
    SetPasskeyDialog,
    ConfigDialog,
    SelfDialog,
    CharacterDialog,
    CharacterEditorDialog,
    WorldbookDialog,
    WorldbookEditorDialog,
    WorldbookEntryEditorDialog,
    WorldbookEntryDeleteDialog,
    SystemDialog,
    EditDialog,
    BranchDialog,
    DeleteConfirmDialog,
    ApiErrorDialog,
    LoadingDialog,
}

enum Action {
    SendMessage(String),
    EditMessage {
        node_id: crate::session::NodeId,
        content: String,
    },
    SlashCommand(String, String),
    Quit,
}

#[derive(Clone, Copy)]
enum StatusLevel {
    Info,
    Warning,
    Error,
}

struct StatusMessage {
    text: String,
    level: StatusLevel,
    expires: std::time::Instant,
}

struct WorldbookCache {
    enabled_names: Vec<String>,
    books: Vec<RuntimeWorldBook>,
}

enum BackgroundEvent {
    KeyDerived(
        std::sync::Arc<crate::crypto::DerivedKey>,
        std::path::PathBuf,
    ),
    KeyDeriveFailed(String),
    PasskeySet(std::sync::Arc<crate::crypto::DerivedKey>),
    PasskeySetFailed(String),
    ReEncryptionComplete(Vec<String>),
    MetadataLoaded {
        path: std::path::PathBuf,
        metadata: session::SessionMetadata,
    },
    ModelFetched(std::result::Result<String, String>),
}

const CONFIG_FIELDS: &[&str] = &[
    "API URL",
    "Template",
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
];

const SELF_FIELDS: &[&str] = &["Name", "Persona"];

#[derive(PartialEq, Eq)]
struct ScrollState {
    auto_scroll: bool,
    nav_cursor: Option<NodeId>,
    head: Option<NodeId>,
    buffer_len: usize,
    width: u16,
    height: u16,
}

const SIDEBAR_WIDTH: u16 = 32;
const INPUT_HEIGHT: u16 = 5;

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
    last_scroll_state: ScrollState,
    sidebar_sessions: Vec<SessionEntry>,
    sidebar_state: ratatui::widgets::ListState,
    streaming_buffer: String,
    is_streaming: bool,
    model_name: Option<String>,
    api_available: bool,
    api_error: String,
    status_message: Option<StatusMessage>,
    should_quit: bool,
    passkey_changed: bool,
    command_picker_selected: usize,

    passkey_input: String,
    passkey_error: String,
    passkey_deriving: bool,

    set_passkey_input: String,
    set_passkey_confirm: String,
    set_passkey_active_field: u8,
    set_passkey_error: String,
    set_passkey_deriving: bool,
    set_passkey_is_initial: bool,

    config_dialog: Option<FieldDialog<'a>>,
    self_dialog: Option<FieldDialog<'a>>,
    system_editor: Option<TextArea<'a>>,
    system_editor_roleplay: bool,
    edit_editor: Option<TextArea<'a>>,

    character_names: Vec<String>,
    character_slugs: Vec<String>,
    character_selected: usize,

    worldbook_list: Vec<String>,
    worldbook_selected: usize,

    character_editor: Option<FieldDialog<'a>>,
    character_editor_slug: String,
    worldbook_editor_entries: Vec<crate::worldinfo::Entry>,
    worldbook_editor_name: String,
    worldbook_editor_selected: usize,
    worldbook_entry_editor: Option<FieldDialog<'a>>,
    worldbook_entry_editor_index: usize,

    chat_content_cache: Option<render::ChatContentCache>,
    sidebar_cache: Option<render::SidebarCache>,
    raw_edit_node: Option<NodeId>,
    nav_cursor: Option<NodeId>,
    branch_dialog_items: Vec<(NodeId, String)>,
    branch_dialog_selected: usize,
    delete_confirm_selected: usize,
    delete_confirm_filename: String,
    user_name: Option<String>,
    config: crate::config::Config,
    worldbook_cache: Option<WorldbookCache>,
    bg_tx: mpsc::Sender<BackgroundEvent>,
}

const STATUS_DURATION: std::time::Duration = std::time::Duration::from_secs(5);
const STREAM_REDRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

impl App<'_> {
    fn set_status(&mut self, text: String, level: StatusLevel) {
        self.status_message = Some(StatusMessage {
            text,
            level,
            expires: std::time::Instant::now() + STATUS_DURATION,
        });
    }

    fn invalidate_chat_cache(&mut self) {
        self.chat_content_cache = None;
    }

    fn invalidate_sidebar_cache(&mut self) {
        self.sidebar_cache = None;
    }

    fn invalidate_worldbook_cache(&mut self) {
        self.worldbook_cache = None;
    }
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    save_mode: SaveMode,
    template: Template,
    sampling: SamplingParams,
) -> Result<()> {
    let sidebar_sessions = business::discover_sidebar_sessions(&save_mode);

    let mut textarea = TextArea::default();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input ")
            .title_bottom(Line::from(" Enter to send, Alt+Enter for newline ").centered()),
    );
    configure_textarea(&mut textarea);

    let sidebar_state = ratatui::widgets::ListState::default();

    let (token_tx, mut token_rx) = mpsc::channel::<StreamToken>(256);
    let (bg_tx, mut bg_rx) = mpsc::channel::<BackgroundEvent>(64);

    {
        let client = client.clone();
        let tx = bg_tx.clone();
        tokio::spawn(async move {
            let result = client.fetch_model_name().await;
            let _ = tx.send(BackgroundEvent::ModelFetched(result)).await;
        });
    }

    let config = crate::config::load();
    let user_name = config.user_name.clone();

    let initial_passkey_setup =
        save_mode.needs_passkey() && !crate::config::key_check_path().exists();

    let mut app = App {
        client,
        session,
        focus: if save_mode.needs_passkey() {
            if initial_passkey_setup {
                Focus::SetPasskeyDialog
            } else {
                Focus::PasskeyDialog
            }
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
        last_scroll_state: ScrollState {
            auto_scroll: false,
            nav_cursor: None,
            head: None,
            buffer_len: 0,
            width: 0,
            height: 0,
        },
        sidebar_sessions,
        sidebar_state,
        streaming_buffer: String::new(),
        is_streaming: false,
        model_name: None,
        api_available: true,
        api_error: String::new(),
        status_message: None,
        should_quit: false,
        passkey_changed: false,
        command_picker_selected: 0,
        passkey_input: String::new(),
        passkey_error: String::new(),
        passkey_deriving: false,
        set_passkey_input: String::new(),
        set_passkey_confirm: String::new(),
        set_passkey_active_field: 0,
        set_passkey_error: String::new(),
        set_passkey_deriving: false,
        set_passkey_is_initial: initial_passkey_setup,
        config_dialog: None,
        self_dialog: None,
        system_editor: None,
        system_editor_roleplay: false,
        edit_editor: None,
        character_names: Vec::new(),
        character_slugs: Vec::new(),
        character_selected: 0,
        worldbook_list: Vec::new(),
        worldbook_selected: 0,
        character_editor: None,
        character_editor_slug: String::new(),
        worldbook_editor_entries: Vec::new(),
        worldbook_editor_name: String::new(),
        worldbook_editor_selected: 0,
        worldbook_entry_editor: None,
        worldbook_entry_editor_index: 0,
        chat_content_cache: None,
        sidebar_cache: None,
        raw_edit_node: None,
        nav_cursor: None,
        branch_dialog_items: Vec::new(),
        branch_dialog_selected: 0,
        delete_confirm_selected: 0,
        delete_confirm_filename: String::new(),
        user_name,
        config,
        worldbook_cache: None,
        bg_tx: bg_tx.clone(),
    };

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut event_stream = EventStream::new();

    if let SaveMode::Encrypted { key, .. } = &app.save_mode {
        commands::spawn_metadata_loading(&app.sidebar_sessions, key, &bg_tx);
    }

    let mut frame_tick = tokio::time::interval(STREAM_REDRAW_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut needs_redraw = false;
    let mut stream_redraw_pending = false;

    terminal.draw(|f| render_frame(f, &mut app))?;

    loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                crate::debug_log::timed("event", "handle", || {
                    if let Some(action) = handle_event(event, &mut app, bg_tx.clone()) {
                        process_action(action, &mut app, token_tx.clone());
                    }
                });
                terminal.draw(|f| render_frame(f, &mut app))?;
                needs_redraw = false;
            }
            Some(stream_token) = token_rx.recv() => {
                crate::debug_log::timed("stream", "token", || {
                    commands::handle_stream_token(stream_token, &mut app)
                })?;
                stream_redraw_pending = true;
                needs_redraw = true;
            }
            Some(bg_event) = bg_rx.recv() => {
                commands::handle_background_event(bg_event, &mut app);
                terminal.draw(|f| render_frame(f, &mut app))?;
                needs_redraw = false;
            }
            _ = frame_tick.tick() => {
                if let Some(ref msg) = app.status_message {
                    if std::time::Instant::now() >= msg.expires {
                        app.status_message = None;
                        needs_redraw = true;
                    }
                }
                if needs_redraw && (stream_redraw_pending || app.status_message.is_none()) {
                    terminal.draw(|f| render_frame(f, &mut app))?;
                    needs_redraw = false;
                    stream_redraw_pending = false;
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

    if app.passkey_changed {
        println!("Passkey changed. Please re-launch to authenticate with your new passkey.");
    }

    Ok(())
}

fn render_frame(f: &mut ratatui::Frame, app: &mut App) {
    let _frame_start = std::time::Instant::now();

    let (outer, columns, right_split) = crate::debug_log::timed("layout", "splits", || {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(1)])
            .split(f.area());
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(30)])
            .split(outer[0]);
        let right_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(INPUT_HEIGHT)])
            .split(columns[1]);
        (outer, columns, right_split)
    });

    let status_area = outer[1];
    let sidebar_area = columns[0];
    let chat_area = right_split[0];
    let input_area = right_split[1];

    let session_count = app.sidebar_sessions.len();
    crate::debug_log::timed("sidebar", &format!("{session_count} sessions"), || {
        render::render_sidebar(f, app, sidebar_area);
    });

    let input_focused = app.focus == Focus::Input;
    let border = render::border_style(input_focused);
    let mut input_block = Block::default()
        .borders(Borders::ALL)
        .title(" Input ")
        .border_style(border);
    if input_focused {
        let hint = if app.nav_cursor.is_some() {
            " Enter to edit, Esc to cancel "
        } else {
            " Up arrow to edit, Enter to send "
        };
        input_block = input_block.title_bottom(Line::from(hint).centered());
    }
    app.textarea.set_block(input_block);
    f.render_widget(&app.textarea, input_area);

    let current_scroll_state = ScrollState {
        auto_scroll: app.auto_scroll,
        nav_cursor: app.nav_cursor,
        head: app.session.tree.head(),
        buffer_len: app.streaming_buffer.len(),
        width: chat_area.width,
        height: chat_area.height,
    };
    let scroll_dirty = current_scroll_state != app.last_scroll_state;
    let mut chat_scroll = app.chat_scroll;

    let mut cache = app.chat_content_cache.take();
    {
        let branch_ids = app.session.tree.current_branch_ids();
        let branch_info = app.session.tree.current_deepest_branch_info();
        let msg_count = branch_ids.len();
        crate::debug_log::log("chat.branch", &format!("{msg_count} nodes in path"));

        crate::debug_log::timed(
            "chat",
            &format!("{msg_count} msgs, scroll_dirty={scroll_dirty}"),
            || {
                render::render_chat(
                    f,
                    app,
                    chat_area,
                    &mut chat_scroll,
                    branch_ids,
                    scroll_dirty,
                    &mut cache,
                );
            },
        );

        crate::debug_log::timed("status", "bar", || {
            render::render_status_bar(f, app, status_area, branch_ids, branch_info);
        });
    }
    app.chat_content_cache = cache;
    app.chat_scroll = chat_scroll;
    app.last_scroll_state = current_scroll_state;

    if app.focus == Focus::Input && input::input_has_command_picker(app) {
        crate::debug_log::timed("picker", "command picker", || {
            render::render_command_picker(f, app, &app.textarea.lines()[0], chat_area);
        });
    }

    let dialog_name = match app.focus {
        Focus::PasskeyDialog => Some("passkey"),
        Focus::SetPasskeyDialog => Some("set_passkey"),
        Focus::ConfigDialog => Some("config"),
        Focus::SelfDialog => Some("self"),
        Focus::CharacterDialog => Some("character"),
        Focus::CharacterEditorDialog => Some("character_editor"),
        Focus::WorldbookDialog => Some("worldbook"),
        Focus::WorldbookEditorDialog => Some("worldbook_editor"),
        Focus::WorldbookEntryEditorDialog => Some("worldbook_entry_editor"),
        Focus::WorldbookEntryDeleteDialog => Some("worldbook_entry_delete"),
        Focus::SystemDialog => Some("system"),
        Focus::EditDialog => Some("edit"),
        Focus::BranchDialog => Some("branch"),
        Focus::DeleteConfirmDialog => Some("delete_confirm"),
        Focus::ApiErrorDialog => Some("api_error"),
        Focus::LoadingDialog => Some("loading"),
        _ => None,
    };

    if let Some(name) = dialog_name {
        crate::debug_log::timed("dialog", name, || {
            render_dialog(f, app);
        });
    }

    let frame_ms = _frame_start.elapsed().as_micros() as f64 / 1000.0;
    crate::debug_log::log("frame", &format!("{frame_ms:.3}ms total"));
}

fn render_dialog(f: &mut ratatui::Frame, app: &App) {
    match app.focus {
        Focus::PasskeyDialog => {
            dialogs::passkey::render_passkey_dialog(f, app, f.area());
        }
        Focus::SetPasskeyDialog => {
            dialogs::set_passkey::render_set_passkey_dialog(f, app, f.area());
        }
        Focus::ConfigDialog => {
            if let Some(ref dialog) = app.config_dialog {
                dialog.render(f, f.area());
            }
        }
        Focus::SelfDialog => {
            if let Some(ref dialog) = app.self_dialog {
                dialog.render(f, f.area());
            }
        }
        Focus::CharacterDialog => {
            dialogs::character::render_character_dialog(f, app, f.area());
        }
        Focus::CharacterEditorDialog => {
            if let Some(ref dialog) = app.character_editor {
                dialog.render(f, f.area());
            }
        }
        Focus::WorldbookDialog => {
            dialogs::worldbook::render_worldbook_dialog(f, app, f.area());
        }
        Focus::WorldbookEditorDialog => {
            dialogs::worldbook::render_worldbook_editor(f, app, f.area());
        }
        Focus::WorldbookEntryEditorDialog => {
            if let Some(ref dialog) = app.worldbook_entry_editor {
                dialog.render(f, f.area());
            }
        }
        Focus::WorldbookEntryDeleteDialog => {
            dialogs::worldbook::render_entry_delete_dialog(f, app, f.area());
        }
        Focus::SystemDialog => {
            dialogs::system::render_system_dialog(f, app, f.area());
        }
        Focus::EditDialog => {
            dialogs::edit::render_edit_dialog(f, app, f.area());
        }
        Focus::BranchDialog => {
            dialogs::branch::render_branch_dialog(f, app, f.area());
        }
        Focus::DeleteConfirmDialog => {
            dialogs::delete_confirm::render_delete_confirm_dialog(f, app, f.area());
        }
        Focus::ApiErrorDialog => {
            dialogs::api_error::render_api_error_dialog(f, app, f.area());
        }
        Focus::LoadingDialog => {
            dialogs::api_error::render_loading_dialog(f, f.area());
        }
        _ => {}
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
        Action::EditMessage { node_id, content } => {
            if let Some(new_root) = app.session.tree.duplicate_subtree(node_id) {
                if app.session.tree.set_message_content(new_root, content) {
                    app.session.tree.switch_to(new_root);
                    app.nav_cursor = Some(new_root);
                    app.focus = Focus::Chat;
                    let _ = app.session.maybe_save(&app.save_mode);
                }
            }
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
        return dialogs::passkey::handle_passkey_key(key, app, bg_tx.clone());
    }
    if app.focus == Focus::SetPasskeyDialog {
        return dialogs::set_passkey::handle_set_passkey_key(key, app, bg_tx);
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
    if app.focus == Focus::CharacterEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::CharacterEditor);
    }
    if app.focus == Focus::WorldbookDialog {
        return dialogs::worldbook::handle_worldbook_dialog_key(key, app);
    }
    if app.focus == Focus::WorldbookEditorDialog {
        return dialogs::worldbook::handle_worldbook_editor_key(key, app);
    }
    if app.focus == Focus::WorldbookEntryEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::WorldbookEntryEditor);
    }
    if app.focus == Focus::WorldbookEntryDeleteDialog {
        return dialogs::worldbook::handle_entry_delete_key(key, app);
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
    if app.focus == Focus::DeleteConfirmDialog {
        return dialogs::delete_confirm::handle_delete_confirm_key(key, app);
    }
    if app.focus == Focus::ApiErrorDialog {
        return dialogs::api_error::handle_api_error_key(key, app);
    }
    if app.focus == Focus::LoadingDialog {
        return dialogs::api_error::handle_loading_key(key);
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
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        app.session.tree.switch_sibling(1);
        let _ = app.session.maybe_save(&app.save_mode);
        return None;
    }

    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Input => {
                app.nav_cursor = app.session.tree.current_branch_ids().last().copied();
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
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        _ => None,
    }
}

fn cancel_generation(app: &mut App) {
    app.streaming_buffer.clear();
    app.is_streaming = false;
    app.session.tree.pop_head();
    app.auto_scroll = true;
}

fn open_edit_dialog_with(app: &mut App, content: &str) {
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let mut editor = TextArea::from(if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    });
    configure_textarea_at_end(&mut editor);
    app.edit_editor = Some(editor);
    app.focus = Focus::EditDialog;
}

fn configure_textarea(ta: &mut TextArea<'_>) {
    ta.set_cursor_line_style(Style::default());
    ta.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);
}

fn configure_textarea_at_end(ta: &mut TextArea<'_>) {
    configure_textarea(ta);
    ta.move_cursor(tui_textarea::CursorMove::Bottom);
    ta.move_cursor(tui_textarea::CursorMove::End);
}

enum DialogKind {
    Config,
    SelfPersona,
    CharacterEditor,
    WorldbookEntryEditor,
}

fn handle_field_dialog_key(key: KeyEvent, app: &mut App, kind: DialogKind) -> Option<Action> {
    let dialog = match kind {
        DialogKind::Config => app.config_dialog.as_mut(),
        DialogKind::SelfPersona => app.self_dialog.as_mut(),
        DialogKind::CharacterEditor => app.character_editor.as_mut(),
        DialogKind::WorldbookEntryEditor => app.worldbook_entry_editor.as_mut(),
    };

    let Some(dialog) = dialog else {
        return None;
    };

    let result = dialog.handle_key(key);

    if matches!(kind, DialogKind::WorldbookEntryEditor) {
        if let Some(ref mut d) = app.worldbook_entry_editor {
            let selective = d
                .values
                .get(2)
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));
            d.hidden_fields = if selective { Vec::new() } else { vec![3] };
        }
    }

    match result {
        dialogs::FieldDialogAction::Continue => None,
        dialogs::FieldDialogAction::Close => {
            match kind {
                DialogKind::Config => {
                    let values = &app.config_dialog.as_ref().unwrap().values;
                    match business::save_config_from_fields(values) {
                        Ok(()) => {
                            business::apply_config(app);
                            app.set_status("Configuration saved.".to_owned(), StatusLevel::Info);
                        }
                        Err(e) => {
                            app.set_status(
                                format!("Failed to save config: {e}"),
                                StatusLevel::Error,
                            );
                        }
                    }
                    app.config_dialog = None;
                }
                DialogKind::SelfPersona => {
                    let values = &app.self_dialog.as_ref().unwrap().values;
                    match business::save_self_fields(values) {
                        Ok(()) => {
                            app.config = crate::config::load();
                            app.user_name = app.config.user_name.clone();
                            app.invalidate_chat_cache();
                            app.set_status("User persona saved.".to_owned(), StatusLevel::Info);
                        }
                        Err(e) => {
                            app.set_status(
                                format!("Failed to save persona: {e}"),
                                StatusLevel::Error,
                            );
                        }
                    }
                    app.self_dialog = None;
                }
                DialogKind::CharacterEditor => {
                    let values = &app.character_editor.as_ref().unwrap().values;
                    let card = crate::character::CharacterCard {
                        name: values[0].clone(),
                        description: values[1].clone(),
                        personality: values[2].clone(),
                        scenario: values[3].clone(),
                        first_mes: values[4].clone(),
                        mes_example: values[5].clone(),
                        system_prompt: values[6].clone(),
                        post_history_instructions: values[7].clone(),
                        alternate_greetings: Vec::new(),
                    };
                    match crate::character::save_card(
                        &card,
                        &crate::config::characters_dir(),
                        app.save_mode.key(),
                    ) {
                        Ok(_) => app.set_status(
                            format!("Saved character: {}", card.name),
                            StatusLevel::Info,
                        ),
                        Err(e) => app.set_status(
                            format!("Failed to save character: {e}"),
                            StatusLevel::Error,
                        ),
                    }
                    app.character_editor = None;
                    app.focus = Focus::CharacterDialog;
                    return None;
                }
                DialogKind::WorldbookEntryEditor => {
                    let values = &app.worldbook_entry_editor.as_ref().unwrap().values;
                    let idx = app.worldbook_entry_editor_index;
                    if idx < app.worldbook_editor_entries.len() {
                        app.worldbook_editor_entries[idx] = dialogs::worldbook::values_to_entry(
                            values,
                            &app.worldbook_editor_entries[idx],
                        );
                    }
                    app.worldbook_entry_editor = None;
                    app.focus = Focus::WorldbookEditorDialog;
                    return None;
                }
            }
            app.focus = Focus::Input;
            None
        }
    }
}
