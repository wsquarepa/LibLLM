//! Shared type definitions for TUI application state.

use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::cli::CliOverrides;
use libllm::client::ApiClient;
use libllm::context::ContextManager;
use libllm::preset::{InstructPreset, ReasoningPreset};
use libllm::sampling::SamplingParams;
use libllm::session::{NodeId, SaveMode, Session, SessionEntry};
use libllm::worldinfo::RuntimeWorldBook;

use super::dialogs;
use super::render;
use super::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Focus {
    Input,
    Chat,
    Sidebar,
    PasskeyDialog,
    SetPasskeyDialog,
    ConfigDialog,
    ThemeDialog,
    BaseThemePickerDialog,
    PresetPickerDialog,
    PresetEditorDialog,
    AuthDialog,
    AuthTypePicker,
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
    FilePickerDialog,
    InjectionWarningDialog,
    LoadingDialog,
}

pub(super) enum Action {
    SendMessage(String),
    EditMessage {
        node_id: libllm::session::NodeId,
        content: String,
    },
    SlashCommand(String, String),
    Quit,
}

pub(super) enum DeleteContext {
    Session,
    Character { slug: String },
    Persona { slug: String },
    SystemPrompt { name: String },
    Worldbook { name: String },
    Preset { kind: dialogs::preset::PresetKind },
    ThemeResetColors,
    ChatMessage { node_id: NodeId },
}

#[derive(Clone, Copy)]
pub(super) enum StatusLevel {
    Info,
    Warning,
    Error,
}

pub(super) struct StatusMessage {
    pub(super) text: String,
    pub(super) level: StatusLevel,
    pub(super) created: std::time::Instant,
    pub(super) expires: std::time::Instant,
}

pub(super) struct WorldbookCache {
    pub(super) enabled_names: Vec<String>,
    pub(super) books: Vec<RuntimeWorldBook>,
}

#[derive(Clone, Copy)]
pub(super) enum SaveTrigger {
    Debounced,
    StreamDone,
    Exit,
    Transition,
    Retry,
}

impl SaveTrigger {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Debounced => "debounced",
            Self::StreamDone => "stream_done",
            Self::Exit => "exit",
            Self::Transition => "transition",
            Self::Retry => "retry",
        }
    }
}

pub(super) struct AutosaveDebugState {
    pub(super) dirty_since: Option<std::time::Instant>,
    pub(super) save_count: u64,
    pub(super) retry_count: u64,
}

pub(super) struct UnlockDebugState {
    pub(super) kind: &'static str,
    pub(super) started_at: std::time::Instant,
}

pub(super) enum BackgroundEvent {
    KeyDerived(
        std::sync::Arc<libllm::crypto::DerivedKey>,
        std::path::PathBuf,
    ),
    KeyDeriveFailed(String),
    PasskeySet(std::sync::Arc<libllm::crypto::DerivedKey>),
    PasskeySetFailed(String),
    ModelFetched(std::result::Result<String, String>),
    ServerContextSize(usize),
    TokenizerReloaded(libllm::tokenizer::TokenCounter),
    TokenCountReady(libllm::tokenizer::TokenCountUpdate),
}

#[derive(PartialEq, Eq)]
pub(super) struct ScrollState {
    pub(super) auto_scroll: bool,
    pub(super) nav_cursor: Option<NodeId>,
    pub(super) head: Option<NodeId>,
    /// Current branch length. Tracked so that mutations which grow the branch while
    /// preserving head (e.g. splicing an auto-summary between existing nodes) still
    /// dirty the scroll state and let `auto_scroll` re-snap to the new bottom.
    pub(super) branch_len: usize,
    pub(super) buffer_len: usize,
    /// Tracks whether the first streamed think block has closed. The transition
    /// collapses the rendered chat height without changing `buffer_len`, so
    /// without this the auto-scroll re-snap misses the collapse frame.
    pub(super) first_think_closed: bool,
    pub(super) width: u16,
    pub(super) height: u16,
    /// Monotonic counter bumped whenever a file-summary completion lands. File
    /// summaries grow the rendered chat height without changing `head` or
    /// `branch_len`, so without this the auto-scroll re-snap never fires.
    pub(super) summary_revision: u64,
}

pub(super) const SIDEBAR_WIDTH: u16 = 32;
pub(super) const INPUT_HEIGHT: u16 = 5;

