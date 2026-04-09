pub mod business;
mod clipboard;
pub mod commands;
mod dialogs;
mod input;
mod maintenance;
mod render;

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};
use tokio::sync::mpsc;
use tui_textarea::{CursorMove, TextArea};

use crate::cli::CliOverrides;
use crate::client::{ApiClient, StreamToken};
use crate::context::ContextManager;
use crate::preset::InstructPreset;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, NodeId, Role, SaveMode, Session, SessionEntry};
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
    PresetPickerDialog,
    PresetEditorDialog,
    PersonaDialog,
    PersonaEditorDialog,
    CharacterDialog,
    CharacterEditorDialog,
    WorldbookDialog,
    WorldbookEditorDialog,
    WorldbookEntryEditorDialog,
    WorldbookEntryDeleteDialog,
    SystemPromptDialog,
    SystemPromptEditorDialog,
    EditDialog,
    BranchDialog,
    DeleteConfirmDialog,
    EditConfirmDialog,
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

enum DeleteContext {
    Session,
    Character { slug: String },
    Persona { name: String },
    SystemPrompt { name: String },
    Worldbook { name: String },
    Preset { kind: dialogs::preset::PresetKind },
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
    created: std::time::Instant,
    expires: std::time::Instant,
}

struct WorldbookCache {
    enabled_names: Vec<String>,
    books: Vec<RuntimeWorldBook>,
}

#[derive(Clone, Copy)]
enum SaveTrigger {
    Debounced,
    Explicit,
    StreamDone,
    Exit,
    Transition,
    Unlock,
    Retry,
}

impl SaveTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debounced => "debounced",
            Self::Explicit => "explicit",
            Self::StreamDone => "stream_done",
            Self::Exit => "exit",
            Self::Transition => "transition",
            Self::Unlock => "unlock",
            Self::Retry => "retry",
        }
    }
}

struct AutosaveDebugState {
    dirty_since: Option<std::time::Instant>,
    save_count: u64,
    retry_count: u64,
}

struct UnlockDebugState {
    kind: &'static str,
    started_at: std::time::Instant,
}

struct HydrationDebugState {
    generation: u64,
    started_at: std::time::Instant,
    scheduled: usize,
    completed: usize,
    failed: usize,
    stale_dropped: usize,
    missing_dropped: usize,
    batch_total: usize,
    batch_finished: usize,
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
        generation: u64,
        path: std::path::PathBuf,
        metadata: session::SessionMetadata,
    },
    MetadataBatchFinished {
        generation: u64,
        loaded_count: usize,
        failed_count: usize,
    },
    MaintenanceFinished(maintenance::MaintenanceUpdate),
    ModelFetched(std::result::Result<String, String>),
}

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

struct LayoutAreas {
    sidebar: Rect,
    chat: Rect,
    input: Rect,
}

struct App<'a> {
    client: ApiClient,
    session: &'a mut Session,
    save_mode: SaveMode,
    session_dirty: bool,
    pending_save_deadline: Option<std::time::Instant>,
    pending_save_trigger: Option<SaveTrigger>,
    instruct_preset: InstructPreset,
    stop_tokens: Vec<String>,
    sampling: SamplingParams,
    context_mgr: ContextManager,

    focus: Focus,
    textarea: TextArea<'a>,
    chat_scroll: u16,
    chat_max_scroll: u16,
    auto_scroll: bool,
    last_scroll_state: ScrollState,
    sidebar_sessions: Vec<SessionEntry>,
    sidebar_hydration_generation: u64,
    sidebar_state: ratatui::widgets::ListState,
    streaming_buffer: String,
    is_streaming: bool,
    is_continuation: bool,
    message_queue: Vec<String>,
    streaming_task: Option<tokio::task::JoinHandle<()>>,
    model_name: Option<String>,
    api_available: bool,
    api_error: String,
    status_message: Option<StatusMessage>,
    should_quit: bool,
    passkey_changed: bool,
    re_encrypt_old_key: Option<std::sync::Arc<crate::crypto::DerivedKey>>,
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
    persona_editor: Option<FieldDialog<'a>>,
    system_prompt_editor: Option<FieldDialog<'a>>,
    system_editor_prompt_name: String,
    system_editor_return_focus: Focus,
    system_editor_read_only: bool,

    system_prompt_list: Vec<String>,
    system_prompt_selected: usize,
    edit_editor: Option<TextArea<'a>>,

    preset_picker_kind: dialogs::preset::PresetKind,
    preset_picker_names: Vec<String>,
    preset_picker_selected: usize,
    preset_editor: Option<FieldDialog<'a>>,
    preset_editor_kind: dialogs::preset::PresetKind,
    preset_editor_original_name: String,

    character_names: Vec<String>,
    character_slugs: Vec<String>,
    character_selected: usize,

    worldbook_list: Vec<String>,
    worldbook_selected: usize,

    character_editor: Option<FieldDialog<'a>>,
    character_editor_slug: String,
    worldbook_editor_entries: Vec<crate::worldinfo::Entry>,
    worldbook_editor_original_entries: Vec<crate::worldinfo::Entry>,
    worldbook_editor_name: String,
    worldbook_editor_original_name: String,
    worldbook_editor_name_selected: bool,
    worldbook_editor_name_editing: bool,
    worldbook_editor_selected: usize,
    worldbook_entry_editor: Option<FieldDialog<'a>>,
    worldbook_entry_editor_index: usize,

    chat_content_cache: Option<render::ChatContentCache>,
    cached_token_count: Option<usize>,
    sidebar_cache: Option<render::SidebarCache>,
    raw_edit_node: Option<NodeId>,
    edit_original_content: String,
    edit_confirm_selected: usize,
    nav_cursor: Option<NodeId>,
    branch_dialog_items: Vec<(NodeId, String)>,
    branch_dialog_selected: usize,
    delete_confirm_selected: usize,
    delete_confirm_filename: String,
    delete_context: DeleteContext,
    active_persona_name: Option<String>,
    active_persona_desc: Option<String>,
    persona_list: Vec<String>,
    persona_selected: usize,
    persona_editor_file_name: String,
    config: crate::config::Config,
    cli_overrides: CliOverrides,
    worldbook_cache: Option<WorldbookCache>,
    bg_tx: mpsc::Sender<BackgroundEvent>,
    layout_areas: Option<LayoutAreas>,
    hover_node: Option<NodeId>,
    autosave_debug: AutosaveDebugState,
    unlock_debug: Option<UnlockDebugState>,
    hydration_debug: Option<HydrationDebugState>,
    input_reject_flash: Option<std::time::Instant>,
}

