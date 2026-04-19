//! Terminal UI application: event loop, layout, and state management.

pub mod business;
mod clipboard;
pub mod commands;
mod dialog_handler;
pub mod dialogs;
mod events;
mod input;
mod maintenance;
mod render;
mod state;
pub mod theme;
mod types;

use types::*;

use anyhow::Result;
use crossterm::event::{Event, EventStream, MouseEventKind};

use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::cli::CliOverrides;
use libllm::client::{ApiClient, StreamToken};
use libllm::context::ContextManager;
use libllm::preset::InstructPreset;
use libllm::sampling::SamplingParams;
use libllm::session::{SaveMode, Session};

pub fn build_effective_system_prompt_standalone(
    session: &Session,
    db: Option<&libllm::db::Database>,
) -> Option<String> {
    business::build_effective_system_prompt(session, db)
}

pub async fn run(
    client: ApiClient,
    session: &mut Session,
    save_mode: SaveMode,
    db: Option<libllm::db::Database>,
    instruct_preset: InstructPreset,
    sampling: SamplingParams,
    cli_overrides: CliOverrides,
) -> Result<Option<String>> {
    let sidebar_sessions = {
        let _span = tracing::info_span!("startup.phase", phase = "sidebar_discovery").entered();
        business::discover_sidebar_sessions(&save_mode, db.as_ref())
    };

    let mut textarea = TextArea::default();
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input ")
            .title_bottom(Line::from(" Enter to send, Alt+Enter for newline ").centered()),
    );
    dialog_handler::configure_textarea(&mut textarea);

    let sidebar_state = ratatui::widgets::ListState::default();

    let (token_tx, mut token_rx) = mpsc::channel::<StreamToken>(256);
    let (bg_tx, mut bg_rx) = mpsc::channel::<BackgroundEvent>(64);

    {
        let client = client.clone();
        let tx = bg_tx.clone();
        tokio::spawn(async move {
            let result = client.fetch_model_name().await;
            let _ = tx.send(BackgroundEvent::ModelFetched(result)).await;
            if let Some(server_ctx) = client.fetch_server_context_size().await {
                let _ = tx
                    .send(BackgroundEvent::ServerContextSize(server_ctx))
                    .await;
            }
        });
    }

    let (tokenizer_tx, mut tokenizer_rx) = mpsc::channel::<libllm::tokenizer::TokenCountUpdate>(64);
    let token_counter = libllm::tokenizer::TokenCounter::new(client.clone(), tokenizer_tx).await;

    let config = libllm::config::load();

    let salt_exists = libllm::config::salt_path().exists();
    let initial_passkey_setup = save_mode.needs_passkey() && !salt_exists;

    let mut app = App {
        client,
        session,
        db,
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
        context_mgr: ContextManager::new(config.summarization.context_size),
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
        sidebar_state,
        streaming_buffer: String::new(),
        is_streaming: false,
        is_continuation: false,
        message_queue: Vec::new(),
        streaming_task: None,
        is_summarizing: false,
        summary_receiver: None,
        summary_branch_head: None,
        summary_pending_dropped: None,
        summarization_enabled: config.summarization.enabled && !cli_overrides.no_summarize,
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
        resolved_passkey: None,
        set_passkey_input: String::new(),
        set_passkey_confirm: String::new(),
        set_passkey_active_field: 0,
        set_passkey_error: String::new(),
        set_passkey_deriving: false,
        set_passkey_is_initial: initial_passkey_setup,
        config_dialog: None,
        auth_dialog: None,
        theme_dialog: None,
        base_theme_picker_names: Vec::new(),
        base_theme_picker_selected: 0,
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
        token_counter,
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
        persona_slugs: Vec::new(),
        persona_names: Vec::new(),
        persona_selected: 0,
        persona_editor_slug: String::new(),
        theme: theme::resolve_theme(&config),
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
        input_reject_flash: None,
        dialog_search: dialogs::SearchState::new(),
        sidebar_search: dialogs::SearchState::new(),
        last_terminal_height: 0,
    };

    if app.token_counter.is_heuristic() {
        app.set_status(
            "token counts approximate — llama.cpp /tokenize unavailable".to_owned(),
            StatusLevel::Warning,
        );
    }

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

    let mut frame_tick = tokio::time::interval(STREAM_REDRAW_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut needs_redraw = false;

    libllm::timed_result!(tracing::Level::INFO, "startup.phase", phase = "first_draw" ; {
        terminal.draw(|f| render_frame(f, &mut app))
    })?;
    {
        let _span = tracing::info_span!("startup.phase", phase = "maintenance_schedule").entered();
        maintenance::spawn_startup_maintenance(&app.save_mode, &app)
    };

    loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                let is_mouse_move = matches!(&event, Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved));
                {
                    let _span = tracing::trace_span!("event", phase = "handle").entered();
                    if let Some(action) = events::handle_event(event, &mut app, bg_tx.clone()) {
                        events::process_action(action, &mut app, token_tx.clone()).await;
                    }
                }
                if is_mouse_move {
                    needs_redraw = true;
                } else {
                    terminal.draw(|f| render_frame(f, &mut app))?;
                    needs_redraw = false;
                }
            }
            Some(stream_token) = token_rx.recv() => {
                use tracing::Instrument;
                commands::handle_stream_token(stream_token, &mut app, token_tx.clone())
                    .instrument(tracing::trace_span!("stream", phase = "token"))
                    .await?;
                needs_redraw = true;
            }
            Some(bg_event) = bg_rx.recv() => {
                commands::handle_background_event(bg_event, &mut app);
                terminal.draw(|f| render_frame(f, &mut app))?;
                needs_redraw = false;
            }
            Some(update) = tokenizer_rx.recv() => {
                commands::handle_background_event(
                    BackgroundEvent::TokenCountReady(update),
                    &mut app,
                );
                terminal.draw(|f| render_frame(f, &mut app))?;
                needs_redraw = false;
            }
            _ = frame_tick.tick() => {
                if app.summary_receiver.is_some() {
                    let completed = app.summary_receiver.as_mut().unwrap().try_recv();
                    if let Ok(result) = completed {
                        let current_head = app.session.tree.head();
                        let expected_head = app.summary_branch_head;
                        app.summary_receiver = None;
                        app.summary_branch_head = None;

                        if current_head == expected_head
                            && let Ok(summary_text) = result {
                                let dropped = app.summary_pending_dropped.take().unwrap_or(0);

                                if dropped > 0 {
                                    let branch_path = app.session.tree.branch_path();
                                    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
                                    let branch_ids = app.session.tree.branch_path_ids();
                                    let summary_boundary = branch_ids.len() - summary_aware.len();
                                    let insert_idx = summary_boundary + dropped - 1;
                                    if insert_idx < branch_ids.len() {
                                        let parent_node_id = branch_ids[insert_idx];
                                        app.session.tree.push(
                                            Some(parent_node_id),
                                            libllm::session::Message::new(
                                                libllm::session::Role::Summary,
                                                summary_text,
                                            ),
                                        );
                                        app.mark_session_dirty(SaveTrigger::StreamDone, true);
                                        app.invalidate_chat_cache();
                                    }
                                }
                            }

                        if app.is_summarizing {
                            app.is_summarizing = false;
                            if !app.message_queue.is_empty() {
                                let next = app.message_queue.remove(0);
                                commands::start_streaming(&mut app, &next, token_tx.clone()).await;
                                if !app.is_streaming {
                                    app.message_queue.clear();
                                }
                            }
                        }
                        needs_redraw = true;
                    }
                }
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

    Ok(app.resolved_passkey.clone())
}

