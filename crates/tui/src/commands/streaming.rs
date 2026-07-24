//! Streaming completion request lifecycle: start, token handling, and worldbook loading.

use std::collections::HashMap;

use anyhow::Result;
use tokio::sync::mpsc;

use libllm_core::group_chat::ChatMode;
use libllm_core::preset::InstructPreset;
use libllm_core::session::{Message, Role};
use libllm_protocol::client::StreamToken;

use crate::business;
use crate::types::{SaveTrigger, StatusLevel, WorldbookCache};

use super::App;

struct SnapshotFileSummaryLookup(HashMap<String, libllm_core::files::FileSummary>);

impl libllm_core::files::FileSummaryLookup for SnapshotFileSummaryLookup {
    fn lookup(&self, content_hash: &str) -> Option<libllm_core::files::FileSummary> {
        self.0.get(content_hash).cloned()
    }
}

pub(crate) fn loaded_worldbooks(app: &mut App) -> Vec<libllm_core::worldinfo::RuntimeWorldBook> {
    let enabled_names = business::enabled_worldbook_names(app.session, &app.config);
    let cache_stale = app
        .worldbook_cache
        .as_ref()
        .is_none_or(|cache| cache.enabled_names != enabled_names);

    if cache_stale {
        let books = {
            let _span = tracing::debug_span!(
                "worldbook.runtime",
                phase = "load",
                cache = "miss",
                enabled_count = enabled_names.len()
            )
            .entered();
            business::load_runtime_worldbooks(&enabled_names, app.db.as_ref())
        };
        app.worldbook_cache = Some(WorldbookCache {
            enabled_names,
            books,
        });
    } else if let Some(cache) = app.worldbook_cache.as_ref() {
        tracing::debug!(
            phase = "load",
            cache = "hit",
            enabled_count = enabled_names.len(),
            book_count = cache.books.len(),
            "worldbook.runtime"
        );
    }

    app.worldbook_cache.as_ref().expect("worldbook_cache is set to Some in the cache_stale branch just above, or was already Some").books.clone()
}

fn build_rendered_prompt_common<F>(app: &crate::App, dropped: usize, render: F) -> (String, usize)
where
    F: FnOnce(&InstructPreset, &[&libllm_core::session::Message], Option<&str>) -> String,
{
    let worldbooks = cached_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let context_messages = app.context_mgr.summary_aware_path(&branch_path);
    let trimmed = libllm_core::context::drop_oldest_non_summary(&context_messages, dropped);
    let effective_prompt = business::build_effective_system_prompt(app.session, app.db.as_ref());
    let user_name = app.persona.active_name.as_deref().unwrap_or("User");
    let injected =
        business::inject_loaded_worldbook_entries(app.session, &trimmed, user_name, &worldbooks);
    let mut injected = business::replace_template_vars(app.session, injected, user_name, |slug| {
        app.character.cards_cache.get(slug).map(|c| c.name.clone())
    });

    // Choose which character card's author_note to inject. For solo sessions, use
    // `session.character`. For group sessions, use the active speaker — the speaker
    // field on the head assistant message (which `run_one_group_turn` and
    // `start_group_continuation` set before this code runs).
    let speaker_for_note: Option<String> = if app.session.characters.len() >= 2 {
        app.session
            .tree
            .head()
            .and_then(|id| app.session.tree.node(id))
            .and_then(|n| n.message.speaker.clone())
    } else {
        app.session.character.clone()
    };
    let card_note = speaker_for_note
        .as_deref()
        .and_then(|name| {
            let db = app.db.as_ref()?;
            let slug = libllm_core::character::slugify(name);
            match db.load_character(&slug) {
                Ok(card) => Some(card),
                Err(err) => {
                    tracing::warn!(
                        name = name,
                        slug = slug.as_str(),
                        result = "error",
                        error = %err,
                        "author_note.card_load"
                    );
                    None
                }
            }
        })
        .and_then(|card| card.author_note);

    libllm_core::author_note::inject_author_notes(
        &mut injected,
        card_note.as_ref(),
        app.session.author_note.as_ref(),
    );

    let injected: Vec<libllm_core::session::Message> = injected
        .into_iter()
        .map(|m| {
            // File-snapshot system messages have their delimiter structure validated
            // at attach time. Applying PromptSend rules to them can transform escaped
            // content into exact delimiter lines, bypassing that validation.
            // Only System-role snapshots skip PromptSend; user/assistant content that
            // happens to match the snapshot shape still runs through the rules.
            if m.role == libllm_core::session::Role::System
                && libllm_core::files::is_snapshot(&m.content)
            {
                return m;
            }
            let new_content = libllm_core::regex_rules::apply(
                &app.compiled_regex,
                libllm_core::regex_rules::Scope::PromptSend,
                m.role,
                &m.content,
            )
            .into_owned();
            libllm_core::session::Message {
                content: new_content,
                ..m
            }
        })
        .map(|m| match m.role {
            libllm_core::session::Role::User => libllm_core::session::Message {
                role: m.role,
                content: libllm_core::files::rewrite_user_message(&m.content),
                timestamp: m.timestamp.clone(),
                thought_seconds: m.thought_seconds,
                speaker: m.speaker.clone(),
                pre_turn_action_points: m.pre_turn_action_points.clone(),
            },
            _ => m,
        })
        .collect();
    let message_count = injected.len();
    let injected_refs: Vec<&libllm_core::session::Message> = injected.iter().collect();
    let rendered = render(
        &app.instruct_preset,
        &injected_refs,
        effective_prompt.as_deref(),
    );
    (rendered, message_count)
}