const STATUS_DURATION: std::time::Duration = std::time::Duration::from_secs(5);
const NOTIFICATION_SLIDE_DURATION: std::time::Duration = std::time::Duration::from_millis(300);
const STREAM_REDRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(350);
const AUTOSAVE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

impl App<'_> {
    fn can_persist_session(&self) -> bool {
        matches!(
            self.save_mode,
            SaveMode::Plaintext(_) | SaveMode::Encrypted { .. }
        )
    }

    fn tick_reject_flashes(&mut self) -> bool {
        let mut needs_redraw = false;
        if let Some(t) = self.input_reject_flash {
            if dialogs::is_flash_active(Some(t)) {
                needs_redraw = true;
            } else {
                self.input_reject_flash = None;
                needs_redraw = true;
            }
        }
        for dialog in [
            &mut self.config_dialog,
            &mut self.persona_editor,
            &mut self.system_prompt_editor,
            &mut self.character_editor,
            &mut self.worldbook_entry_editor,
        ] {
            if let Some(d) = dialog.as_mut() {
                if let Some(t) = d.reject_flash {
                    if dialogs::is_flash_active(Some(t)) {
                        needs_redraw = true;
                    } else {
                        d.reject_flash = None;
                        needs_redraw = true;
                    }
                }
            }
        }
        needs_redraw
    }

    const MAX_STATUS_LENGTH: usize = 64;

    fn set_status(&mut self, text: String, level: StatusLevel) {
        let now = std::time::Instant::now();
        let created = if self.status_message.is_some() {
            now - NOTIFICATION_SLIDE_DURATION
        } else {
            now
        };
        let truncated = if text.len() > Self::MAX_STATUS_LENGTH {
            let end = text.floor_char_boundary(Self::MAX_STATUS_LENGTH - 3);
            format!("{}...", &text[..end])
        } else {
            text
        };
        self.status_message = Some(StatusMessage {
            text: truncated,
            level,
            created,
            expires: now + STATUS_DURATION,
        });
    }

    fn invalidate_chat_cache(&mut self) {
        self.chat_content_cache = None;
        self.cached_token_count = None;
    }

    fn invalidate_sidebar_cache(&mut self) {
        self.sidebar_cache = None;
    }

    fn invalidate_worldbook_cache(&mut self) {
        self.worldbook_cache = None;
    }

    fn mark_session_dirty(&mut self, trigger: SaveTrigger, immediate: bool) {
        self.session_dirty = true;
        self.pending_save_trigger = Some(trigger);
        if self.can_persist_session() {
            let deadline = if immediate {
                std::time::Instant::now()
            } else {
                std::time::Instant::now() + AUTOSAVE_DEBOUNCE
            };
            self.pending_save_deadline = Some(deadline);
        }
        if self.autosave_debug.dirty_since.is_none() {
            self.autosave_debug.dirty_since = Some(std::time::Instant::now());
        }
        crate::debug_log::log_kv(
            "autosave",
            &[
                crate::debug_log::field("phase", "schedule"),
                crate::debug_log::field("trigger", trigger.as_str()),
                crate::debug_log::field("persistable", self.can_persist_session()),
                crate::debug_log::field("session_dirty", self.session_dirty),
            ],
        );
    }

    fn discard_pending_session_save(&mut self) {
        self.session_dirty = false;
        self.pending_save_deadline = None;
        self.pending_save_trigger = None;
        self.autosave_debug.dirty_since = None;
    }

    fn flush_session_save(&mut self, trigger: SaveTrigger) -> Result<()> {
        if !self.session_dirty || !self.can_persist_session() {
            crate::debug_log::log_kv(
                "autosave",
                &[
                    crate::debug_log::field("phase", "flush"),
                    crate::debug_log::field("trigger", trigger.as_str()),
                    crate::debug_log::field("result", "skipped"),
                    crate::debug_log::field("session_dirty", self.session_dirty),
                    crate::debug_log::field("persistable", self.can_persist_session()),
                ],
            );
            return Ok(());
        }

        let dirty_elapsed_ms = self
            .autosave_debug
            .dirty_since
            .map(|started| started.elapsed().as_secs_f64() * 1000.0);

        let path = self.save_mode.path().map(|path| path.display().to_string());
        let start = std::time::Instant::now();
        let result = self.session.maybe_save(&self.save_mode);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        match result {
            Ok(()) => {
                self.autosave_debug.save_count += 1;
                let mut fields = vec![
                    crate::debug_log::field("phase", "flush"),
                    crate::debug_log::field("trigger", trigger.as_str()),
                    crate::debug_log::field("result", "ok"),
                    crate::debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                ];
                if let Some(path) = path.as_deref() {
                    fields.push(crate::debug_log::field("path", path));
                }
                if let Some(dirty_elapsed_ms) = dirty_elapsed_ms {
                    fields.push(crate::debug_log::field(
                        "dirty_elapsed_ms",
                        format!("{dirty_elapsed_ms:.3}"),
                    ));
                }
                fields.push(crate::debug_log::field(
                    "save_count",
                    self.autosave_debug.save_count,
                ));
                crate::debug_log::log_kv("autosave", &fields);
                self.discard_pending_session_save();
                Ok(())
            }
            Err(err) => {
                self.pending_save_deadline = Some(std::time::Instant::now() + AUTOSAVE_RETRY_DELAY);
                self.pending_save_trigger = Some(SaveTrigger::Retry);
                self.autosave_debug.retry_count += 1;
                let mut fields = vec![
                    crate::debug_log::field("phase", "flush"),
                    crate::debug_log::field("trigger", trigger.as_str()),
                    crate::debug_log::field("result", "error"),
                    crate::debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                    crate::debug_log::field("retry_delay_ms", AUTOSAVE_RETRY_DELAY.as_millis()),
                    crate::debug_log::field("error", &err),
                ];
                if let Some(path) = path.as_deref() {
                    fields.push(crate::debug_log::field("path", path));
                }
                if let Some(dirty_elapsed_ms) = dirty_elapsed_ms {
                    fields.push(crate::debug_log::field(
                        "dirty_elapsed_ms",
                        format!("{dirty_elapsed_ms:.3}"),
                    ));
                }
                fields.push(crate::debug_log::field(
                    "retry_count",
                    self.autosave_debug.retry_count,
                ));
                crate::debug_log::log_kv("autosave", &fields);
                Err(err)
            }
        }
    }

    fn flush_session_before_transition(&mut self) -> bool {
        match self.flush_session_save(SaveTrigger::Transition) {
            Ok(()) => true,
            Err(err) => {
                self.set_status(format!("Save error: {err}"), StatusLevel::Error);
                false
            }
        }
    }
}