pub(super) struct LayoutAreas {
    pub(super) sidebar: Rect,
    pub(super) chat: Rect,
    pub(super) input: Rect,
}

pub(super) struct App<'a> {
    pub(super) client: ApiClient,
    pub(super) session: &'a mut Session,
    pub(super) save_mode: SaveMode,
    pub(super) db: Option<libllm::db::Database>,
    pub(super) session_dirty: bool,
    pub(super) pending_save_deadline: Option<std::time::Instant>,
    pub(super) pending_save_trigger: Option<SaveTrigger>,
    pub(super) instruct_preset: InstructPreset,
    pub(super) reasoning_preset: Option<ReasoningPreset>,
    pub(super) stop_tokens: Vec<String>,
    pub(super) sampling: SamplingParams,
    pub(super) context_mgr: ContextManager,

    pub(super) focus: Focus,
    pub(super) textarea: TextArea<'a>,
    pub(super) chat_scroll: u16,
    pub(super) chat_max_scroll: u16,
    pub(super) auto_scroll: bool,
    pub(super) last_scroll_state: ScrollState,
    pub(super) sidebar_sessions: Vec<SessionEntry>,
    pub(super) sidebar_state: ratatui::widgets::ListState,
    pub(super) streaming_buffer: String,
    pub(super) is_streaming: bool,
    pub(super) is_continuation: bool,
    pub(super) stream_started_at: Option<std::time::Instant>,
    pub(super) stream_first_think_closed_at: Option<std::time::Instant>,
    pub(super) message_queue: Vec<String>,
    pub(super) streaming_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) is_summarizing: bool,
    pub(super) summary_receiver: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    pub(super) summary_branch_head: Option<NodeId>,
    pub(super) summary_pending_dropped: Option<usize>,
    pub(super) summarization_enabled: bool,
    pub(super) model_name: Option<String>,
    pub(super) api_available: bool,
    pub(super) api_error: String,
    pub(super) file_picker: Option<dialogs::file_picker::FilePickerState>,
    pub(super) injection_warning: Option<dialogs::injection_warning::InjectionWarning>,
    pub(super) status_message: Option<StatusMessage>,
    pub(super) should_quit: bool,
    pub(super) passkey_changed: bool,
    pub(super) command_picker_selected: usize,

    pub(super) passkey_input: String,
    pub(super) passkey_error: String,
    pub(super) passkey_deriving: bool,
    pub(super) resolved_passkey: Option<String>,
    pub(super) pending_new_passkey: Option<String>,

    pub(super) set_passkey_input: String,
    pub(super) set_passkey_confirm: String,
    pub(super) set_passkey_active_field: u8,
    pub(super) set_passkey_error: String,
    pub(super) set_passkey_deriving: bool,
    pub(super) set_passkey_is_initial: bool,

    pub(super) config_dialog: Option<dialogs::TabbedFieldDialog<'a>>,
    pub(super) auth_dialog: Option<dialogs::auth::AuthDialogState>,
    pub(super) theme_dialog: Option<dialogs::TabbedFieldDialog<'a>>,
    pub(super) base_theme_picker_names: Vec<String>,
    pub(super) base_theme_picker_selected: usize,
    pub(super) persona_editor: Option<dialogs::FieldDialog<'a>>,
    pub(super) system_prompt_editor: Option<dialogs::FieldDialog<'a>>,
    pub(super) system_editor_prompt_name: String,
    pub(super) system_editor_return_focus: Focus,
    pub(super) system_editor_read_only: bool,

    pub(super) system_prompt_list: Vec<String>,
    pub(super) system_prompt_selected: usize,
    pub(super) edit_editor: Option<TextArea<'a>>,

    pub(super) preset_picker_kind: dialogs::preset::PresetKind,
    pub(super) preset_picker_names: Vec<String>,
    pub(super) preset_picker_selected: usize,
    pub(super) preset_editor: Option<dialogs::FieldDialog<'a>>,
    pub(super) preset_editor_kind: dialogs::preset::PresetKind,
    pub(super) preset_editor_original_name: String,

    pub(super) character_names: Vec<String>,
    pub(super) character_slugs: Vec<String>,
    pub(super) character_selected: usize,

    pub(super) worldbook_list: Vec<String>,
    pub(super) worldbook_selected: usize,

    pub(super) character_editor: Option<dialogs::FieldDialog<'a>>,
    pub(super) character_editor_slug: String,
    pub(super) worldbook_editor_entries: Vec<libllm::worldinfo::Entry>,
    pub(super) worldbook_editor_original_entries: Vec<libllm::worldinfo::Entry>,
    pub(super) worldbook_editor_name: String,
    pub(super) worldbook_editor_original_name: String,
    pub(super) worldbook_editor_name_selected: bool,
    pub(super) worldbook_editor_name_editing: bool,
    pub(super) worldbook_editor_selected: usize,
    pub(super) worldbook_entry_editor: Option<dialogs::FieldDialog<'a>>,
    pub(super) worldbook_entry_editor_index: usize,

    pub(super) chat_content_cache: Option<render::ChatContentCache>,
    pub(super) cached_token_count: Option<libllm::tokenizer::CountState>,
    pub(super) token_counter: libllm::tokenizer::TokenCounter,
    pub(super) tokenizer_tx: mpsc::Sender<libllm::tokenizer::TokenCountUpdate>,
    pub(super) sidebar_cache: Option<render::SidebarCache>,
    pub(super) sidebar_age_refresh_at: std::time::Instant,
    pub(super) raw_edit_node: Option<NodeId>,
    pub(super) edit_original_content: String,
    pub(super) edit_confirm_selected: usize,
    pub(super) nav_cursor: Option<NodeId>,
    pub(super) branch_dialog_items: Vec<(NodeId, String)>,
    pub(super) branch_dialog_selected: usize,
    pub(super) delete_confirm_selected: usize,
    pub(super) delete_confirm_filename: String,
    pub(super) delete_context: DeleteContext,
    pub(super) active_persona_name: Option<String>,
    pub(super) active_persona_desc: Option<String>,
    pub(super) persona_slugs: Vec<String>,
    pub(super) persona_names: Vec<String>,
    pub(super) persona_selected: usize,
    pub(super) persona_editor_slug: String,
    pub(super) config: libllm::config::Config,
    pub(super) theme: theme::Theme,
    pub(super) cli_overrides: CliOverrides,
    pub(super) worldbook_cache: Option<WorldbookCache>,
    pub(super) bg_tx: mpsc::Sender<BackgroundEvent>,
    pub(super) layout_areas: Option<LayoutAreas>,
    pub(super) hover_node: Option<NodeId>,
    pub(super) autosave_debug: AutosaveDebugState,
    pub(super) unlock_debug: Option<UnlockDebugState>,
    pub(super) input_reject_flash: Option<std::time::Instant>,
    pub(super) dialog_search: dialogs::SearchState,
    pub(super) sidebar_search: dialogs::SearchState,
    pub(super) last_terminal_height: u16,
    pub(super) input_file_cache: crate::tui::input_file_cache::InputFileCache,
    /// When `Some`, the next `SendMessage` is the resend of a recalled user
    /// message. The stored value is the `@file` refs from the recalled
    /// content; the send path compares these to the outgoing content to
    /// decide whether to reuse the existing file-snapshot parent chain or
    /// push a fresh one.
    pub(super) recall_refs: Option<Vec<String>>,
    pub(super) file_summarizer:
        Option<std::sync::Arc<libllm::files::FileSummarizer>>,
    pub(super) file_summary_ready_tx:
        tokio::sync::mpsc::UnboundedSender<libllm::files::ReadyEvent>,
    pub(super) file_summary_ready_rx:
        tokio::sync::mpsc::UnboundedReceiver<libllm::files::ReadyEvent>,
    /// Monotonic counter bumped on each file-summary completion so that the
    /// next render sees `scroll_dirty` and re-snaps to the new bottom.
    pub(super) file_summary_revision: u64,
}

impl<'a> App<'a> {
    pub(super) fn open_paged_dialog(&mut self, focus: Focus) {
        self.dialog_search = dialogs::SearchState::new();
        self.focus = focus;
    }
}

pub(super) const STATUS_DURATION: std::time::Duration = std::time::Duration::from_secs(5);
pub(super) const NOTIFICATION_SLIDE_DURATION: std::time::Duration =
    std::time::Duration::from_millis(300);
pub(super) const STREAM_REDRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);
pub(super) const SIDEBAR_AGE_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
pub(super) const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(350);
pub(super) const AUTOSAVE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