/// Builds the final prompt string for streaming, with the `dropped` oldest non-summary
/// messages removed. This is the exact byte stream that would be POSTed to `/completion`.
/// Returns the rendered prompt and the number of messages that composed it (after
/// summary-aware trim, drop, worldbook injection, and template rewrite). Callers that
/// only need the string can `.0` the tuple.
pub(crate) fn build_rendered_prompt(app: &crate::App, dropped: usize) -> (String, usize) {
    let (prompt, message_count) =
        build_rendered_prompt_common(app, dropped, |preset, refs, sys| preset.render(refs, sys));
    let final_prompt = match app.reasoning_preset.as_ref() {
        Some(preset) => preset.apply_prefix(&prompt),
        None => prompt,
    };
    (final_prompt, message_count)
}

/// Same as `build_rendered_prompt` but uses `InstructPreset::render_continuation` instead
/// of `render`. Used by the `/continue` command path in `commands/mod.rs`.
pub(crate) fn build_rendered_prompt_continuation(
    app: &crate::App,
    dropped: usize,
) -> (String, usize) {
    build_rendered_prompt_common(app, dropped, |preset, refs, sys| {
        preset.render_continuation(refs, sys)
    })
}

/// Same as `build_rendered_prompt` but overrides the system prompt with `system` and
/// optionally injects a short `nudge` system message immediately before the assistant
/// prefill. Used by the group-chat path where `build_turn_prompt` supplies the
/// speaker-specific system prompt and group nudge.
///
/// Renders in continuation mode: the last assistant message holds the speaker-name prefill
/// (e.g. `Alice: `), and the model is meant to extend it. Using `render` here would close
/// the assistant turn with `output_suffix` (e.g. `<|im_end|>`), which forces the model to
/// open a fresh turn after a system instruction it just received — the path that produces
/// "Understood. I will follow..." preambles.
fn build_rendered_prompt_with_system(
    app: &crate::App,
    dropped: usize,
    system: &str,
    nudge: Option<&str>,
) -> (String, usize) {
    let system = system.to_owned();
    let nudge = nudge.map(str::to_owned);
    build_rendered_prompt_common_with_nudge(app, dropped, nudge, move |preset, refs, _| {
        preset.render_continuation(refs, Some(&system))
    })
}

/// Variant of `build_rendered_prompt_common` that inserts a `Role::System` message
/// containing `nudge` immediately before the last branch message (the assistant
/// prefill, in the group-chat path) before passing the message list to `render`.
/// Mirrors SillyTavern's `groupNudge` injection (`openai.js:1361-1375`,
/// `group-chats.js:114`): the nudge sits between the user's last turn and the
/// assistant's turn opener, naming the active speaker.
fn build_rendered_prompt_common_with_nudge<F>(
    app: &crate::App,
    dropped: usize,
    nudge: Option<String>,
    render: F,
) -> (String, usize)
where
    F: FnOnce(&InstructPreset, &[&libllm_core::session::Message], Option<&str>) -> String,
{
    build_rendered_prompt_common(app, dropped, |preset, refs, sys| {
        let Some(nudge_text) = nudge.as_deref() else {
            return render(preset, refs, sys);
        };
        let nudge_msg = libllm_core::session::Message::new(
            libllm_core::session::Role::System,
            nudge_text.to_owned(),
        );
        let mut with_nudge: Vec<&libllm_core::session::Message> = refs.to_vec();
        if with_nudge.is_empty() {
            with_nudge.push(&nudge_msg);
        } else {
            let last_idx = with_nudge.len() - 1;
            with_nudge.insert(last_idx, &nudge_msg);
        }
        render(preset, &with_nudge, sys)
    })
}

/// Read-only view of the worldbook cache for `build_rendered_prompt*`. The cache is
/// always populated by a prior `loaded_worldbooks` call in the same request path; a miss
/// yields an empty slice, which is a correct (if degraded) rendering.
fn cached_worldbooks(app: &crate::App) -> Vec<libllm_core::worldinfo::RuntimeWorldBook> {
    app.worldbook_cache
        .as_ref()
        .map(|cache| cache.books.clone())
        .unwrap_or_default()
}