pub fn build_effective_system_prompt_standalone(
    session: &Session,
    key: Option<&crate::crypto::DerivedKey>,
) -> Option<String> {
    business::build_effective_system_prompt(session, key)
}

pub async fn run(
    client: ApiClient,
    session: &mut Session,
    save_mode: SaveMode,
    instruct_preset: InstructPreset,
    sampling: SamplingParams,
    cli_overrides: CliOverrides,
) -> Result<()> {
    let sidebar_sessions = crate::debug_log::timed_kv(
        "startup.phase",
        &[crate::debug_log::field("phase", "sidebar_discovery")],
        || business::discover_sidebar_sessions(&save_mode),
    );

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

    let key_check_exists = crate::config::key_check_path().exists();
    let has_encrypted_sessions = !key_check_exists
        && std::fs::read_dir(crate::config::sessions_dir())
            .ok()
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.path().extension().and_then(|ext| ext.to_str()) == Some("session")
                        && std::fs::read(e.path())
                            .ok()
                            .is_some_and(|data| crate::crypto::is_encrypted(&data))
                })
            })
            .unwrap_or(false);
    let initial_passkey_setup =
        save_mode.needs_passkey() && !key_check_exists && !has_encrypted_sessions;

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
        session_dirty: false,
        pending_save_deadline: None,
        pending_save_trigger: None,
        stop_tokens: instruct_preset.stop_tokens(),
        instruct_preset,
        sampling,
        context_mgr: ContextManager::default(),
        textarea,
        chat_scroll: 0,
        chat_max_scroll: 0,
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
        sidebar_hydration_generation: 0,
        sidebar_state,
        streaming_buffer: String::new(),
        is_streaming: false,
        is_continuation: false,
        message_queue: Vec::new(),
        streaming_task: None,
        model_name: None,
        api_available: true,
        api_error: String::new(),
        status_message: None,
        should_quit: false,
        passkey_changed: false,
        re_encrypt_old_key: None,
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
        persona_editor: None,
        system_prompt_editor: None,
        system_editor_prompt_name: String::new(),
        system_editor_return_focus: Focus::Input,
        system_editor_read_only: false,
        system_prompt_list: Vec::new(),
        system_prompt_selected: 0,
        edit_editor: None,
        preset_picker_kind: dialogs::preset::PresetKind::Instruct,
        preset_picker_names: Vec::new(),
        preset_picker_selected: 0,
        preset_editor: None,
        preset_editor_kind: dialogs::preset::PresetKind::Instruct,
        preset_editor_original_name: String::new(),
        character_names: Vec::new(),
        character_slugs: Vec::new(),
        character_selected: 0,
        worldbook_list: Vec::new(),
        worldbook_selected: 0,
        character_editor: None,
        character_editor_slug: String::new(),
        worldbook_editor_entries: Vec::new(),
        worldbook_editor_original_entries: Vec::new(),
        worldbook_editor_name: String::new(),
        worldbook_editor_original_name: String::new(),
        worldbook_editor_name_selected: true,
        worldbook_editor_name_editing: false,
        worldbook_editor_selected: 0,
        worldbook_entry_editor: None,
        worldbook_entry_editor_index: 0,
        chat_content_cache: None,
        cached_token_count: None,
        sidebar_cache: None,
        raw_edit_node: None,
        edit_original_content: String::new(),
        edit_confirm_selected: 0,
        nav_cursor: None,
        branch_dialog_items: Vec::new(),
        branch_dialog_selected: 0,
        delete_confirm_selected: 0,
        delete_confirm_filename: String::new(),
        delete_context: DeleteContext::Session,
        active_persona_name: None,
        active_persona_desc: None,
        persona_list: Vec::new(),
        persona_selected: 0,
        persona_editor_file_name: String::new(),
        config,
        cli_overrides,
        worldbook_cache: None,
        layout_areas: None,
        hover_node: None,
        bg_tx: bg_tx.clone(),
        autosave_debug: AutosaveDebugState {
            dirty_since: None,
            save_count: 0,
            retry_count: 0,
        },
        unlock_debug: None,
        hydration_debug: None,
        input_reject_flash: None,
    };

    business::load_active_persona(&mut app);

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut event_stream = EventStream::new();

    crate::debug_log::timed_kv(
        "startup.phase",
        &[crate::debug_log::field("phase", "metadata_schedule")],
        || commands::spawn_metadata_loading(&mut app),
    );

    let mut frame_tick = tokio::time::interval(STREAM_REDRAW_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut needs_redraw = false;

    crate::debug_log::timed_result(
        "startup.phase",
        &[crate::debug_log::field("phase", "first_draw")],
        || terminal.draw(|f| render_frame(f, &mut app)),
    )?;
    crate::debug_log::timed_kv(
        "startup.phase",
        &[crate::debug_log::field("phase", "maintenance_schedule")],
        || maintenance::spawn_startup_maintenance(&app.save_mode, &bg_tx),
    );

    loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                let is_mouse_move = matches!(&event, Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved));
                crate::debug_log::timed_kv("event", &[crate::debug_log::field("phase", "handle")], || {
                    if let Some(action) = handle_event(event, &mut app, bg_tx.clone()) {
                        process_action(action, &mut app, token_tx.clone());
                    }
                });
                if is_mouse_move {
                    needs_redraw = true;
                } else {
                    terminal.draw(|f| render_frame(f, &mut app))?;
                    needs_redraw = false;
                }
            }
            Some(stream_token) = token_rx.recv() => {
                crate::debug_log::timed_result("stream", &[crate::debug_log::field("phase", "token")], || {
                    commands::handle_stream_token(stream_token, &mut app, token_tx.clone())
                })?;
                needs_redraw = true;
            }
            Some(bg_event) = bg_rx.recv() => {
                commands::handle_background_event(bg_event, &mut app);
                terminal.draw(|f| render_frame(f, &mut app))?;
                needs_redraw = false;
            }
            _ = frame_tick.tick() => {
                if app.pending_save_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                    let trigger = app.pending_save_trigger.unwrap_or(SaveTrigger::Retry);
                    if let Err(err) = app.flush_session_save(trigger) {
                        app.set_status(format!("Save error: {err}"), StatusLevel::Error);
                    }
                    needs_redraw = true;
                }
                if let Some(ref msg) = app.status_message {
                    if std::time::Instant::now() >= msg.expires {
                        app.status_message = None;
                    }
                    needs_redraw = true;
                }
                if app.tick_reject_flashes() {
                    needs_redraw = true;
                }
                if needs_redraw {
                    terminal.draw(|f| render_frame(f, &mut app))?;
                    needs_redraw = false;
                }
            }
        }

        if app.should_quit {
            match app.flush_session_save(SaveTrigger::Exit) {
                Ok(()) => break,
                Err(err) => {
                    app.should_quit = false;
                    app.set_status(format!("Save error: {err}"), StatusLevel::Error);
                    needs_redraw = true;
                }
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste
    )?;

    if app.passkey_changed {
        println!("Passkey changed. Please re-launch to authenticate with your new passkey.");
    }

    Ok(())
}