fn render_frame(f: &mut ratatui::Frame, app: &mut App) {
    app.last_terminal_height = f.area().height;
    let _frame_start = std::time::Instant::now();

    let (outer, columns, right_split) = {
        let _span = tracing::trace_span!("layout", phase = "splits").entered();
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
    };

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
    {
        let _span = tracing::trace_span!("sidebar", session_count).entered();
        render::render_sidebar(f, app, sidebar_area);
    }

    let input_focused = app.focus == Focus::Input;
    let input_token_count = estimate_input_tokens(app);
    let border = render::border_style(input_focused, &app.theme);
    let mut input_block = Block::default()
        .borders(Borders::ALL)
        .title(" Input ")
        .title(
            Line::from(format!(" Est. {} ", format_token_count(input_token_count))).right_aligned(),
        )
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

    let max_scroll;
    let mut cache = app.chat_content_cache.take();
    {
        let branch_ids = app.session.tree.current_branch_ids();
        let state = match app.cached_token_count {
            Some(state) => state,
            None => {
                let text = commands::streaming::build_rendered_prompt(app, 0);
                let state = app.token_counter.count_cached(&text);
                app.cached_token_count = Some(state);
                state
            }
        };
        let msg_count = branch_ids.len();
        tracing::trace!(node_count = msg_count, "chat.branch");
        {
            let _span =
                tracing::trace_span!("chat", message_count = msg_count, scroll_dirty).entered();
            max_scroll = render::render_chat(
                f,
                app,
                messages_area,
                branch_ids,
                render::TokenDisplayParams {
                    token_state: state,
                    is_heuristic: app.token_counter.is_heuristic(),
                    budget: app.context_mgr.token_limit(),
                },
                render::ChatRenderState {
                    chat_scroll: &mut chat_scroll,
                    scroll_dirty,
                    cache: &mut cache,
                },
            );
            if let Some(queue_rect) = queue_area {
                render::render_message_queue(f, app, queue_rect);
            }
        }

        {
            let _span = tracing::trace_span!("status", phase = "bar").entered();
            render::render_status_bar(f, app, status_area);
        }
    }
    app.chat_content_cache = cache;
    app.chat_scroll = chat_scroll;
    app.chat_max_scroll = max_scroll;
    app.last_scroll_state = current_scroll_state;

    if app.focus == Focus::Input && input::input_has_command_picker(app) {
        {
            let _span = tracing::trace_span!("picker", phase = "command_picker").entered();
            render::render_command_picker(f, app, &app.textarea.lines()[0], chat_area);
        }
    }

    let dialog_name = match app.focus {
        Focus::PasskeyDialog => Some("passkey"),
        Focus::SetPasskeyDialog => Some("set_passkey"),
        Focus::ConfigDialog => Some("config"),
        Focus::ThemeDialog => Some("theme"),
        Focus::BaseThemePickerDialog => Some("base_theme_picker"),
        Focus::PresetPickerDialog => Some("preset_picker"),
        Focus::AuthDialog => Some("auth_dialog"),
        Focus::AuthTypePicker => Some("auth_type_picker"),
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
        let _span = tracing::trace_span!("dialog", name).entered();
        render_dialog(f, app);
    }

    let frame_ms = _frame_start.elapsed().as_micros() as f64 / 1000.0;
    tracing::trace!(
        phase = "frame",
        elapsed_ms = format!("{frame_ms:.3}"),
        "frame"
    );
}