/// Binary-searches the smallest `k ∈ [0, max_drop]` such that
/// `counter.count_authoritative(&render(k)).await? ≤ budget`. Returns `max_drop` if no
/// value satisfies the budget (defensive fallback; the caller logs this as a warning).
pub(crate) async fn find_smallest_drop<F>(
    counter: &libllm_protocol::tokenizer::TokenCounter,
    budget: usize,
    max_drop: usize,
    render: &F,
) -> anyhow::Result<usize>
where
    F: Fn(usize) -> String,
{
    let full_count = counter.count_authoritative(&render(0)).await?;
    if full_count <= budget {
        return Ok(0);
    }

    let (mut lo, mut hi) = (1usize, max_drop);
    let mut best = max_drop;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let count = counter.count_authoritative(&render(mid)).await?;
        if count <= budget {
            best = mid;
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    Ok(best)
}

enum StreamPreflight {
    Proceed,
    Queued,
    Blocked,
}

fn stream_preflight(app: &mut App<'_>, content: &str) -> StreamPreflight {
    if app.summarize.receiver.is_some() {
        app.summarize.in_progress = true;
        app.streaming.message_queue.push(content.to_owned());
        tracing::debug!(
            phase = "queued_for_summary",
            queue_len = app.streaming.message_queue.len(),
            "stream.start"
        );
        return StreamPreflight::Queued;
    }
    if app.model_name.is_none() {
        tracing::debug!(phase = "blocked", reason = "model_pending", "stream.start");
        app.set_status(
            "Connecting to API server...".to_owned(),
            StatusLevel::Warning,
        );
        return StreamPreflight::Blocked;
    }
    if !app.api_available {
        tracing::debug!(
            phase = "blocked",
            reason = "api_unavailable",
            "stream.start"
        );
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            StatusLevel::Error,
        );
        return StreamPreflight::Blocked;
    }
    StreamPreflight::Proceed
}

fn push_user_segments(app: &mut App<'_>, content: &str) {
    for c in app.session.characters.iter_mut() {
        c.spoke_this_round = false;
    }
    app.session.characters =
        libllm_core::group_chat::renormalize_action_values(&app.session.characters);
    let mut parent = app.session.tree.head();
    let segments: Vec<String> = if app.session.character.is_some() {
        libllm_core::side_character::split_user_input(content)
    } else {
        vec![content.to_owned()]
    };
    for segment in segments {
        let new_id = app
            .session
            .tree
            .push(parent, Message::new(Role::User, segment));
        parent = Some(new_id);
    }
}

async fn launch_stream(app: &mut App<'_>, sender: mpsc::Sender<StreamToken>) {
    app.mark_session_dirty(SaveTrigger::Debounced, false);
    app.invalidate_chat_caches();
    app.streaming.active = true;
    app.streaming.started_at = None;
    app.streaming.first_think_closed_at = None;
    app.focus = crate::Focus::Input;
    app.nav_cursor = None;
    app.hover_node = None;
    app.streaming.buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let budget = app.context_mgr.token_limit();
    let branch_path = app.session.tree.branch_path();
    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
    let max_drop = libllm_core::context::droppable_count(&summary_aware).saturating_sub(1);

    let render = |k: usize| -> String { build_rendered_prompt(app, k).0 };

    let dropped = match find_smallest_drop(&app.token_counter, budget, max_drop, &render).await {
        Ok(k) => k,
        Err(err) => {
            tracing::warn!(
                result = "fallback_heuristic",
                error = %err,
                "stream.truncate"
            );
            0
        }
    };
    let effective_prompt = business::build_effective_system_prompt(app.session, app.db.as_ref());
    let prompt = build_rendered_prompt(app, dropped).0;
    let stop_tokens = app.stop_tokens.clone();
    let sampling = app.sampling.clone();

    tracing::info!(
        phase = "dispatch",
        branch_len = branch_path.len(),
        summary_aware_len = summary_aware.len(),
        dropped = dropped,
        worldbook_count = worldbooks.len(),
        has_system_prompt = effective_prompt.is_some(),
        stop_token_count = stop_tokens.len(),
        prompt_bytes = prompt.len(),
        continuation = false,
        "stream.start"
    );

    let client = app.client.clone();
    let handle = tokio::spawn(async move {
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        client
            .stream_completion_to_channel(&prompt, &stop_refs, &sampling, sender)
            .await;
    });
    app.streaming.task = Some(handle);
}