fn render_frame(f: &mut ratatui::Frame, app: &mut App) {
    let _frame_start = std::time::Instant::now();

    let (outer, columns, right_split) = crate::debug_log::timed_kv(
        "layout",
        &[crate::debug_log::field("phase", "splits")],
        || {
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
        },
    );

    let status_area = outer[1];
    let sidebar_area = columns[0];
    let chat_area = right_split[0];
    let input_area = right_split[1];

    app.layout_areas = Some(LayoutAreas {
        sidebar: sidebar_area,
        chat: chat_area,
        input: input_area,
    });

    let session_count = app.sidebar_sessions.len();
    crate::debug_log::timed_kv(
        "sidebar",
        &[crate::debug_log::field("session_count", session_count)],
        || {
            render::render_sidebar(f, app, sidebar_area);
        },
    );

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

    let (messages_area, queue_area) = render::split_chat_area_for_queue(chat_area, app);

    let current_scroll_state = ScrollState {
        auto_scroll: app.auto_scroll,
        nav_cursor: app.nav_cursor,
        head: app.session.tree.head(),
        buffer_len: app.streaming_buffer.len(),
        width: messages_area.width,
        height: messages_area.height,
    };
    let scroll_dirty = current_scroll_state != app.last_scroll_state;
    let mut chat_scroll = app.chat_scroll;

    let mut max_scroll = 0u16;
    let mut cache = app.chat_content_cache.take();
    {
        let branch_ids = app.session.tree.current_branch_ids();
        let branch_info = app.session.tree.current_deepest_branch_info();
        let msg_count = branch_ids.len();
        crate::debug_log::log_kv(
            "chat.branch",
            &[crate::debug_log::field("node_count", msg_count)],
        );
        crate::debug_log::timed_kv(
            "chat",
            &[
                crate::debug_log::field("message_count", msg_count),
                crate::debug_log::field("scroll_dirty", scroll_dirty),
            ],
            || {
                max_scroll = render::render_chat(
                    f,
                    app,
                    messages_area,
                    &mut chat_scroll,
                    branch_ids,
                    scroll_dirty,
                    &mut cache,
                );
                if let Some(queue_rect) = queue_area {
                    render::render_message_queue(f, app, queue_rect);
                }
            },
        );

        let token_count = *app.cached_token_count.get_or_insert_with(|| {
            ContextManager::estimate_tokens_for_messages(
                branch_ids
                    .iter()
                    .filter_map(|&id| app.session.tree.node(id).map(|node| &node.message)),
            )
        });

        crate::debug_log::timed_kv("status", &[crate::debug_log::field("phase", "bar")], || {
            render::render_status_bar(f, app, status_area, branch_info, token_count);
        });
    }
    app.chat_content_cache = cache;
    app.chat_scroll = chat_scroll;
    app.chat_max_scroll = max_scroll;
    app.last_scroll_state = current_scroll_state;

    if app.focus == Focus::Input && input::input_has_command_picker(app) {
        crate::debug_log::timed_kv(
            "picker",
            &[crate::debug_log::field("phase", "command_picker")],
            || {
                render::render_command_picker(f, app, &app.textarea.lines()[0], chat_area);
            },
        );
    }

    let dialog_name = match app.focus {
        Focus::PasskeyDialog => Some("passkey"),
        Focus::SetPasskeyDialog => Some("set_passkey"),
        Focus::ConfigDialog => Some("config"),
        Focus::PresetPickerDialog => Some("preset_picker"),
        Focus::PresetEditorDialog => Some("preset_editor"),
        Focus::PersonaDialog => Some("persona"),
        Focus::PersonaEditorDialog => Some("persona_editor"),
        Focus::CharacterDialog => Some("character"),
        Focus::CharacterEditorDialog => Some("character_editor"),
        Focus::WorldbookDialog => Some("worldbook"),
        Focus::WorldbookEditorDialog => Some("worldbook_editor"),
        Focus::WorldbookEntryEditorDialog => Some("worldbook_entry_editor"),
        Focus::WorldbookEntryDeleteDialog => Some("worldbook_entry_delete"),
        Focus::SystemPromptDialog => Some("system_prompt"),
        Focus::SystemPromptEditorDialog => Some("system_prompt_editor"),
        Focus::EditDialog => Some("edit"),
        Focus::EditConfirmDialog => Some("edit_confirm"),
        Focus::BranchDialog => Some("branch"),
        Focus::DeleteConfirmDialog => Some("delete_confirm"),
        Focus::ApiErrorDialog => Some("api_error"),
        Focus::LoadingDialog => Some("loading"),
        _ => None,
    };

    if let Some(name) = dialog_name {
        crate::debug_log::timed_kv("dialog", &[crate::debug_log::field("name", name)], || {
            render_dialog(f, app);
        });
    }

    let frame_ms = _frame_start.elapsed().as_micros() as f64 / 1000.0;
    crate::debug_log::log_kv(
        "frame",
        &[
            crate::debug_log::field("phase", "frame"),
            crate::debug_log::field("elapsed_ms", format!("{frame_ms:.3}")),
        ],
    );
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
        Focus::PresetPickerDialog => {
            dialogs::preset::render_preset_dialog(f, app, f.area());
        }
        Focus::PresetEditorDialog => {
            if let Some(ref dialog) = app.preset_editor {
                dialog.render(f, f.area());
            }
        }
        Focus::PersonaDialog => {
            dialogs::persona::render_persona_dialog(f, app, f.area());
        }
        Focus::PersonaEditorDialog => {
            if let Some(ref dialog) = app.persona_editor {
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
        Focus::SystemPromptDialog => {
            dialogs::system_prompt::render_system_prompt_dialog(f, app, f.area());
        }
        Focus::SystemPromptEditorDialog => {
            if let Some(ref dialog) = app.system_prompt_editor {
                dialog.render(f, f.area());
            }
        }
        Focus::EditDialog => {
            dialogs::edit::render_edit_dialog(f, app, f.area());
        }
        Focus::EditConfirmDialog => {
            dialogs::edit::render_edit_dialog(f, app, f.area());
            dialogs::edit::render_edit_confirm_dialog(f, app, f.area());
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
        Event::Paste(ref text) => handle_paste(text.clone(), event, app),
        Event::Mouse(mouse) => handle_mouse(mouse, app),
        _ => None,
    }
}

fn handle_paste(text: String, raw_event: Event, app: &mut App) -> Option<Action> {
    let cleaned = clean_pasted_path(&text);
    let path = std::path::Path::new(&cleaned);

    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let handled = match app.focus {
            Focus::CharacterDialog => dialogs::character::handle_character_paste(path, &ext, app),
            Focus::WorldbookDialog => dialogs::worldbook::handle_worldbook_paste(path, &ext, app),
            Focus::SystemPromptDialog => {
                dialogs::system_prompt::handle_system_prompt_paste(path, &ext, app)
            }
            Focus::PersonaDialog => dialogs::persona::handle_persona_paste(path, &ext, app),
            Focus::Sidebar => input::handle_sidebar_paste(path, &ext, app),
            _ => false,
        };

        if handled {
            return None;
        }
    }

    match app.focus {
        Focus::Input => {
            app.textarea.input(raw_event);
        }
        Focus::EditDialog => {
            if let Some(ref mut editor) = app.edit_editor {
                editor.insert_str(&text);
            }
        }
        Focus::PresetEditorDialog => {
            if let Some(ref mut d) = app.preset_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::PersonaEditorDialog => {
            if let Some(ref mut d) = app.persona_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::CharacterEditorDialog => {
            if let Some(ref mut d) = app.character_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::SystemPromptEditorDialog => {
            if let Some(ref mut d) = app.system_prompt_editor {
                d.insert_into_active_editor(&text);
            }
        }
        Focus::WorldbookEntryEditorDialog => {
            if let Some(ref mut d) = app.worldbook_entry_editor {
                d.insert_into_active_editor(&text);
            }
        }
        _ => {}
    }
    None
}

fn clean_pasted_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        trimmed.to_owned()
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
                    app.invalidate_chat_cache();
                    app.nav_cursor = Some(new_root);
                    app.focus = Focus::Chat;
                    app.mark_session_dirty(SaveTrigger::Debounced, false);
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
    if app.focus == Focus::PresetPickerDialog {
        return dialogs::preset::handle_preset_dialog_key(key, app);
    }
    if app.focus == Focus::PresetEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::PresetEditor);
    }
    if app.focus == Focus::ConfigDialog {
        return handle_field_dialog_key(key, app, DialogKind::Config);
    }
    if app.focus == Focus::PersonaDialog {
        return dialogs::persona::handle_persona_dialog_key(key, app);
    }
    if app.focus == Focus::PersonaEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::PersonaEditor);
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
    if app.focus == Focus::SystemPromptDialog {
        return dialogs::system_prompt::handle_system_prompt_dialog_key(key, app);
    }
    if app.focus == Focus::SystemPromptEditorDialog {
        return handle_field_dialog_key(key, app, DialogKind::SystemPromptEditor);
    }
    if app.focus == Focus::EditDialog {
        return dialogs::edit::handle_edit_key(key, app);
    }
    if app.focus == Focus::EditConfirmDialog {
        return dialogs::edit::handle_edit_confirm_key(key, app);
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
        if app.focus == Focus::Input && app.textarea.selection_range().is_some() {
            let (consumed, warning) = clipboard::handle_clipboard_key(&key, &mut app.textarea);
            if let Some(msg) = warning {
                app.set_status(msg, StatusLevel::Warning);
            }
            if consumed {
                return None;
            }
        }
        return Some(Action::Quit);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        let previous_head = app.session.tree.head();
        app.session.tree.switch_sibling(-1);
        if app.session.tree.head() != previous_head {
            app.mark_session_dirty(SaveTrigger::Debounced, false);
        }
        return None;
    }
    if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
        app.nav_cursor = None;
        let previous_head = app.session.tree.head();
        app.session.tree.switch_sibling(1);
        if app.session.tree.head() != previous_head {
            app.mark_session_dirty(SaveTrigger::Debounced, false);
        }
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
    if key.code == KeyCode::Esc {
        cancel_generation(app);
        if !app.message_queue.is_empty() {
            let next = app.message_queue.remove(0);
            return Some(Action::SendMessage(next));
        }
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }

    if key.code == KeyCode::Enter && key.modifiers.is_empty() {
        let lines: Vec<String> = app.textarea.lines().to_vec();
        let trimmed = lines.join("\n").trim().to_owned();

        if trimmed.is_empty() {
            return None;
        }

        if trimmed.starts_with('/') {
            app.set_status(
                "Slash commands cannot be queued during generation".to_owned(),
                StatusLevel::Warning,
            );
            return None;
        }

        app.textarea = TextArea::default();
        configure_textarea(&mut app.textarea);
        app.message_queue.push(trimmed);
        return None;
    }

    app.textarea.input(key);
    None
}