fn estimate_input_tokens(app: &App) -> usize {
    let input = app.textarea.lines().join("\n");
    estimate_input_tokens_from_text(&input, &app.token_counter)
}

fn estimate_input_tokens_from_text(
    input: &str,
    token_counter: &libllm::tokenizer::TokenCounter,
) -> usize {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return 0;
    }

    token_counter.heuristic_count(trimmed)
}

fn format_token_count(count: usize) -> String {
    if count == 1 {
        "1 token".to_owned()
    } else {
        format!("{count} tokens")
    }
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
                dialog.render(f, f.area(), &app.theme);
            }
        }
        Focus::ThemeDialog => {
            if let Some(ref dialog) = app.theme_dialog {
                dialog.render(f, f.area(), &app.theme);
            }
        }
        Focus::BaseThemePickerDialog => {
            render_base_theme_picker(f, app, f.area());
        }
        Focus::PresetPickerDialog => {
            dialogs::preset::render_preset_dialog(f, app, f.area());
        }
        Focus::AuthDialog => {
            dialogs::auth::render_auth_dialog(f, app, f.area());
        }
        Focus::AuthTypePicker => {
            dialogs::auth::render_type_picker(f, app, f.area());
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

fn render_base_theme_picker(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let names = &app.base_theme_picker_names;
    let count = names.len();
    let dialog = render::clear_centered(
        f,
        dialogs::LIST_DIALOG_WIDTH,
        count as u16 + dialogs::LIST_DIALOG_TALL_PADDING,
        area,
    );

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, name) in names.iter().enumerate() {
        let is_selected = i == app.base_theme_picker_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{name}"), style)));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(render::dialog_block(" Select Base Theme ", Color::Yellow));
    f.render_widget(paragraph, dialog);
    render::render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from("Up/Down: navigate  Enter: select  Esc: cancel")],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heuristic_counter() -> libllm::tokenizer::TokenCounter {
        let (tx, _rx) = mpsc::channel(8);
        libllm::tokenizer::TokenCounter::new_with_backend(
            libllm::tokenizer::TokenizerBackend::Heuristic(
                libllm::tokenizer::HeuristicTokenizer::standard(),
            ),
            tx,
        )
    }

    #[test]
    fn estimate_input_tokens_from_text_returns_zero_for_blank_input() {
        let counter = heuristic_counter();
        assert_eq!(estimate_input_tokens_from_text("   \n\t  ", &counter), 0);
    }

    #[test]
    fn estimate_input_tokens_from_text_trims_outer_whitespace() {
        let counter = heuristic_counter();
        assert_eq!(estimate_input_tokens_from_text("  abcd  ", &counter), 5);
    }

    #[test]
    fn estimate_input_tokens_from_text_counts_multiline_content() {
        let counter = heuristic_counter();
        assert_eq!(estimate_input_tokens_from_text("abcd\nefgh", &counter), 6);
    }

    #[test]
    fn format_token_count_uses_singular_and_plural() {
        assert_eq!(format_token_count(1), "1 token");
        assert_eq!(format_token_count(2), "2 tokens");
    }
}