/// Prepares streaming state for a group-chat assistant turn, then spawns the
/// completion request. The message node for this turn must already be appended
/// to the tree and set as head (with prefill content, speaker, and
/// pre_turn_action_points populated) before calling this. On `Done`, the token
/// handler appends the streamed completion to the head node's existing content
/// via the continuation path, preserving the prefill and speaker fields.
pub(crate) async fn stream_into_message(
    app: &mut App<'_>,
    system: String,
    mut stop_sequences: Vec<String>,
    nudge: Option<String>,
    sender: mpsc::Sender<StreamToken>,
) {
    for stop in &app.stop_tokens {
        if !stop_sequences.iter().any(|existing| existing == stop) {
            stop_sequences.push(stop.clone());
        }
    }
    app.mark_session_dirty(SaveTrigger::Debounced, false);
    app.invalidate_chat_caches();
    app.streaming.active = true;
    app.streaming.is_continuation = true;
    app.streaming.started_at = None;
    app.streaming.first_think_closed_at = None;
    app.focus = crate::Focus::Input;
    app.nav_cursor = None;
    app.hover_node = None;
    app.streaming.buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let budget = app.context_mgr.token_limit();
    let branch_path = app.session.tree.branch_path();
    let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
    let max_drop = libllm_core::context::droppable_count(&summary_aware).saturating_sub(1);

    let render = |k: usize| -> String {
        build_rendered_prompt_with_system(app, k, &system, nudge.as_deref()).0
    };

    let dropped = match find_smallest_drop(&app.token_counter, budget, max_drop, &render).await {
        Ok(k) => k,
        Err(err) => {
            tracing::warn!(
                result = "fallback_heuristic",
                error = %err,
                "stream.truncate"
            );
            0
        }
    };

    let prompt = build_rendered_prompt_with_system(app, dropped, &system, nudge.as_deref()).0;
    let sampling = app.sampling.clone();

    tracing::info!(
        phase = "dispatch",
        branch_len = branch_path.len(),
        summary_aware_len = summary_aware.len(),
        dropped = dropped,
        worldbook_count = worldbooks.len(),
        stop_token_count = stop_sequences.len(),
        prompt_bytes = prompt.len(),
        continuation = true,
        group_turn = true,
        "stream.start"
    );

    let client = app.client.clone();
    let handle = tokio::spawn(async move {
        let stop_refs: Vec<&str> = stop_sequences.iter().map(String::as_str).collect();
        client
            .stream_completion_to_channel(&prompt, &stop_refs, &sampling, sender)
            .await;
    });
    app.streaming.task = Some(handle);
}

pub(super) async fn run_one_group_turn(
    app: &mut App<'_>,
    speaker_slug: &str,
    snapshot_json: &str,
    sender: &mpsc::Sender<StreamToken>,
) {
    let tpl_name = app
        .config
        .template_preset
        .as_deref()
        .unwrap_or("Default")
        .to_owned();
    let template = libllm_core::preset::resolve_template_preset(
        &tpl_name,
        &libllm_config::template_presets_dir(),
    );

    let persona = app
        .session
        .persona
        .as_ref()
        .and_then(|slug| app.db.as_ref().and_then(|db| db.load_persona(slug).ok()));

    let base_system_prompt = app.session.system_prompt.clone().or_else(|| {
        app.db
            .as_ref()
            .and_then(|db| {
                db.load_prompt(libllm_core::system_prompt::BUILTIN_ROLEPLAY)
                    .ok()
            })
            .map(|p| p.content)
            .filter(|s| !s.is_empty())
    });
    let nudge_template = app.config.group_chat.nudge_prompt.clone();

    let prompt = match libllm_core::group_chat::build_turn_prompt(
        libllm_core::group_chat::TurnPromptInputs {
            session: app.session,
            cards: &app.character.cards_cache,
            persona: persona.as_ref(),
            template: Some(&template),
            speaker_slug,
            base_system_prompt: base_system_prompt.as_deref(),
            nudge_template: Some(nudge_template.as_str()),
        },
    ) {
        Ok(p) => p,
        Err(e) => {
            app.set_status(
                format!("group prompt build failed: {e}"),
                StatusLevel::Error,
            );
            return;
        }
    };

    let _span = tracing::info_span!(
        "group_turn",
        speaker = %speaker_slug,
        mode = ?app.session.chat_mode
    )
    .entered();

    let mut assistant_msg = Message::new(Role::Assistant, prompt.prefill.clone());
    assistant_msg.speaker = Some(speaker_slug.to_owned());
    assistant_msg.pre_turn_action_points = Some(snapshot_json.to_owned());

    let parent_id = app.session.tree.head();
    app.session.tree.push(parent_id, assistant_msg);

    app.streaming.prefill = Some(prompt.prefill.clone());

    stream_into_message(
        app,
        prompt.system,
        prompt.stop_sequences,
        prompt.nudge,
        sender.clone(),
    )
    .await;
}

/// Initializes group-chat loop state and starts the first turn. Subsequent turns are
/// triggered by `handle_stream_token` after each `Done` event.
pub(crate) async fn start_group_chat_loop(app: &mut App<'_>, sender: &mpsc::Sender<StreamToken>) {
    app.group_chat.loop_rng = Some(rand::make_rng());
    app.group_chat.consecutive = 0;
    app.group_chat.max_consecutive = app.config.group_chat.effective_max_consecutive_turns();
    app.group_chat.remaining_budget = libllm_core::group_chat::DEFAULT_TURN_TIME_BUDGET;
    continue_group_chat_loop(app, sender).await;
}