fn handle_mouse(mouse: MouseEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }
    let Some(ref areas) = app.layout_areas else {
        return None;
    };
    let sidebar = areas.sidebar;
    let chat = areas.chat;
    let input = areas.input;
    let pos = Position::new(mouse.column, mouse.row);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if is_dialog_focus(app.focus) {
                dialogs::handle_dialog_mouse_click(mouse, app);
                return None;
            }

            if sidebar.contains(pos) {
                app.focus = Focus::Sidebar;
                app.nav_cursor = None;
                let inner_row = mouse.row.saturating_sub(sidebar.y + 1) as usize;
                let offset = app.sidebar_state.offset();
                let selected_idx = app.sidebar_state.selected();
                let mut cumulative: usize = 0;
                let mut hit_index: Option<usize> = None;
                for i in offset..app.sidebar_sessions.len() {
                    let has_preview = selected_idx == Some(i)
                        && app.sidebar_sessions[i].sidebar_preview.is_some();
                    let item_height: usize = if has_preview { 2 } else { 1 };
                    if inner_row < cumulative + item_height {
                        hit_index = Some(i);
                        break;
                    }
                    cumulative += item_height;
                }
                if let Some(index) = hit_index {
                    if selected_idx != Some(index) {
                        app.sidebar_state.select(Some(index));
                        input::load_sidebar_selection(app);
                    }
                }
            } else if chat.contains(pos) {
                app.focus = Focus::Chat;
                if let Some(ref cache) = app.chat_content_cache {
                    let branch_ids = app.session.tree.current_branch_ids();
                    if let Some(node_id) = render::hit_test_chat_message(
                        cache,
                        &branch_ids,
                        chat,
                        app.chat_scroll,
                        mouse.row,
                    ) {
                        app.nav_cursor = Some(node_id);
                    }
                }
                app.auto_scroll = false;
            } else if input.contains(pos) {
                app.focus = Focus::Input;
                app.nav_cursor = None;
                app.auto_scroll = true;
                app.textarea.cancel_selection();
                move_textarea_cursor_to_mouse(&mut app.textarea, input, mouse.column, mouse.row);
            }
            None
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.focus == Focus::Input && input.contains(pos) {
                if app.textarea.selection_range().is_none() {
                    app.textarea.start_selection();
                }
                move_textarea_cursor_to_mouse(&mut app.textarea, input, mouse.column, mouse.row);
            } else if app.focus == Focus::EditDialog {
                if let Some(ref mut editor) = app.edit_editor {
                    if let Ok((tw, th)) = crossterm::terminal::size() {
                        let terminal_area = Rect::new(0, 0, tw, th);
                        let width = (tw as f32 * dialogs::DIALOG_WIDTH_RATIO) as u16;
                        let height = (th as f32 * dialogs::DIALOG_HEIGHT_RATIO) as u16;
                        let dialog = render::centered_rect(width, height, terminal_area);
                        let editor_area = Rect {
                            x: dialog.x + 2,
                            y: dialog.y + 1,
                            width: dialog.width.saturating_sub(4),
                            height: dialog.height.saturating_sub(2),
                        };
                        if editor.selection_range().is_none() {
                            editor.start_selection();
                        }
                        move_textarea_cursor_to_mouse(editor, editor_area, mouse.column, mouse.row);
                    }
                }
            }
            None
        }
        MouseEventKind::ScrollUp => {
            if chat.contains(pos) {
                app.chat_scroll = app.chat_scroll.saturating_sub(3);
                app.auto_scroll = false;
            } else if sidebar.contains(pos) {
                let selected = app.sidebar_state.selected().unwrap_or(0);
                let new = selected.saturating_sub(1);
                app.sidebar_state.select(Some(new));
                input::load_sidebar_selection(app);
            }
            None
        }
        MouseEventKind::ScrollDown => {
            if chat.contains(pos) {
                app.chat_scroll = app.chat_scroll.saturating_add(3).min(app.chat_max_scroll);
                app.auto_scroll = false;
            } else if sidebar.contains(pos) {
                let selected = app.sidebar_state.selected().unwrap_or(0);
                let count = app.sidebar_sessions.len();
                if count > 0 {
                    let new = (selected + 1).min(count - 1);
                    app.sidebar_state.select(Some(new));
                    input::load_sidebar_selection(app);
                }
            }
            None
        }
        MouseEventKind::Moved => {
            let old_hover = app.hover_node;
            if chat.contains(pos) {
                if let Some(ref cache) = app.chat_content_cache {
                    let branch_ids = app.session.tree.current_branch_ids();
                    app.hover_node = render::hit_test_chat_message(
                        cache,
                        &branch_ids,
                        chat,
                        app.chat_scroll,
                        mouse.row,
                    );
                } else {
                    app.hover_node = None;
                }
            } else {
                app.hover_node = None;
            }
            if app.hover_node != old_hover {
                None
            } else {
                None
            }
        }
        _ => None,
    }
}

fn move_textarea_cursor_to_mouse(
    textarea: &mut TextArea,
    widget_area: Rect,
    screen_col: u16,
    screen_row: u16,
) {
    let inner_row = screen_row.saturating_sub(widget_area.y + 1);
    let inner_col = screen_col.saturating_sub(widget_area.x + 1);
    textarea.move_cursor(CursorMove::Jump(inner_row, inner_col));
}

fn is_dialog_focus(focus: Focus) -> bool {
    !matches!(focus, Focus::Input | Focus::Chat | Focus::Sidebar)
}

fn cancel_generation(app: &mut App) {
    if let Some(handle) = app.streaming_task.take() {
        handle.abort();
    }

    if app.is_continuation {
        if !app.streaming_buffer.is_empty() {
            let head = app.session.tree.head().unwrap();
            let existing = app.session.tree.node(head).unwrap().message.content.clone();
            let combined = format!("{}{}", existing, app.streaming_buffer);
            app.session.tree.set_message_content(head, combined);
        }
        app.is_continuation = false;
    } else if !app.streaming_buffer.is_empty() {
        let content = std::mem::take(&mut app.streaming_buffer);
        let head = app.session.tree.head().unwrap();
        app.session
            .tree
            .push(Some(head), Message::new(Role::Assistant, content));
    }

    app.streaming_buffer.clear();
    app.is_streaming = false;
    app.mark_session_dirty(SaveTrigger::StreamDone, true);
    app.invalidate_chat_cache();
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
    app.edit_original_content = content.lines().collect::<Vec<_>>().join("\n");
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
    PresetEditor,
    PersonaEditor,
    CharacterEditor,
    SystemPromptEditor,
    WorldbookEntryEditor,
}