/// Runs the next turn of the active group-chat loop, or ends the loop if no speaker is
/// eligible or the consecutive-turn cap has been reached.
///
/// Dispatch per mode:
/// - `Directed`: no auto-speak; loop exits immediately.
/// - `RoundRobin` / `WeightedRandom`: pick once with no budget, run one turn, then exit.
/// - `ActionValue`: cascade with forced first turn (no budget) then budget-gated turns.
pub(crate) async fn continue_group_chat_loop(
    app: &mut App<'_>,
    sender: &mpsc::Sender<StreamToken>,
) {
    if app.group_chat.loop_rng.is_none() {
        return;
    }

    if app.group_chat.consecutive >= app.group_chat.max_consecutive {
        tracing::warn!(
            consecutive = app.group_chat.consecutive,
            "group_chat: consecutive-turn cap fired, yielding to user"
        );
        app.group_chat.loop_rng = None;
        return;
    }

    match app.session.chat_mode {
        ChatMode::Directed => {
            app.group_chat.loop_rng = None;
        }
        ChatMode::RoundRobin | ChatMode::WeightedRandom => {
            if app.group_chat.consecutive > 0 {
                app.group_chat.loop_rng = None;
                return;
            }
            let mode = app.session.chat_mode;
            let decision = {
                let rng = app.group_chat.loop_rng.as_mut().expect("group_chat.loop_rng is Some, guarded by the is_none() early return at the top of this function");
                libllm_core::group_chat::decide_next_speaker(
                    &app.session.characters,
                    mode,
                    rng,
                    None,
                )
            };
            let Some(decision) = decision else {
                tracing::debug!(
                    mode = app.session.chat_mode.as_str(),
                    "group_chat: no eligible speaker, yielding to user"
                );
                app.group_chat.loop_rng = None;
                return;
            };
            apply_decision(app, &decision);
            let snapshot_json =
                serde_json::to_string(&decision.snapshot_before).unwrap_or_default();
            let speaker_slug = decision.speaker_slug.clone();
            app.group_chat.consecutive += 1;
            app.group_chat.loop_rng = None;
            run_one_group_turn(app, &speaker_slug, &snapshot_json, sender).await;
        }
        ChatMode::ActionValue => {
            let time_budget = if app.group_chat.consecutive == 0 {
                None
            } else {
                Some(app.group_chat.remaining_budget)
            };
            let mode = app.session.chat_mode;
            let decision = {
                let rng = app.group_chat.loop_rng.as_mut().expect("group_chat.loop_rng is Some, guarded by the is_none() early return at the top of this function");
                libllm_core::group_chat::decide_next_speaker(
                    &app.session.characters,
                    mode,
                    rng,
                    time_budget,
                )
            };
            let Some(decision) = decision else {
                tracing::debug!(
                    "group_chat: cascade complete (no eligible speakers), yielding to user"
                );
                app.group_chat.loop_rng = None;
                return;
            };
            app.group_chat.remaining_budget -= decision.time_advanced.max(0.0);
            apply_decision(app, &decision);
            let snapshot_json =
                serde_json::to_string(&decision.snapshot_before).unwrap_or_default();
            let speaker_slug = decision.speaker_slug.clone();
            app.group_chat.consecutive += 1;
            run_one_group_turn(app, &speaker_slug, &snapshot_json, sender).await;
        }
    }
}

fn apply_decision(app: &mut App<'_>, decision: &libllm_core::group_chat::TurnDecision) {
    for (slug, av) in &decision.updated_action_points {
        if let Some(c) = app.session.characters.iter_mut().find(|c| &c.slug == slug) {
            c.action_points = *av;
        }
    }
    if let Some(c) = app
        .session
        .characters
        .iter_mut()
        .find(|c| c.slug == decision.speaker_slug)
    {
        c.spoke_this_round = true;
    }
}

pub(crate) async fn start_streaming(
    app: &mut App<'_>,
    content: &str,
    sender: mpsc::Sender<StreamToken>,
) {
    match stream_preflight(app, content) {
        StreamPreflight::Proceed => {}
        StreamPreflight::Queued => {
            app.clear_input_textarea_if_holds(content);
            return;
        }
        StreamPreflight::Blocked => return,
    }
    debug_assert!(
        !content.trim().is_empty(),
        "start_streaming called with blank content"
    );
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let resolved = match libllm_core::files::resolve_all_resolved(content, &cwd, &app.config.files)
    {
        Ok(v) => v,
        Err(libllm_core::files::FileError::Collision { path, kind }) => {
            crate::dialogs::injection_warning::open(app, &path, kind);
            return;
        }
        Err(err) => {
            app.set_status(err.to_string(), crate::types::StatusLevel::Error);
            return;
        }
    };

    if app.config.summarization.enabled && app.file_summary.summarizer.is_some() {
        let context_size = app.context_mgr.token_limit();
        for file in &resolved {
            if let Err(err) = libllm_protocol::summarize::check_file_fits(
                &app.token_counter,
                file,
                &app.config.files.summary_prompt,
                context_size,
            )
            .await
            {
                app.set_status(err.to_string(), crate::types::StatusLevel::Error);
                return;
            }
        }
    }

    let sys_messages =
        match libllm_core::files::assemble_snapshot_messages(resolved, &app.config.files) {
            Ok(v) => v,
            Err(err) => {
                app.set_status(err.to_string(), crate::types::StatusLevel::Error);
                return;
            }
        };
    app.clear_input_textarea_if_holds(content);
    match (
        app.config.files.summarize_mode == libllm_core::config::FileSummarizeMode::Eager,
        app.config.summarization.enabled,
        app.save_mode.id(),
        app.file_summary.summarizer.as_ref(),
    ) {
        (false, _, _, _) => {
            tracing::debug!(reason = "mode_lazy", "files.summary.eager_schedule.skipped")
        }
        (_, false, _, _) => tracing::debug!(
            reason = "summarization_disabled",
            "files.summary.eager_schedule.skipped"
        ),
        (_, _, None, _) => tracing::debug!(
            reason = "no_session_id",
            "files.summary.eager_schedule.skipped"
        ),
        (_, _, _, None) => tracing::debug!(
            reason = "no_summarizer",
            "files.summary.eager_schedule.skipped"
        ),
        (true, true, Some(session_id), Some(summarizer)) => {
            let to_summarize = libllm_core::files::files_to_summarize_from_messages(&sys_messages);
            tracing::info!(
                session_id = %session_id,
                file_count = to_summarize.len(),
                "files.summary.eager_schedule.dispatching"
            );
            for file in &to_summarize {
                summarizer.schedule(session_id, file);
            }
        }
    }

    let mut parent = app.session.tree.head();
    for sys_msg in sys_messages {
        let new_id = app.session.tree.push(parent, sys_msg);
        parent = Some(new_id);
    }
    push_user_segments(app, content);

    if app.session.characters.len() >= 2 {
        start_group_chat_loop(app, &sender).await;
    } else {
        launch_stream(app, sender).await;
    }
}