fn handle_field_dialog_key(key: KeyEvent, app: &mut App, kind: DialogKind) -> Option<Action> {
    let dialog = match kind {
        DialogKind::Config => app.config_dialog.as_mut(),
        DialogKind::PresetEditor => app.preset_editor.as_mut(),
        DialogKind::PersonaEditor => app.persona_editor.as_mut(),
        DialogKind::CharacterEditor => app.character_editor.as_mut(),
        DialogKind::SystemPromptEditor => app.system_prompt_editor.as_mut(),
        DialogKind::WorldbookEntryEditor => app.worldbook_entry_editor.as_mut(),
    };

    let Some(dialog) = dialog else {
        return None;
    };

    let result = dialog.handle_key(key);

    if let Some(msg) = dialog.clipboard_warning.take() {
        app.set_status(msg, StatusLevel::Warning);
    }

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
        dialogs::FieldDialogAction::OpenSelector(field_index) => {
            if matches!(kind, DialogKind::Config) {
                match field_index {
                    2 => dialogs::preset::open_preset_picker(
                        app,
                        dialogs::preset::PresetKind::Template,
                    ),
                    3 => dialogs::preset::open_preset_picker(
                        app,
                        dialogs::preset::PresetKind::Instruct,
                    ),
                    4 => dialogs::preset::open_preset_picker(
                        app,
                        dialogs::preset::PresetKind::Reasoning,
                    ),
                    _ => {}
                }
            }
            None
        }
        dialogs::FieldDialogAction::Close => {
            match kind {
                DialogKind::Config => {
                    let dialog = app.config_dialog.as_ref().unwrap();
                    if !dialog.has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.config_dialog = None;
                    } else {
                        let values = &dialog.values;
                        let locked = business::config_locked_fields(&app.cli_overrides);
                        match business::save_config_from_fields(values, &locked) {
                            Ok(()) => {
                                business::apply_config(app);
                                app.set_status(
                                    "Configuration saved.".to_owned(),
                                    StatusLevel::Info,
                                );
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
                }
                DialogKind::PresetEditor => {
                    if !app.preset_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                    } else {
                        let editor = app.preset_editor.as_ref().unwrap();
                        let original_name = app.preset_editor_original_name.clone();
                        let edited_preset_name = editor.values[0].trim().to_owned();
                        match dialogs::preset::save_preset_from_editor(
                            app.preset_editor_kind,
                            &editor.values,
                            &original_name,
                        ) {
                            Ok(()) => {
                                app.set_status("Preset saved.".to_owned(), StatusLevel::Info);
                                dialogs::preset::refresh_preset_list(app);
                                if matches!(
                                    app.preset_editor_kind,
                                    dialogs::preset::PresetKind::Instruct
                                ) && app.instruct_preset.name == original_name
                                {
                                    let resolve_name = if edited_preset_name.is_empty() {
                                        &original_name
                                    } else {
                                        &edited_preset_name
                                    };
                                    app.instruct_preset =
                                        crate::preset::resolve_instruct_preset(resolve_name);
                                    app.stop_tokens = app.instruct_preset.stop_tokens();
                                }
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save preset: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                    }
                    app.preset_editor = None;
                    app.focus = Focus::PresetPickerDialog;
                    return None;
                }
                DialogKind::PersonaEditor => {
                    let is_cli_locked = app.cli_overrides.persona.is_some();
                    if is_cli_locked {
                        app.persona_editor = None;
                        app.focus = Focus::Input;
                    } else if !app.persona_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.persona_editor = None;
                        app.focus = Focus::PersonaDialog;
                    } else {
                        let values = &app.persona_editor.as_ref().unwrap().values;
                        let file_name = app.persona_editor_file_name.clone();
                        let persona = crate::persona::PersonaFile {
                            name: values[0].clone(),
                            persona: values[1].clone(),
                        };

                        if file_name != persona.name
                            && app.persona_list.iter().any(|n| n == &persona.name)
                        {
                            app.set_status(
                                format!("Name '{}' is already in use.", persona.name),
                                StatusLevel::Error,
                            );
                            return None;
                        }

                        let dir = crate::config::personas_dir();
                        if !file_name.is_empty() && file_name != persona.name {
                            if let Some(old_path) =
                                crate::persona::resolve_persona_path(&dir, &file_name)
                            {
                                let _ = std::fs::remove_file(&old_path);
                            }
                        }
                        match crate::persona::save_persona(&persona, &dir, app.save_mode.key()) {
                            Ok(_) => {
                                app.invalidate_chat_cache();
                                if app.session.persona.as_deref() == Some(&file_name)
                                    || app.session.persona.as_deref() == Some(persona.name.as_str())
                                {
                                    app.active_persona_name = Some(persona.name.clone());
                                    app.active_persona_desc = Some(persona.persona.clone());
                                    app.session.persona = Some(persona.name.clone());
                                }
                                app.set_status(
                                    format!("Persona '{}' saved.", persona.name),
                                    StatusLevel::Info,
                                );
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save persona: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                        app.persona_editor = None;
                        maintenance::reload_persona_picker(app);
                        app.focus = Focus::PersonaDialog;
                    }
                    return None;
                }
                DialogKind::SystemPromptEditor => {
                    if app.system_editor_read_only {
                        app.system_prompt_editor = None;
                        app.system_editor_read_only = false;
                        app.focus = app.system_editor_return_focus;
                        return None;
                    }

                    if !app.system_prompt_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.system_prompt_editor = None;
                        app.focus = app.system_editor_return_focus;
                        return None;
                    }

                    let values = &app.system_prompt_editor.as_ref().unwrap().values;
                    let new_name = values[0].clone();
                    let content = values[1].clone();
                    let original_name = app.system_editor_prompt_name.clone();

                    if original_name != new_name
                        && app.system_prompt_list.iter().any(|n| n == &new_name)
                    {
                        app.set_status(
                            format!("Name '{new_name}' is already in use."),
                            StatusLevel::Error,
                        );
                        return None;
                    }

                    let value = if content.trim().is_empty() {
                        None
                    } else {
                        Some(content.clone())
                    };
                    app.session.system_prompt = value;
                    app.invalidate_chat_cache();
                    app.mark_session_dirty(SaveTrigger::Debounced, false);

                    if !original_name.is_empty() {
                        let dir = crate::config::system_prompts_dir();

                        let prompt = crate::system_prompt::SystemPromptFile {
                            name: new_name.clone(),
                            content,
                        };
                        match crate::system_prompt::save_prompt(&prompt, &dir, app.save_mode.key())
                        {
                            Ok(_) => {
                                if original_name != new_name {
                                    let old_path = crate::system_prompt::resolve_prompt_path(
                                        &dir,
                                        &original_name,
                                    );
                                    if old_path.exists() {
                                        let _ = std::fs::remove_file(&old_path);
                                    }
                                }
                                let prompts =
                                    crate::system_prompt::list_prompts(&dir, app.save_mode.key());
                                app.system_prompt_list =
                                    prompts.into_iter().map(|p| p.name).collect();
                                app.set_status(
                                    format!("System prompt '{}' saved.", new_name),
                                    StatusLevel::Info,
                                );
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save prompt: {e}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                    }

                    app.system_prompt_editor = None;
                    app.focus = app.system_editor_return_focus;
                    return None;
                }
                DialogKind::CharacterEditor => {
                    if !app.character_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                        app.character_editor = None;
                        app.focus = Focus::CharacterDialog;
                        return None;
                    }

                    let values = &app.character_editor.as_ref().unwrap().values;
                    let new_slug = crate::character::slugify(&values[0]);
                    if new_slug != app.character_editor_slug
                        && app.character_slugs.iter().any(|s| s == &new_slug)
                    {
                        app.set_status(
                            format!("Name '{}' is already in use.", values[0]),
                            StatusLevel::Error,
                        );
                        return None;
                    }

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
                    let old_path = crate::character::resolve_card_path(
                        &crate::config::characters_dir(),
                        &app.character_editor_slug,
                    );
                    match crate::character::save_card(
                        &card,
                        &crate::config::characters_dir(),
                        app.save_mode.key(),
                    ) {
                        Ok(new_path) => {
                            let mut saved_with_warning = false;
                            if new_path != old_path {
                                if old_path.exists() {
                                    if let Err(err) = std::fs::remove_file(&old_path) {
                                        saved_with_warning = true;
                                        app.set_status(
                                            format!(
                                                "Saved character but failed to remove old file: {err}"
                                            ),
                                            StatusLevel::Warning,
                                        );
                                    } else {
                                        crate::index::warn_if_save_fails(
                                            crate::index::remove_character(
                                                &old_path,
                                                app.save_mode.key(),
                                            ),
                                            "failed to remove character index entry",
                                        );
                                    }
                                } else {
                                    crate::index::warn_if_save_fails(
                                        crate::index::remove_character(
                                            &old_path,
                                            app.save_mode.key(),
                                        ),
                                        "failed to remove character index entry",
                                    );
                                }
                            }

                            let cards = crate::character::list_cards(
                                &crate::config::characters_dir(),
                                app.save_mode.key(),
                            );
                            app.character_names =
                                cards.iter().map(|entry| entry.name.clone()).collect();
                            app.character_slugs =
                                cards.into_iter().map(|entry| entry.slug).collect();
                            let new_slug = new_path
                                .file_stem()
                                .map(|stem| stem.to_string_lossy().to_string())
                                .unwrap_or_default();
                            app.character_selected = app
                                .character_slugs
                                .iter()
                                .position(|existing| existing == &new_slug)
                                .unwrap_or(0)
                                .min(app.character_slugs.len().saturating_sub(1));
                            app.character_editor_slug = new_slug;
                            if !saved_with_warning {
                                app.set_status(
                                    format!("Saved character: {}", card.name),
                                    StatusLevel::Info,
                                );
                            }
                            let is_active =
                                app.session.character.as_deref().is_some_and(|name| {
                                    crate::character::slugify(name)
                                        == app.character_editor_slug
                                });
                            if is_active {
                                let cfg = crate::config::load();
                                let tpl_name =
                                    cfg.template_preset.as_deref().unwrap_or("Default");
                                let tpl =
                                    crate::preset::resolve_template_preset(tpl_name);
                                app.session.system_prompt = Some(
                                    crate::character::build_system_prompt(
                                        &card,
                                        Some(&tpl),
                                    ),
                                );
                                app.session.character = Some(card.name.clone());
                                app.invalidate_chat_cache();
                            }
                        }
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
                    if !app.worldbook_entry_editor.as_ref().unwrap().has_changes() {
                        app.set_status("No changes found.".to_owned(), StatusLevel::Info);
                    } else {
                        let values = &app.worldbook_entry_editor.as_ref().unwrap().values;
                        let idx = app.worldbook_entry_editor_index;
                        if idx < app.worldbook_editor_entries.len() {
                            app.worldbook_editor_entries[idx] = dialogs::worldbook::values_to_entry(
                                values,
                                &app.worldbook_editor_entries[idx],
                            );
                        }
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