/// Push a new user message at the current head and stream. Unlike
/// `start_streaming`, this does not resolve `@file` references, so file
/// snapshots already present in the branch are shared with the new sibling
/// rather than duplicated.
pub(crate) async fn start_retry_streaming(
    app: &mut App<'_>,
    content: &str,
    sender: mpsc::Sender<StreamToken>,
) {
    match stream_preflight(app, content) {
        StreamPreflight::Proceed => {}
        StreamPreflight::Queued => {
            app.clear_input_textarea_if_holds(content);
            return;
        }
        StreamPreflight::Blocked => return,
    }
    debug_assert!(
        !content.trim().is_empty(),
        "start_retry_streaming called with blank content"
    );
    app.clear_input_textarea_if_holds(content);
    push_user_segments(app, content);
    if app.session.characters.len() >= 2 {
        start_group_chat_loop(app, &sender).await;
    } else {
        launch_stream(app, sender).await;
    }
}

pub(crate) async fn handle_stream_token(
    token: StreamToken,
    app: &mut App<'_>,
    sender: mpsc::Sender<StreamToken>,
) -> Result<()> {
    if !app.streaming.active {
        return Ok(());
    }
    match token {
        StreamToken::Token(text) => {
            if app.streaming.started_at.is_none() {
                app.streaming.started_at = Some(std::time::Instant::now());
            }
            app.streaming.buffer.push_str(&text);
            if app.streaming.first_think_closed_at.is_none()
                && !app.streaming.is_continuation
                && let Some(preset) = app.reasoning_preset.as_ref()
            {
                let close = preset.suffix.trim();
                if !close.is_empty() && app.streaming.buffer.contains(close) {
                    app.streaming.first_think_closed_at = Some(std::time::Instant::now());
                }
            }
            app.auto_scroll = true;
        }
        StreamToken::Done(full_response) => {
            let head = app.session.tree.head().expect("tree has a head node because a user message was pushed before the stream was started");
            let response_bytes = full_response.len();
            let is_continuation = app.streaming.is_continuation;
            let measured_seconds = libllm_core::thought::measured_thought_seconds(
                app.streaming.started_at,
                app.streaming.first_think_closed_at,
            );
            if app.streaming.is_continuation {
                let combined = match app.streaming.prefill.take() {
                    Some(prefill) => format!("{}{}", prefill, full_response.trim_start()),
                    None => {
                        let existing = app.session.tree.node(head).expect("head id was obtained from tree.head() and is a valid allocated node").message.content.clone();
                        format!("{}{}", existing, full_response)
                    }
                };
                let combined = libllm_core::regex_rules::apply(
                    &app.compiled_regex,
                    libllm_core::regex_rules::Scope::PromptRecv,
                    Role::Assistant,
                    &combined,
                )
                .into_owned();
                app.session.tree.set_message_content(head, combined);
                let current_seconds = app
                    .session
                    .tree
                    .node(head)
                    .and_then(|node| node.message.thought_seconds);
                let final_seconds = libllm_core::thought::resolve_thought_seconds(
                    &app.session
                        .tree
                        .node(head)
                        .expect(
                            "head id was obtained from tree.head() and is a valid allocated node",
                        )
                        .message
                        .content,
                    current_seconds,
                    measured_seconds,
                    app.reasoning_preset.as_ref(),
                );
                app.session
                    .tree
                    .set_message_thought_seconds(head, final_seconds);
                app.streaming.is_continuation = false;
            } else {
                let stored_content = libllm_core::thought::normalize_assistant_content(
                    &full_response,
                    app.reasoning_preset.as_ref(),
                )
                .into_owned();
                let stored_content = libllm_core::regex_rules::apply(
                    &app.compiled_regex,
                    libllm_core::regex_rules::Scope::PromptRecv,
                    Role::Assistant,
                    &stored_content,
                )
                .into_owned();
                let final_seconds = libllm_core::thought::resolve_thought_seconds(
                    &stored_content,
                    None,
                    measured_seconds,
                    app.reasoning_preset.as_ref(),
                );
                app.session.tree.push(
                    Some(head),
                    Message::new(Role::Assistant, stored_content)
                        .with_thought_seconds(final_seconds),
                );
            }
            tracing::info!(
                result = "ok",
                bytes = response_bytes,
                is_continuation,
                node_id = head,
                "stream.done"
            );
            app.mark_session_dirty(SaveTrigger::StreamDone, true);
            app.invalidate_chat_caches();
            app.streaming.buffer.clear();
            app.streaming.active = false;
            app.streaming.started_at = None;
            app.streaming.first_think_closed_at = None;
            app.auto_scroll = true;
            app.flush_session_save(SaveTrigger::StreamDone)?;
            business::refresh_sidebar(app);
            if app.summarize.enabled && app.summarize.receiver.is_none() {
                let context_size = app.context_mgr.token_limit();
                let trigger_percent = app.config.summarization.effective_trigger_percent();
                let threshold_tokens = context_size * trigger_percent as usize / 100;
                let branch_path = app.session.tree.branch_path();
                let summary_aware = app.context_mgr.summary_aware_path(&branch_path);
                let max_drop =
                    libllm_core::context::droppable_count(&summary_aware).saturating_sub(1);
                let (full_prompt, _) = build_rendered_prompt(app, 0);
                let actual_tokens = match app.token_counter.count_authoritative(&full_prompt).await
                {
                    Ok(n) => n,
                    Err(err) => {
                        tracing::warn!(
                            result = "fallback_skip",
                            error = %err,
                            "stream.summary.trigger"
                        );
                        0
                    }
                };

                if actual_tokens < threshold_tokens {
                    tracing::debug!(
                        result = "not_fired",
                        context_size,
                        trigger_percent,
                        actual_tokens,
                        threshold_tokens,
                        "stream.summary.trigger"
                    );
                } else {
                    let keep_last = app.config.summarization.keep_last;
                    let droppable = libllm_core::context::droppable_count(&summary_aware);
                    let dropped = droppable.saturating_sub(keep_last).min(max_drop);
                    let summary_boundary = branch_path.len() - summary_aware.len();
                    let split_idx = libllm_core::context::drop_split_index(&summary_aware, dropped);
                    let messages_to_summarize: Vec<Message> = branch_path
                        [summary_boundary..summary_boundary + split_idx]
                        .iter()
                        .map(|m| (*m).clone())
                        .collect();

                    if !messages_to_summarize.is_empty() {
                        tracing::info!(
                            result = "scheduled",
                            dropped,
                            trigger_percent,
                            context_size,
                            actual_tokens,
                            threshold_tokens,
                            summary_boundary,
                            messages_to_summarize = messages_to_summarize.len(),
                            "stream.summary.schedule"
                        );
                        let session_id_for_summarizer = app.save_mode.id().map(str::to_owned);
                        let files_to_wait_on = libllm_core::files::files_to_summarize_from_messages(
                            &messages_to_summarize,
                        );

                        if !files_to_wait_on.is_empty() {
                            if let (Some(session_id), Some(summarizer_svc)) = (
                                session_id_for_summarizer.as_deref(),
                                app.file_summary.summarizer.as_ref(),
                            ) {
                                tracing::info!(
                                    session_id = %session_id,
                                    file_count = files_to_wait_on.len(),
                                    "files.summary.ensure_ready.dispatch"
                                );
                                if let Err(err) = summarizer_svc
                                    .ensure_ready(session_id, &files_to_wait_on)
                                    .await
                                {
                                    tracing::warn!(
                                        result = "error",
                                        error = %err,
                                        "files.summary.ensure_ready_before_auto_summarize"
                                    );
                                }
                            } else {
                                tracing::debug!(
                                    file_count = files_to_wait_on.len(),
                                    session_present = app.save_mode.id().is_some(),
                                    summarizer_present = app.file_summary.summarizer.is_some(),
                                    "files.summary.ensure_ready.skipped"
                                );
                            }
                        }

                        let summaries_snapshot: HashMap<String, libllm_core::files::FileSummary> =
                            if let (Some(session_id), Some(summarizer_svc)) = (
                                session_id_for_summarizer.as_deref(),
                                app.file_summary.summarizer.as_ref(),
                            ) {
                                let snapshot: HashMap<String, libllm_core::files::FileSummary> =
                                    files_to_wait_on
                                        .iter()
                                        .filter_map(|f| {
                                            summarizer_svc
                                                .lookup(session_id, &f.content_hash)
                                                .map(|s| (f.content_hash.clone(), s))
                                        })
                                        .collect();
                                tracing::debug!(
                                    session_id = %session_id,
                                    snapshot_size = snapshot.len(),
                                    wanted = files_to_wait_on.len(),
                                    "files.summary.snapshot.built"
                                );
                                snapshot
                            } else {
                                tracing::debug!(
                                    session_id = session_id_for_summarizer.as_deref().unwrap_or(""),
                                    snapshot_size = 0usize,
                                    wanted = files_to_wait_on.len(),
                                    "files.summary.snapshot.built"
                                );
                                HashMap::new()
                            };

                        let summarize_api_url =
                            crate::business::summarize_api_url(&app.config, &app.cli_overrides);
                        let summarizer_auth = libllm_core::config::resolve_auth(
                            &app.config,
                            &app.cli_overrides.auth_overrides(),
                        );
                        let summarizer_client = libllm_protocol::client::ApiClient::new(
                            &summarize_api_url,
                            app.config.tls_skip_verify || app.cli_overrides.tls_skip_verify,
                            summarizer_auth,
                        );
                        let summarizer = libllm_protocol::summarize::Summarizer::new(
                            summarizer_client,
                            app.config.summarization.prompt.clone(),
                        );
                        let token_budget = app.context_mgr.token_limit();
                        let current_head = app.session.tree.head();
                        let scenario = app.session.scenario.clone();

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        app.summarize.receiver = Some(rx);
                        app.summarize.branch_head = current_head;
                        app.summarize.pending_dropped = Some(dropped);
                        app.summarize.in_progress = true;

                        let summary_counter = app.token_counter.clone();
                        tokio::spawn(async move {
                            let refs: Vec<&Message> = messages_to_summarize.iter().collect();
                            let lookup = SnapshotFileSummaryLookup(summaries_snapshot);
                            let result = summarizer
                                .summarize(
                                    scenario.as_deref(),
                                    &refs,
                                    token_budget,
                                    &summary_counter,
                                    &lookup,
                                )
                                .await;
                            let _ = tx.send(result.map_err(|e| e.to_string()));
                        });
                    }
                }
            }
            if app.group_chat.loop_rng.is_some() && !app.summarize.in_progress {
                Box::pin(continue_group_chat_loop(app, &sender)).await;
            } else if !app.streaming.message_queue.is_empty() {
                let next = app.streaming.message_queue.remove(0);
                Box::pin(start_streaming(app, &next, sender)).await;
                if !app.streaming.active {
                    app.streaming.message_queue.clear();
                }
            }
        }
        StreamToken::Error(err) => {
            tracing::error!(result = "error", is_continuation = app.streaming.is_continuation, error = %err, "stream.done");
            app.streaming.buffer.clear();
            app.streaming.active = false;
            app.streaming.is_continuation = false;
            app.streaming.prefill = None;
            app.streaming.started_at = None;
            app.streaming.first_think_closed_at = None;
            app.streaming.message_queue.clear();
            app.group_chat.loop_rng = None;
            app.set_status(format!("Error: {err}"), StatusLevel::Error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn find_smallest_drop_binary_search() {
        use libllm_protocol::tokenizer::{HeuristicTokenizer, TokenCounter, TokenizerBackend};

        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );

        let render_at = |k: usize| -> String {
            let chars = 400usize.saturating_sub(40 * k);
            "a".repeat(chars)
        };

        let k = find_smallest_drop(&counter, 60, 8, &render_at)
            .await
            .unwrap();
        assert_eq!(k, 6);
    }

    #[tokio::test]
    async fn find_smallest_drop_zero_when_fits() {
        use libllm_protocol::tokenizer::{HeuristicTokenizer, TokenCounter, TokenizerBackend};
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );
        let render_at = |_k: usize| -> String { "x".repeat(40) };
        let k = find_smallest_drop(&counter, 100, 8, &render_at)
            .await
            .unwrap();
        assert_eq!(k, 0);
    }

    #[test]
    fn threshold_tokens_math_matches_spec() {
        let context_size = 131_072usize;
        let trigger_percent = 90u8;
        let threshold = context_size * trigger_percent as usize / 100;
        assert_eq!(threshold, 117_964);

        let context_size = 4096usize;
        let trigger_percent = 50u8;
        let threshold = context_size * trigger_percent as usize / 100;
        assert_eq!(threshold, 2048);

        let context_size = 1000usize;
        let trigger_percent = 100u8;
        let threshold = context_size * trigger_percent as usize / 100;
        assert_eq!(threshold, 1000);
    }

    #[test]
    fn rewrite_user_message_pass_substitutes_at_tokens_on_user_role_only() {
        use libllm_core::session::{Message, Role};

        let original = [
            Message::new(Role::User, "@./notes.md please".to_owned()),
            Message::new(Role::Assistant, "first response".to_owned()),
            Message::new(Role::User, "also @./code.rs".to_owned()),
            Message::new(Role::System, "system literal @./leave.md alone".to_owned()),
        ];
        let rewritten: Vec<Message> = original
            .iter()
            .map(|m| match m.role {
                Role::User => Message {
                    role: m.role,
                    content: libllm_core::files::rewrite_user_message(&m.content),
                    timestamp: m.timestamp.clone(),
                    thought_seconds: m.thought_seconds,
                    speaker: m.speaker.clone(),
                    pre_turn_action_points: m.pre_turn_action_points.clone(),
                },
                _ => m.clone(),
            })
            .collect();
        assert_eq!(rewritten[0].content, "[notes.md] please");
        assert_eq!(rewritten[1].content, "first response");
        assert_eq!(rewritten[2].content, "also [code.rs]");
        assert_eq!(rewritten[3].content, "system literal @./leave.md alone");
    }
}
